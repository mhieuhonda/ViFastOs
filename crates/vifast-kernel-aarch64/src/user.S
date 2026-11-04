    .section .text.userprog, "ax"
    .globl user_program_start
user_program_start:
    .globl user_entry_server
user_entry_server:
    b user_thread_a
    .balign 16
    .globl user_entry_client
user_entry_client:
    b user_thread_b
    .balign 16
    .globl user_entry_fault1
user_entry_fault1:
    b user_thread_fault
    .balign 16
    .globl user_entry_fault2
user_entry_fault2:
    b user_thread_fault2
    .balign 16
    .globl user_entry_fault3
user_entry_fault3:
    b user_thread_fault3
    .balign 16

    .globl user_root
user_root:
    mov x8, #9
    mov x0, #2
    mov x1, #2
    mov x2, #4
    svc #0
    mov x8, #9
    mov x0, #2
    mov x1, #1
    mov x2, #5
    svc #0
    mov x8, #9
    mov x0, #2
    mov x1, #1
    mov x2, #6
    svc #0
    mov x8, #9
    mov x0, #2
    mov x1, #3
    mov x2, #7
    svc #0
    mov x8, #9
    mov x0, #2
    mov x1, #3
    mov x2, #8
    svc #0
    mov x8, #9
    mov x0, #2
    mov x1, #1
    mov x2, #9
    svc #0
    mov x8, #9
    mov x0, #2
    mov x1, #1
    mov x2, #10
    svc #0
    mov x8, #9
    mov x0, #2
    mov x1, #1
    mov x2, #11
    svc #0

    mov x8, #10
    mov x0, #4
    mov x1, #5
    mov x2, #1
    mov x3, #1
    svc #0
    mov x8, #10
    mov x0, #4
    mov x1, #6
    mov x2, #1
    mov x3, #2
    svc #0
    mov x8, #10
    mov x0, #1
    mov x1, #6
    mov x2, #2
    mov x3, #2
    svc #0
    mov x8, #10
    mov x0, #3
    mov x1, #6
    mov x2, #3
    mov x3, #2
    svc #0
    mov x8, #10
    mov x0, #7
    mov x1, #6
    mov x2, #5
    mov x3, #11
    svc #0

    mov x8, #15
    mov x0, #5
    adr x1, user_thread_a
    ldr x2, ={SERVER_STACK_TOP}
    mov x3, {SERVER_VSPACE_SLOT}
    mov x4, #5
    svc #0
    mov x8, #15
    mov x0, #6
    adr x1, user_thread_b
    ldr x2, ={CLIENT_STACK_TOP}
    mov x3, #1
    mov x4, #5
    svc #0
    mov x8, #16
    mov x0, #5
    svc #0
    mov x8, #16
    mov x0, #6
    svc #0
    mov x8, #15
    mov x0, #9
    adr x1, user_thread_fault
    ldr x2, ={FAULT1_STACK_TOP}
    mov x3, #1
    mov x4, #6
    svc #0
    mov x8, #16
    mov x0, #9
    svc #0
    mov x8, #15
    mov x0, #10
    adr x1, user_thread_fault2
    ldr x2, ={FAULT2_STACK_TOP}
    mov x3, #1
    mov x4, #6
    svc #0
    mov x8, #16
    mov x0, #10
    svc #0
    mov x8, #15
    mov x0, #11
    adr x1, user_thread_fault3
    ldr x2, ={FAULT3_STACK_TOP}
    mov x3, #1
    mov x4, #6
    svc #0
    mov x8, #16
    mov x0, #11
    svc #0
    mov x8, #6
    mov x0, #8
    svc #0
1:
    b 1b

    .globl user_thread_fault
user_thread_fault:
    mov x8, #32
    adr x0, user_fault_msg
    mov x1, #6
    svc #0
    mov x1, #0x8000
    lsl x1, x1, #16
    ldr x0, [x1]
1:
    b 1b

    .globl user_thread_fault2
user_thread_fault2:
    mov x8, #32
    adr x0, user_fault2_msg
    mov x1, #7
    svc #0
    hlt #0
1:
    b 1b

    .globl user_thread_fault3
user_thread_fault3:
    mov x8, #32
    adr x0, user_isolation_msg
    mov x1, #10
    svc #0
    ldr x1, ={SERVER_STACK_BASE}
    str xzr, [x1]
1:
    b 1b

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
    mov w20, #0xdead
    str w20, [x19]
    mov x8, #26
    mov x0, #2
    mov x1, x19
    mov x2, #4
    svc #0
    mov x8, #14
    mov x0, #3
    svc #0
    mov x8, #9
    mov x0, #3
    mov x1, #5
    mov x2, #4
    svc #0
    mov x8, #25
    mov x0, #2
    mov x1, #4
    ldr x2, ={DATA_VA}
    mov x3, #3
    svc #0
    mov x19, x1
    ldr w20, [x19]
    cbz w20, 7f
    mov x8, #32
    adr x0, user_zero_bad
    mov x1, #9
    svc #0
    b 8f
7:
    mov x8, #32
    adr x0, user_zero_ok
    mov x1, #8
    svc #0
8:
    mov x8, #26
    mov x0, #2
    mov x1, x19
    mov x2, #4
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
user_fault_msg:
    .asciz "Fault\n"
user_fault2_msg:
    .asciz "Fault2\n"
user_zero_ok:
    .asciz "Zero OK\n"
user_zero_bad:
    .asciz "Zero BAD\n"
user_isolation_msg:
    .asciz "Isolation\n"
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
