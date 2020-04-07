//! Convert closures to futures based on greenthread on bare-metal (no_std + no_alloc).

#![cfg_attr(not(test), no_std)]
#![feature(asm)]
#![feature(global_asm)]
#![feature(naked_functions)]
#![feature(untagged_unions)]
#![deny(warnings)]

use core::future::Future;
use core::mem::ManuallyDrop;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

#[cfg(target_arch = "x86_64")]
include!("x86_64.rs");
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
include!("riscv.rs");

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

    /// The waker of task.
    waker: Option<Waker>,

    /// A canary value to detect stack overflow.
    canary: usize,

    /// Thread state. Contains function object or return value.
    state: State<F, T>,
}

unsafe impl<F, T> Send for TCB<F, T> {}

const RAW_SIZE: usize = 0x2000;

#[cfg(target_pointer_width = "32")]
const CANARY: usize = 0xdeadbeaf;
#[cfg(target_pointer_width = "64")]
const CANARY: usize = 0xcafebabe_deadbeaf;

impl<F, T> TCB<F, T> {
    /// Get a mutable reference of current TCB.
    unsafe fn current() -> &'static mut Self {
        let sp = stack_pointer() & !(RAW_SIZE - 1);
        let tcb = &mut *(sp as *mut Self);
        // ensure we got a valid structure
        assert_eq!(
            tcb.canary, CANARY,
            "canary is changed. maybe stack overflow!"
        );
        tcb
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
                waker: None,
                canary: CANARY,
                state: State::Ready(f),
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
                (*context).set_pc(entry::<F, T> as usize);
                raw.tcb.context_ptr = context;
                raw.tcb.waker = Some(cx.waker().clone());
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
            // yield_now or park
            Poll::Pending
        }
    }
}

/// A static function as the entry of new thread
unsafe extern "C" fn entry<F, T>()
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
        // wake up myself, otherwise the executor won't poll me again
        tcb.waker.as_ref().unwrap().wake_by_ref();
        // switch back to the executor thread
        ThreadContext::switch(&mut tcb.context_ptr);
    }
}

/// Blocks unless or until the current thread's token is made available.
pub fn park() {
    unsafe {
        // type `F` and `T` do not matter
        let tcb = TCB::<fn(), ()>::current();
        // switch back to the executor thread
        ThreadContext::switch(&mut tcb.context_ptr);
    }
}

/// Get waker of the current thread.
pub fn current_waker() -> Waker {
    unsafe {
        // type `F` and `T` do not matter
        let tcb = TCB::<fn(), ()>::current();
        tcb.waker.as_ref().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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
        assert_eq!(h1.await.unwrap(), 1);
        assert_eq!(h2.await.unwrap(), 2);
    }

    #[tokio::test]
    async fn sleep_and_wake() {
        let h1 = tokio::spawn(ThreadFuture::from(|| {
            let waker = current_waker();
            tokio::spawn(async move {
                tokio::time::delay_for(Duration::from_millis(10)).await;
                waker.wake();
            });
            park();
        }));
        h1.await.unwrap();
    }
}
