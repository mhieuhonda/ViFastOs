    .section .text.userprog, "ax"
    .globl user_program_start
user_program_start:
    .globl user_thread_a
user_thread_a:
    li a7, 32
    la a0, user_message_a
    li a1, 7
    ecall
1:
    li a7, 2
    li a0, 1
    li a1, 4
    ecall
    li a7, 5
    li a0, 4
    ecall
    li a7, 3
    li a1, 0x55
    li a2, 42
    ecall
    j 1b

    .globl user_thread_b
user_thread_b:
    li a7, 9
    li a0, 3
    li a1, 5
    li a2, 4
    ecall
    li a7, 25
    li a0, 2
    li a1, 4
    li a2, 0
    li a3, 3
    ecall
    mv t3, a1
    la t4, user_mapped_text
    lw t5, 0(t4)
    sw t5, 0(t3)
    lw t5, 4(t4)
    sw t5, 4(t3)
    li a7, 32
    mv a0, t3
    li a1, 7
    ecall
    li a7, 35
    la a0, user_motd_path
    li a1, 5
    li a2, 0
    ecall
    mv t4, a1
    li a7, 36
    mv a0, t4
    mv a1, t3
    li a2, 64
    ecall
    mv t5, a1
    li a7, 32
    mv a0, t3
    mv a1, t5
    ecall
    li a7, 38
    mv a0, t4
    ecall
    li a7, 35
    la a0, user_scratch_path
    li a1, 8
    li a2, 6
    ecall
    mv t4, a1
    li a7, 37
    mv a0, t4
    la a1, user_write_text
    li a2, 15
    ecall
    li a7, 40
    mv a0, t4
    li a1, 0
    ecall
    li a7, 36
    mv a0, t4
    mv a1, t3
    li a2, 64
    ecall
    mv t5, a1
    li a7, 32
    mv a0, t3
    mv a1, t5
    ecall
    li a7, 38
    mv a0, t4
    ecall
    li a7, 26
    li a0, 2
    mv a1, t3
    ecall
    li a7, 32
    la a0, user_message_b
    li a1, 7
    ecall
2:
    li a7, 1
    li a0, 1
    li a1, 0x1234
    li a2, 7
    li a3, 0
    li a4, 0
    li a5, 5
    ecall
    mv t6, a0
    mv t5, a3
    li a7, 7
    li a0, 5
    ecall
    beqz a1, 4f
    li a7, 32
    la a0, user_cap_ok
    li a1, 7
    ecall
    j 5f
4:
    li a7, 32
    la a0, user_cap_bad
    li a1, 8
    ecall
5:
    bnez t6, 3f
    li t0, 42
    bne t5, t0, 3f
    li a7, 32
    la a0, user_message_c
    li a1, 9
    ecall
    j 2b
3:
    li a7, 32
    la a0, user_message_d
    li a1, 10
    ecall
    j 2b

    .balign 8
user_mapped_text:
    .ascii "Mapped\n\0"
user_motd_path:
    .ascii "/motd"
user_scratch_path:
    .ascii "/scratch"
user_write_text:
    .asciz "ViFastOS write\n"
user_message_a:
    .asciz "Server\n"
user_message_b:
    .asciz "Client\n"
user_message_c:
    .asciz "Reply OK\n"
user_message_d:
    .asciz "Reply BAD\n"
user_cap_ok:
    .asciz "Cap OK\n"
user_cap_bad:
    .asciz "Cap BAD\n"
    .globl user_program_end
user_program_end:
