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
    /*
     * r227 (T2): tô boot stack bằng 0xA5 giống boot.S. Bản handoff trước đây
     * bỏ mất bước này, nên vùng .bss (toàn số 0) bị đo thành "đã dùng 100%"
     * (used=196608/196608) và accept() từ chối vì HWM vượt 50%. Bài học: số đo
     * sai còn tệ hơn không đo — nó chỉ sai lệch sang một kết luận khác.
     */
    la t0, __stack_bottom
    la t1, __stack_top
    li t2, 0xA5A5A5A5
.Lhandoff_paint:
    sw t2, 0(t0)
    addi t0, t0, 4
    bltu t0, t1, .Lhandoff_paint
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
    bge s2, t0, handoff_fail
    j 4b
6:
    mv a0, s0
    li a7, 0x48534d
    li a6, 1
    ecall
2:
    wfi
    j 2b

handoff_fail:
    /*
     * Hart 0 never became startable.  Do not stop this hart and leave both
     * harts parked; say why and reset the machine instead.
     */
    la t0, boot_handoff_fail_msg
    li t1, 0x10000000
7:
    lb t2, 0(t0)
    beqz t2, 8f
9:
    lb t3, 5(t1)
    andi t3, t3, 0x20
    beqz t3, 9b
    sb t2, 0(t1)
    addi t0, t0, 1
    j 7b
8:
    li a7, 0x53525354
    li a6, 0
    li a0, 0
    li a1, 0
    ecall
10:
    wfi
    j 10b

    .section .rodata
boot_handoff_fail_msg:
    .asciz "BOOT HANDOFF FAIL\r\n"
