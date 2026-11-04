    .section .text.userprog, "ax"
    .globl user_program_start
user_program_start:
    .globl user_root
user_root:
    /* Root task: create endpoint, two TCBs and a notification from the
       metadata Untyped passed by the runtime. */
    li a7, 9
    li a0, 2
    li a1, 2
    li a2, 4
    ecall
    li a7, 9
    li a0, 2
    li a1, 1
    li a2, 5
    ecall
    li a7, 9
    li a0, 2
    li a1, 1
    li a2, 6
    ecall
    li a7, 9
    li a0, 2
    li a1, 3
    li a2, 7
    ecall
    /* Separate notification used only to block root after bootstrap. */
    li a7, 9
    li a0, 2
    li a1, 3
    li a2, 8
    ecall
    /* Fault-injection thread: must be suspended, not panic the kernel. */
    li a7, 9
    li a0, 2
    li a1, 1
    li a2, 9
    ecall

    /* endpoint -> server slot 1 (READ), client slot 1 (WRITE) */
    li a7, 10
    li a0, 4
    li a1, 5
    li a2, 1
    li a3, 1
    ecall
    li a7, 10
    li a0, 4
    li a1, 6
    li a2, 1
    li a3, 2
    ecall
    /* VSpace + data Untyped for the client */
    li a7, 10
    li a0, 1
    li a1, 6
    li a2, 2
    li a3, 2
    ecall
    li a7, 10
    li a0, 3
    li a1, 6
    li a2, 3
    li a3, 2
    ecall
    /* notification -> client slot 5 with GRANT; first Call transfers it
       into server slot 4. */
    li a7, 10
    li a0, 7
    li a1, 6
    li a2, 5
    li a3, 11
    ecall

    /* Configure and resume the two child tasks. */
    li a7, 15
    li a0, 5
    la a1, user_thread_a
    li a2, 0x1000005000
    li a3, 1
    li a4, 5
    ecall
    li a7, 15
    li a0, 6
    la a1, user_thread_b
    li a2, 0x1000009000
    li a3, 1
    li a4, 5
    ecall
    li a7, 16
    li a0, 5
    ecall
    li a7, 16
    li a0, 6
    ecall
    li a7, 15
    li a0, 9
    la a1, user_thread_fault
    li a2, 0x1000005000
    li a3, 1
    li a4, 6
    ecall
    li a7, 16
    li a0, 9
    ecall
    /* Root has no policy loop; block until someone signals slot 8. */
    li a7, 6
    li a0, 8
    ecall
1:
    j 1b

    .globl user_thread_fault
user_thread_fault:
    li a7, 32
    la a0, user_fault_msg
    li a1, 6
    ecall
    li t0, 0x80000000
    ld t1, 0(t0)
1:
    j 1b

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
    li s0, 5
2:
    li a7, 1
    li a0, 1
    li a1, 0x1234
    li a2, 7
    li a3, 0
    li a4, 0
    mv a5, s0
    ecall
    mv t6, a0
    mv t5, a3
    li s0, -1
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
user_fault_msg:
    .asciz "Fault\n"
    .globl user_program_end
user_program_end:
