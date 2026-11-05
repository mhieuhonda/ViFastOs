    .section .text.entry, "ax"
    .globl _start
_start:
    /*
     * OpenSBI may pick any hart as the boot hart.  The kernel's primary CPU
     * is hart 0, so a non-zero boot hart starts hart 0 here and then stops
     * itself.  The primary will bring it back through the normal SMP path.
     */
    bnez a0, promote_hart0
    la sp, __stack_top
    call rust_main
1:
    wfi
    j 1b

promote_hart0:
    la sp, __secondary_stack_top
    mv s0, a0
    la a1, _start
    li a7, 0x48534d
    li a6, 0
    li a0, 0
    li a2, 0
    ecall
    bnez a0, 2f
    /* Hart 0 is now primary; stop this hart until start_smp() wakes it. */
    mv a0, s0
    li a7, 0x48534d
    li a6, 1
    ecall
2:
    wfi
    j 2b
