/// Saved registers of a thread.
#[repr(C)]
#[derive(Debug)]
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

    /// Set value of program counter.
    fn set_pc(&mut self, pc: usize) {
        self.rip = pc;
    }
}

/// Get stack pointer.
#[inline(always)]
unsafe fn stack_pointer() -> usize {
    let mut sp: usize;
    asm!("" : "={rsp}"(sp));
    sp
}
