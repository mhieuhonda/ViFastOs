    .section .text, "ax"

    .macro SAVE_FRAME
    sub sp, sp, #0x120
    stp x0, x1, [sp, #0x00]
    stp x2, x3, [sp, #0x10]
    stp x4, x5, [sp, #0x20]
    stp x6, x7, [sp, #0x30]
    stp x8, x9, [sp, #0x40]
    stp x10, x11, [sp, #0x50]
    stp x12, x13, [sp, #0x60]
    stp x14, x15, [sp, #0x70]
    stp x16, x17, [sp, #0x80]
    stp x18, x19, [sp, #0x90]
    stp x20, x21, [sp, #0xa0]
    stp x22, x23, [sp, #0xb0]
    stp x24, x25, [sp, #0xc0]
    stp x26, x27, [sp, #0xd0]
    stp x28, x29, [sp, #0xe0]
    str x30, [sp, #0xf0]
    mrs x0, sp_el0
    str x0, [sp, #0xf8]
    mrs x0, elr_el1
    str x0, [sp, #0x100]
    mrs x0, spsr_el1
    str x0, [sp, #0x108]
    mrs x0, esr_el1
    str x0, [sp, #0x110]
    mrs x0, far_el1
    str x0, [sp, #0x118]
    .endm

    .macro RESTORE_FRAME
    ldr x0, [sp, #0x100]
    msr elr_el1, x0
    ldr x0, [sp, #0x108]
    msr spsr_el1, x0
    ldr x0, [sp, #0xf8]
    msr sp_el0, x0
    ldp x0, x1, [sp, #0x00]
    ldp x2, x3, [sp, #0x10]
    ldp x4, x5, [sp, #0x20]
    ldp x6, x7, [sp, #0x30]
    ldp x8, x9, [sp, #0x40]
    ldp x10, x11, [sp, #0x50]
    ldp x12, x13, [sp, #0x60]
    ldp x14, x15, [sp, #0x70]
    ldp x16, x17, [sp, #0x80]
    ldp x18, x19, [sp, #0x90]
    ldp x20, x21, [sp, #0xa0]
    ldp x22, x23, [sp, #0xb0]
    ldp x24, x25, [sp, #0xc0]
    ldp x26, x27, [sp, #0xd0]
    ldp x28, x29, [sp, #0xe0]
    ldr x30, [sp, #0xf0]
    add sp, sp, #0x120
    .endm

    .balign 2048
    .globl vector_table
vector_table:
    .rept 8
    b unhandled
    .space 0x7c
    .endr
    b sync_lower
    .space 0x7c
    b irq_lower
    .space 0x7c
    .rept 6
    b unhandled
    .space 0x7c
    .endr

unhandled:
    /* Any current-EL or unsupported exception is a kernel bug. */
    bl aarch64_kernel_fault
    wfi
    b unhandled

sync_lower:
    SAVE_FRAME
    mov x0, sp
    bl aarch64_trap_handler
    RESTORE_FRAME
    eret

irq_lower:
    SAVE_FRAME
    mov x0, sp
    bl aarch64_irq_handler
    RESTORE_FRAME
    eret
