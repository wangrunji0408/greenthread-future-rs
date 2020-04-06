//! Convert closures to futures based on greenthread on bare-metal (no_std + no_alloc).

#![cfg_attr(not(test), no_std)]
#![feature(asm)]
#![feature(naked_functions)]
#![feature(untagged_unions)]
#![deny(warnings)]

use core::future::Future;
use core::mem::ManuallyDrop;
use core::pin::Pin;
use core::task::{Context, Poll};

/// Future that wraps a blocking thread.
#[repr(C, align(0x2000))]
pub union ThreadFuture<F, T> {
    tcb: ManuallyDrop<TCB<F, T>>,
    stack: [usize; RAW_SIZE / 8],
}

/// Thread Control Block (TCB)
///
/// This struct is allocated on heap whose start address is aligned to 0x2000.
/// So that we can quickly locate it from stack pointer (just like Linux).
#[repr(C)]
struct TCB<F, T> {
    /// Pointer to the context of executor or thread.
    ///
    /// Running thread call `switch` on this to switch back to executor.
    context_ptr: *mut ThreadContext,

    /// A canary value to detect stack overflow.
    canary: usize,

    /// Thread state. Contains function object or return value.
    state: State<F, T>,
}

unsafe impl<F, T> Send for TCB<F, T> {}

const RAW_SIZE: usize = 0x2000;

const CANARY: usize = 0xcafebabe_deadbeaf;

impl<F, T> TCB<F, T> {
    /// Get a mutable reference of current TCB.
    unsafe fn current() -> &'static mut Self {
        let mut rsp: usize;
        asm!("" : "={rsp}"(rsp));
        rsp &= !(RAW_SIZE - 1);
        &mut *(rsp as *mut _)
    }
}

/// Thread state
enum State<F, T> {
    Ready(F),
    Running,
    Exited(T),
    Invalid,
}

impl<F, T> State<F, T> {
    /// Takes the return value out of the state if it's `Exited`.
    fn take_ret(&mut self) -> Option<T> {
        if let State::Exited(_) = self {
            if let State::Exited(ret) = core::mem::replace(self, State::Invalid) {
                Some(ret)
            } else {
                unreachable!()
            }
        } else {
            None
        }
    }
}

/// Saved registers of a thread.
#[repr(C)]
#[derive(Default, Debug)]
struct ThreadContext {
    rbx: usize,
    rbp: usize,
    r12: usize,
    r13: usize,
    r14: usize,
    r15: usize,
    rip: usize,
}

impl ThreadContext {
    /// Switch context to another thread.
    #[naked]
    #[inline(never)]
    unsafe extern "sysv64" fn switch(_ptr_ptr: *mut *mut Self) {
        asm!(r#"
        // push rip (by caller)
        push r15
        push r14
        push r13
        push r12
        push rbp
        push rbx

        mov rax, [rdi]
        mov [rdi], rsp
        mov rsp, rax

        pop rbx
        pop rbp
        pop r12
        pop r13
        pop r14
        pop r15
        // pop rip (by ret)
        "# :::: "volatile" "intel" "alignstack");
    }
}

impl<F, T> From<F> for ThreadFuture<F, T>
where
    F: Send + 'static + Unpin + FnOnce() -> T,
    T: Send + 'static + Unpin,
{
    /// Convert a closure of blocking thread to future.
    ///
    /// # Example
    /// TODO
    fn from(f: F) -> Self {
        assert_eq!(core::mem::size_of::<Self>(), RAW_SIZE, "TCB size exceed");
        ThreadFuture {
            tcb: ManuallyDrop::new(TCB {
                context_ptr: core::ptr::null_mut(),
                state: State::Ready(f),
                canary: CANARY,
            }),
        }
    }
}

impl<F, T> Future for ThreadFuture<F, T>
where
    F: Send + 'static + Unpin + FnOnce() -> T,
    T: Send + 'static + Unpin,
{
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // allocate executor context at stack
        let raw = self.get_mut();
        let state = unsafe {
            // fill SP and PC at first run
            if let State::Ready(_) = &raw.tcb.state {
                let context = ((raw as *mut Self).add(1) as *mut ThreadContext).sub(1);
                (*context).rip = entry::<F, T> as usize;
                raw.tcb.context_ptr = context;
            }
            // switch to the thread
            ThreadContext::switch(&mut raw.tcb.context_ptr);
            &mut raw.tcb.state
        };
        // check the thread state
        if let Some(ret) = state.take_ret() {
            // exited
            Poll::Ready(ret)
        } else {
            // yield_now
            // wake up myself, otherwise the executor won't poll me again
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// A static function as the entry of new thread
unsafe extern "sysv64" fn entry<F, T>()
where
    F: Send + 'static + FnOnce() -> T,
    T: Send + 'static,
{
    let tcb = TCB::<F, T>::current();
    if let State::Ready(f) = core::mem::replace(&mut tcb.state, State::Running) {
        let ret = f();
        tcb.state = State::Exited(ret);
    } else {
        unreachable!()
    }
    yield_now();
    unreachable!();
}

/// Cooperatively gives up the CPU to the executor.
///
/// # Example
/// TODO
pub fn yield_now() {
    unsafe {
        // type `F` and `T` do not matter
        let tcb = TCB::<fn(), ()>::current();
        // ensure we got a valid structure
        assert_eq!(
            tcb.canary, CANARY,
            "canary is changed. maybe stack overflow!"
        );
        // switch back to the executor thread
        ThreadContext::switch(&mut tcb.context_ptr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test() {
        let h1 = tokio::spawn(ThreadFuture::from(|| {
            println!("1.1");
            yield_now();
            println!("1.2");
            1u32
        }));
        let h2 = tokio::spawn(ThreadFuture::from(|| {
            println!("2.1");
            yield_now();
            println!("2.2");
            2u32
        }));
        println!("join 1 => {}", h1.await.unwrap());
        println!("join 2 => {}", h2.await.unwrap());
    }
}
