    .section .text._start, "ax"
    .globl _start
_start:
    /* The kernel enters with a 16-byte-aligned user stack.  A normal call
       then gives root_main the SysV entry alignment (rsp % 16 == 8). */
    and rsp, -16
    call root_main
1:
    jmp 1b
