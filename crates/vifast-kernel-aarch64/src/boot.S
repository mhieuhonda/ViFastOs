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
    /* Paint the boot stack so the exit path can report a high-water mark. */
    ldr x1, =__stack_bottom
    mov w2, #0xA5
.Lpaint_boot_stack:
    strb w2, [x1], #1
    cmp x1, x0
    b.lo .Lpaint_boot_stack
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
