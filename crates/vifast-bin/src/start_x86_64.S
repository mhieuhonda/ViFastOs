    .section .text._start, "ax"
    .globl _start
_start:
    and rsp, -16
    call bin_main
1:
    jmp 1b
