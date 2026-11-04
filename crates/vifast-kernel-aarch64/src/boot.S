    .section .text.entry, "ax"
    .globl _start
_start:
    mrs x0, CurrentEL
    and x0, x0, #0xc
    cmp x0, #0x8
    b.eq 2f
1:
    ldr x0, =__stack_top
    mov sp, x0
    bl rust_main
3:
    wfi
    b 3b

2:
    /* Drop from EL2 to EL1. */
    mov x0, #0x80000000
    msr hcr_el2, x0
    mov x0, #0x3c5
    msr spsr_el2, x0
    adr x0, 1b
    msr elr_el2, x0
    isb
    eret
