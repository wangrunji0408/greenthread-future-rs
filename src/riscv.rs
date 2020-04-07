/// Saved registers of a thread.
#[repr(C)]
#[derive(Debug)]
struct ThreadContext {
    /// Callee-saved registers
    s: [usize; 12],
    /// Return address
    ra: usize,
}

#[cfg(target_arch = "riscv32")]
global_asm!(
    r"
.equ XLENB, 4
.macro LOAD reg, mem
    lw \reg, \mem
.endm
.macro STORE reg, mem
    sw \reg, \mem
.endm"
);
#[cfg(target_arch = "riscv64")]
global_asm!(
    r"
.equ XLENB, 8
.macro LOAD reg, mem
    ld \reg, \mem
.endm
.macro STORE reg, mem
    sd \reg, \mem
.endm"
);

impl ThreadContext {
    /// Switch context to another thread.
    #[naked]
    #[inline(never)]
    unsafe extern "C" fn switch(_ptr_ptr: *mut *mut Self) {
        asm!(r#"
        addi  sp, sp, (-XLENB*13)
        STORE s0, 0*XLENB(sp)
        STORE s1, 1*XLENB(sp)
        STORE s2, 2*XLENB(sp)
        STORE s3, 3*XLENB(sp)
        STORE s4, 4*XLENB(sp)
        STORE s5, 5*XLENB(sp)
        STORE s6, 6*XLENB(sp)
        STORE s7, 7*XLENB(sp)
        STORE s8, 8*XLENB(sp)
        STORE s9, 9*XLENB(sp)
        STORE s10, 10*XLENB(sp)
        STORE s11, 11*XLENB(sp)
        STORE ra, 12*XLENB(sp)

        LOAD    t0, (a0)
        STORE   sp, (a0)
        mv      sp, t0

        LOAD s0, 0*XLENB(sp)
        LOAD s1, 1*XLENB(sp)
        LOAD s2, 2*XLENB(sp)
        LOAD s3, 3*XLENB(sp)
        LOAD s4, 4*XLENB(sp)
        LOAD s5, 5*XLENB(sp)
        LOAD s6, 6*XLENB(sp)
        LOAD s7, 7*XLENB(sp)
        LOAD s8, 8*XLENB(sp)
        LOAD s9, 9*XLENB(sp)
        LOAD s10, 10*XLENB(sp)
        LOAD s11, 11*XLENB(sp)
        LOAD ra, 12*XLENB(sp)
        addi sp, sp, (XLENB*13)
        "# :::: "volatile");
    }

    /// Set value of program counter.
    fn set_pc(&mut self, pc: usize) {
        self.ra = pc;
    }
}

/// Get stack pointer.
#[inline(always)]
unsafe fn stack_pointer() -> usize {
    let mut sp: usize;
    asm!("" : "={x2}"(sp));
    sp
}
