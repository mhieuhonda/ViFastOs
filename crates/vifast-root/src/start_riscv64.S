    .section .text._start, "ax"
    .globl _start
_start:
    .option push
    .option norelax
    la gp, __global_pointer$
    .option pop
    andi sp, sp, -16
    call root_main
1:
    j 1b
