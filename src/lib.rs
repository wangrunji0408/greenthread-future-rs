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
    /// Pointer to the executor context.
    ///
    /// This value is set by the executor before running a thread.
    /// Running thread call `switch` on this to switch back to executor.
    executor_context_ptr: usize,

    /// The register context of thread.
    context: ThreadContext,

    /// A canary value to detect stack overflow.
    canary: usize,

    /// Thread state. Contains function object or return value.
    state: State<F, T>,
}

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
    rsp: usize,
    rbx: usize,
    rbp: usize,
    r12: usize,
    r13: usize,
    r14: usize,
    r15: usize,
}

impl ThreadContext {
    /// Switch context to another thread.
    #[naked]
    #[inline(never)]
    unsafe extern "sysv64" fn switch(&mut self, _target: &mut Self) {
        asm!(r#"
        mov [rdi], rsp
        mov [rdi + 8*1], rbx
        mov [rdi + 8*2], rbp
        mov [rdi + 8*3], r12
        mov [rdi + 8*4], r13
        mov [rdi + 8*5], r14
        mov [rdi + 8*6], r15

        mov rbx, [rsi + 8*1]
        mov rbp, [rsi + 8*2]
        mov r12, [rsi + 8*3]
        mov r13, [rsi + 8*4]
        mov r14, [rsi + 8*5]
        mov r15, [rsi + 8*6]
        mov rsp, [rsi]
        "# :::: "volatile" "intel");
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
                executor_context_ptr: 0,
                context: Default::default(),
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
        let mut context = ThreadContext::default();
        let raw = self.get_mut();
        let state = unsafe {
            // fill 'rsp' at first run
            if let State::Ready(_) = &raw.tcb.state {
                let rsp = &mut raw.stack[RAW_SIZE / 8 - 1];
                *rsp = entry::<F, T> as usize;
                raw.tcb.context.rsp = rsp as *mut _ as usize;
            }
            // fill pointer to my context so that the thread can switch back
            raw.tcb.executor_context_ptr = &mut context as *mut _ as usize;
            // switch to the thread
            context.switch(&mut raw.tcb.context);
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
        let executor_context = &mut *(tcb.executor_context_ptr as *mut _);
        tcb.context.switch(executor_context);
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
