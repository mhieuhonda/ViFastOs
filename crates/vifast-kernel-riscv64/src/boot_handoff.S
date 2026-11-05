    .section .bss
    .globl __boot_handoff_from
__boot_handoff_from:
    .byte 0
__boot_handoff_sim:
    .byte 0

    .section .text.entry, "ax"
    .globl _start
_start:
    /*
     * Test-only boot-handoff injection.  A primary hart 0 first simulates
     * OpenSBI choosing hart 1, then hart 1 exercises the normal fallback.
     */
    bnez a0, promote_hart0
    la t0, __boot_handoff_sim
    lb t1, 0(t0)
    bnez t1, normal_primary
    li t1, 1
    sb t1, 0(t0)
    la t0, __boot_handoff_from
    li t1, 1
    sb t1, 0(t0)
    la sp, __stack_top
    la a1, _start
    li a7, 0x48534d
    li a6, 0
    li a0, 1
    li a2, 0
    ecall
    bnez a0, normal_primary
    /* The simulated boot hart is now hart 1; stop this hart. */
    li a7, 0x48534d
    li a6, 1
    li a0, 0
    ecall
3:
    wfi
    j 3b

normal_primary:
    la sp, __stack_top
    call rust_main

promote_hart0:
    /* The production fallback: promote hart 0, then stop this hart. */
    la t0, __boot_handoff_from
    li t1, 1
    sb t1, 0(t0)
    la t0, __boot_handoff_sim
    li t1, 1
    sb t1, 0(t0)
    la sp, __secondary_stack_top
    mv s0, a0
    /* Hart 0 may still be executing its simulated stop.  Retry HSM start
     * until it is actually stopped; this closes the handoff race. */
    li s2, 0
4:
    la a1, _start
    li a7, 0x48534d
    li a6, 0
    li a0, 0
    li a2, 0
    ecall
    beqz a0, 6f
    addi s2, s2, 1
    li t0, 100000
5:
    addi t0, t0, -1
    bnez t0, 5b
    li t0, 1000
    bge s2, t0, 2f
    j 4b
6:
    mv a0, s0
    li a7, 0x48534d
    li a6, 1
    ecall
2:
    wfi
    j 2b
