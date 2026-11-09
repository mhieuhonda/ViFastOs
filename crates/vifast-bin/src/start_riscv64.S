    .section .text._start, "ax"
    .globl _start
_start:
    andi sp, sp, -16
    call bin_main
1:
    j 1b
