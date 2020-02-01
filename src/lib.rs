//! Convert blocking functions to thread future on bare-metal (no_std).

#![cfg_attr(not(test), no_std)]
#![feature(asm)]
#![feature(naked_functions)]
#![deny(warnings)]

extern crate alloc;

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

/// Future that wraps a blocking thread.
pub struct ThreadFuture<F, T>
where
    F: Send + 'static + FnOnce() -> T,
    T: Send + 'static,
{
    inner: Box<TCB<F, T>>,
}

/// Thread Control Block (TCB)
///
/// This struct is allocated on heap whose start address is aligned to 0x2000.
/// So that we can quickly locate it from stack pointer (just like Linux).
#[repr(C)]
#[repr(align(0x4000))]
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

    /// Stack of the thread.
    stack: [u8; 0x3000],
}

// TODO: dynamic set stack size
const TCB_SIZE: usize = 0x4000;

const CANARY: usize = 0xcafebabe_deadbeaf;

impl<F, T> TCB<F, T> {
    /// Get a mutable reference of current raw struct.
    unsafe fn current() -> &'static mut Self {
        let mut rsp: usize;
        asm!("" : "={rsp}"(rsp));
        rsp &= !(TCB_SIZE - 1);
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

impl<F, T> ThreadFuture<F, T>
where
    F: Send + 'static + FnOnce() -> T,
    T: Send + 'static,
{
    /// Convert a closure of blocking thread to future.
    ///
    /// # Example
    /// TODO
    pub fn new(f: F) -> Self {
        assert!(
            core::mem::size_of::<TCB<F, T>>() <= TCB_SIZE,
            "TCB size exceed"
        );
        let mut inner = Box::new(TCB {
            executor_context_ptr: 0,
            context: Default::default(),
            state: State::Ready(f),
            canary: CANARY,
            stack: [0; 0x3000],
        });
        unsafe {
            let rsp = (((&*inner as *const TCB<F, T>).add(1) as usize) & !0xf) - 8;
            (rsp as *mut usize).write(entry::<F, T> as usize);
            inner.context.rsp = rsp;
        }
        // define a static function as the entry of new thread
        unsafe extern "sysv64" fn entry<F, T>()
        where
            F: Send + 'static + FnOnce() -> T,
            T: Send + 'static,
        {
            let raw = TCB::<F, T>::current();
            if let State::Ready(f) = core::mem::replace(&mut raw.state, State::Running) {
                let ret = f();
                raw.state = State::Exited(ret);
            } else {
                unreachable!()
            }
            yield_now();
        }
        ThreadFuture { inner }
    }
}

impl<F, T> Future for ThreadFuture<F, T>
where
    F: Send + 'static + FnOnce() -> T,
    T: Send + 'static,
{
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut context = ThreadContext::default();
        self.inner.executor_context_ptr = &mut context as *mut _ as usize;
        unsafe {
            context.switch(&mut self.inner.context);
        }
        if let Some(ret) = self.inner.state.take_ret() {
            Poll::Ready(ret)
        } else {
            // yield_now
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

/// Cooperatively gives up the CPU to the executor.
///
/// # Example
/// TODO
pub fn yield_now() {
    unsafe {
        // type `F` and `T` do not matter
        let current = TCB::<fn(), ()>::current();
        // ensure we got a valid structure
        assert_eq!(
            current.canary, CANARY,
            "canary is changed. maybe stack overflow!"
        );
        // switch back to the executor thread
        let executor_context = &mut *(current.executor_context_ptr as *mut _);
        current.context.switch(executor_context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test() {
        let h1 = tokio::spawn(ThreadFuture::new(|| {
            println!("1.1");
            yield_now();
            println!("1.2");
            1u32
        }));
        let h2 = tokio::spawn(ThreadFuture::new(|| {
            println!("2.1");
            yield_now();
            println!("2.2");
            2u32
        }));
        println!("join 1 => {}", h1.await.unwrap());
        println!("join 2 => {}", h2.await.unwrap());
    }
}
