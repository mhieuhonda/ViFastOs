    .section .text.entry, "ax"
    .globl secondary_entry
secondary_entry:
    /* D1: no kernel interrupts on the secondary CPU. */
    msr daifset, #0xf
    ldr x0, =__secondary_stack_top
    mov sp, x0
    bl rust_secondary
1:
    wfi
    b 1b

    .section .bss
    .balign 16
__secondary_stack_bottom:
    .space 16384
__secondary_stack_top:
