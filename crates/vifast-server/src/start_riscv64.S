    .section .text._start, "ax"
    .globl _start
_start:
    andi sp, sp, -16
    call server_main
1:
    j 1b
