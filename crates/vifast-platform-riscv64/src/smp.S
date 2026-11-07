    .section .text.entry, "ax"
    .globl secondary_entry
secondary_entry:
    // a0 = hartid (opaque từ HSM start). Mỗi hart phụ một stack 16 KiB
    // riêng: top = base + hart*16384. Trước đây mọi hart dùng chung một
    // stack nên hart 2,3 đè stack hart 1 (mốc 5, vòng 28).
    la sp, __secondary_stack_bottom
    slli t0, a0, 14
    add sp, sp, t0
    call rust_secondary
1:
    wfi
    j 1b

    .section .bss
    .balign 16
__secondary_stack_bottom:
    // 4 hart × 16 KiB (hart 0 không dùng nhưng giữ công thức đơn giản).
    .space 65536
__secondary_stack_top:
