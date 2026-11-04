    .section .text._start, "ax"
    .globl _start
_start:
    mov x29, xzr
    mov x30, xzr
    mov x9, sp
    and x9, x9, #0xfffffffffffffff0
    mov sp, x9
    bl client_main
1:
    b 1b
