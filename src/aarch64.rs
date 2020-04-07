#[repr(C)]
#[derive(Debug)]
pub struct ThreadContext {
    x19to29: [usize; 11],
    lr: usize,
}

impl ThreadContext {
    /// Switch context to another thread.
    #[naked]
    #[inline(never)]
    unsafe extern "C" fn switch(_ptr_ptr: *mut *mut Self) {
        asm!(
        "
        // store callee-saved registers
        stp x29, lr, [sp, #-16]!
        stp x27, x28, [sp, #-16]!
        stp x25, x26, [sp, #-16]!
        stp x23, x24, [sp, #-16]!
        stp x21, x22, [sp, #-16]!
        stp x19, x20, [sp, #-16]!

        // load target sp
        mov x8, sp
        ldr x9, [x0]
        str x8, [x0]
        mov sp, x9

        // load callee-saved registers
        ldp x19, x20, [sp], #16
        ldp x21, x22, [sp], #16
        ldp x23, x24, [sp], #16
        ldp x25, x26, [sp], #16
        ldp x27, x28, [sp], #16
        ldp x29, lr, [sp], #16
        " : : : : "volatile" );
    }

    /// Set value of program counter.
    fn set_pc(&mut self, pc: usize) {
        self.lr = pc;
    }
}

/// Get stack pointer.
#[inline(always)]
unsafe fn stack_pointer() -> usize {
    let mut sp: usize;
    asm!("" : "={sp}"(sp));
    sp
}
