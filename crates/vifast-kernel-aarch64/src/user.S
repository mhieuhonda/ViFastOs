    .section .text.userprog, "ax"
    .globl user_program_start
user_program_start:
    .globl user_thread_a
user_thread_a:
    mov x8, #32
    adr x0, user_message_a
    mov x1, #7
    svc #0
1:
    mov x8, #2
    mov x0, #1
    mov x1, #4
    svc #0
    mov x8, #5
    mov x0, #4
    svc #0
    mov x8, #3
    mov x1, #0x55
    mov x2, #42
    svc #0
    b 1b

    .globl user_thread_b
user_thread_b:
    mov x8, #9
    mov x0, #3
    mov x1, #5
    mov x2, #4
    svc #0
    mov x8, #25
    mov x0, #2
    mov x1, #4
    mov x2, #0
    mov x3, #3
    svc #0
    mov x19, x1
    adr x0, user_mapped_text
    ldr x2, [x0]
    str x2, [x19]
    mov x8, #32
    mov x0, x19
    mov x1, #7
    svc #0
    mov x8, #35
    adr x0, user_motd_path
    mov x1, #5
    mov x2, #0
    svc #0
    mov x22, x1
    mov x8, #36
    mov x0, x22
    mov x1, x19
    mov x2, #64
    svc #0
    mov x23, x1
    mov x8, #32
    mov x0, x19
    mov x1, x23
    svc #0
    mov x8, #38
    mov x0, x22
    svc #0
    mov x8, #35
    adr x0, user_scratch_path
    mov x1, #8
    mov x2, #6
    svc #0
    mov x22, x1
    mov x8, #37
    mov x0, x22
    adr x1, user_write_text
    mov x2, #15
    svc #0
    mov x8, #40
    mov x0, x22
    mov x1, #0
    svc #0
    mov x8, #36
    mov x0, x22
    mov x1, x19
    mov x2, #64
    svc #0
    mov x23, x1
    mov x8, #32
    mov x0, x19
    mov x1, x23
    svc #0
    mov x8, #38
    mov x0, x22
    svc #0
    mov x8, #26
    mov x0, #2
    mov x1, x19
    svc #0
    mov x8, #32
    adr x0, user_message_b
    mov x1, #7
    svc #0
    mov x24, #5
2:
    mov x8, #1
    mov x0, #1
    mov x1, #0x1234
    mov x2, #7
    mov x3, #0
    mov x4, #0
    mov x5, x24
    svc #0
    mov x20, x0
    mov x21, x3
    mov x24, #-1
    mov x8, #7
    mov x0, #5
    svc #0
    cbz x1, 4f
    mov x8, #32
    adr x0, user_cap_ok
    mov x1, #7
    svc #0
    b 5f
4:
    mov x8, #32
    adr x0, user_cap_bad
    mov x1, #8
    svc #0
5:
    cbnz x20, 3f
    cmp x21, #42
    b.ne 3f
    mov x8, #32
    adr x0, user_message_c
    mov x1, #9
    svc #0
    b 2b
3:
    mov x8, #32
    adr x0, user_message_d
    mov x1, #10
    svc #0
    b 2b

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
    .balign 8
user_mapped_text:
    .ascii "Mapped\n\0"
user_motd_path:
    .ascii "/motd"
user_scratch_path:
    .ascii "/scratch"
user_write_text:
    .asciz "ViFastOS write\n"
    .globl user_program_end
user_program_end:
