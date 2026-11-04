    .section .text.entry, "ax"
    .globl _start
_start:
    /* Secondary harts may enter here on some OpenSBI configurations. */
    bnez a0, secondary_entry
    la sp, __stack_top
    call rust_main
1:
    wfi
    j 1b
