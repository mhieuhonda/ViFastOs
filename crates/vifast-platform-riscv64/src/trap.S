    .section .text.entry, "ax"
    .balign 4
    .globl trap_entry
trap_entry:
    /* User: tp is user-controlled, sscratch points at the kernel CpuState.
       Kernel: sscratch is zero, tp points at CpuState.  The trap entry swaps
       the two, so the user can never redirect the kernel state pointer. */
    csrrw tp, sscratch, tp
    beqz tp, 9f

    /* Save the user stack in CpuState.user_sp, then switch to the trap stack. */
    sd sp, 24(tp)
    ld sp, 16(tp)
    addi sp, sp, -0x120

    sd x1, 8(sp)
    sd x3, 24(sp)
    sd x5, 40(sp)
    sd x6, 48(sp)
    sd x7, 56(sp)
    sd x8, 64(sp)
    sd x9, 72(sp)
    sd x10, 80(sp)
    sd x11, 88(sp)
    sd x12, 96(sp)
    sd x13, 104(sp)
    sd x14, 112(sp)
    sd x15, 120(sp)
    sd x16, 128(sp)
    sd x17, 136(sp)
    sd x18, 144(sp)
    sd x19, 152(sp)
    sd x20, 160(sp)
    sd x21, 168(sp)
    sd x22, 176(sp)
    sd x23, 184(sp)
    sd x24, 192(sp)
    sd x25, 200(sp)
    sd x26, 208(sp)
    sd x27, 216(sp)
    sd x28, 224(sp)
    sd x29, 232(sp)
    sd x30, 240(sp)
    sd x31, 248(sp)

    /* x4 is the user's tp, still in sscratch. */
    csrr t0, sscratch
    sd t0, 32(sp)
    /* x2 is the user's sp, saved in CpuState.user_sp. */
    ld t0, 24(tp)
    sd t0, 16(sp)

    /* Keep the kernel CpuState pointer in s0 across the Rust call. */
    mv s0, tp
    csrw sscratch, zero

    csrr t0, sepc
    sd t0, 256(sp)
    csrr t0, sstatus
    sd t0, 264(sp)
    csrr t0, scause
    sd t0, 272(sp)

    mv a0, sp
    call riscv_trap_handler

    ld t0, 256(sp)
    csrw sepc, t0
    ld t0, 264(sp)
    csrw sstatus, t0

    /* Restore the kernel-only CpuState pointer for the next user trap. */
    csrw sscratch, s0

    ld x1, 8(sp)
    ld x3, 24(sp)
    ld x4, 32(sp)
    ld x5, 40(sp)
    ld x6, 48(sp)
    ld x7, 56(sp)
    ld x8, 64(sp)
    ld x9, 72(sp)
    ld x10, 80(sp)
    ld x11, 88(sp)
    ld x12, 96(sp)
    ld x13, 104(sp)
    ld x14, 112(sp)
    ld x15, 120(sp)
    ld x16, 128(sp)
    ld x17, 136(sp)
    ld x18, 144(sp)
    ld x19, 152(sp)
    ld x20, 160(sp)
    ld x21, 168(sp)
    ld x22, 176(sp)
    ld x23, 184(sp)
    ld x24, 192(sp)
    ld x25, 200(sp)
    ld x26, 208(sp)
    ld x27, 216(sp)
    ld x28, 224(sp)
    ld x29, 232(sp)
    ld x30, 240(sp)
    ld x31, 248(sp)
    ld sp, 16(sp)
    sret

9:
    /* Trap from S-mode: sp is still the interrupted kernel stack. */
    call riscv_kernel_trap
    j 9b
