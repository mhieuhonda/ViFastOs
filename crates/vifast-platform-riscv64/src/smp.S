    .section .text.entry, "ax"
    .globl secondary_entry
secondary_entry:
    la sp, __secondary_stack_top
    call rust_secondary
1:
    wfi
    j 1b

    .section .bss
    .balign 16
__secondary_stack_bottom:
    .space 16384
__secondary_stack_top:
