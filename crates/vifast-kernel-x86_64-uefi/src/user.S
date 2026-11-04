    .section .text.userprog, "ax"
    .globl user_program_start
user_program_start:
    .globl user_root
user_root:
    mov rax, 9
    mov rdi, 2
    mov rsi, 2
    mov rdx, 4
    int 0x80
    mov rax, 9
    mov rdi, 2
    mov rsi, 1
    mov rdx, 5
    int 0x80
    mov rax, 9
    mov rdi, 2
    mov rsi, 1
    mov rdx, 6
    int 0x80
    mov rax, 9
    mov rdi, 2
    mov rsi, 3
    mov rdx, 7
    int 0x80

    mov rax, 10
    mov rdi, 4
    mov rsi, 5
    mov rdx, 1
    mov rcx, 1
    int 0x80
    mov rax, 10
    mov rdi, 4
    mov rsi, 6
    mov rdx, 1
    mov rcx, 2
    int 0x80
    mov rax, 10
    mov rdi, 1
    mov rsi, 6
    mov rdx, 2
    mov rcx, 2
    int 0x80
    mov rax, 10
    mov rdi, 3
    mov rsi, 6
    mov rdx, 3
    mov rcx, 2
    int 0x80
    mov rax, 10
    mov rdi, 7
    mov rsi, 6
    mov rdx, 5
    mov rcx, 11
    int 0x80

    mov rax, 15
    mov rdi, 5
    lea rsi, [rip + user_thread_a]
    movabs rdx, 0x1000005000
    mov rcx, 1
    mov r8, 5
    int 0x80
    mov rax, 15
    mov rdi, 6
    lea rsi, [rip + user_thread_b]
    movabs rdx, 0x1000009000
    mov rcx, 1
    mov r8, 5
    int 0x80
    mov rax, 16
    mov rdi, 5
    int 0x80
    mov rax, 16
    mov rdi, 6
    int 0x80
1:
    mov rax, 8
    int 0x80
    jmp 1b

    .globl user_thread_a
user_thread_a:
    mov rax, 32
    lea rdi, [rip + user_message_a]
    mov rsi, 7
    int 0x80
1:
    mov rax, 2
    mov rdi, 1
    mov rsi, 4
    int 0x80
    mov rax, 5
    mov rdi, 4
    int 0x80
    mov rax, 3
    xor rdi, rdi
    mov rsi, 0x55
    mov rdx, 42
    int 0x80
    jmp 1b

    .globl user_thread_b
user_thread_b:
    mov rax, 9
    mov rdi, 3
    mov rsi, 5
    mov rdx, 4
    int 0x80
    mov rax, 25
    mov rdi, 2
    mov rsi, 4
    mov rdx, 0
    mov rcx, 3
    int 0x80
    mov r12, rdi
    lea rsi, [rip + user_mapped_text]
    mov rdx, [rsi]
    mov [r12], rdx
    mov rax, 32
    mov rdi, r12
    mov rsi, 7
    int 0x80
    mov rax, 35
    lea rdi, [rip + user_motd_path]
    mov rsi, 5
    xor rdx, rdx
    int 0x80
    mov r15, rdi
    mov rax, 36
    mov rdi, r15
    mov rsi, r12
    mov rdx, 64
    int 0x80
    mov r14, rdi
    mov rax, 32
    mov rdi, r12
    mov rsi, r14
    int 0x80
    mov rax, 38
    mov rdi, r15
    int 0x80
    mov rax, 35
    lea rdi, [rip + user_scratch_path]
    mov rsi, 8
    mov rdx, 6
    int 0x80
    mov r15, rdi
    mov rax, 37
    mov rdi, r15
    lea rsi, [rip + user_write_text]
    mov rdx, 15
    int 0x80
    mov rax, 40
    mov rdi, r15
    xor rsi, rsi
    int 0x80
    mov rax, 36
    mov rdi, r15
    mov rsi, r12
    mov rdx, 64
    int 0x80
    mov r14, rdi
    mov rax, 32
    mov rdi, r12
    mov rsi, r14
    int 0x80
    mov rax, 38
    mov rdi, r15
    int 0x80
    mov rax, 26
    mov rdi, 2
    mov rsi, r12
    int 0x80
    mov rax, 32
    lea rdi, [rip + user_message_b]
    mov rsi, 7
    int 0x80
    mov r10, 5
2:
    mov rax, 1
    mov rdi, 1
    mov rsi, 0x1234
    mov rdx, 7
    xor rcx, rcx
    xor r8, r8
    mov r9, r10
    int 0x80
    mov r13, rax
    mov r14, rdx
    mov r10, -1
    mov rax, 7
    mov rdi, 5
    int 0x80
    test rdi, rdi
    jz 4f
    mov rax, 32
    lea rdi, [rip + user_cap_ok]
    mov rsi, 7
    int 0x80
    jmp 5f
4:
    mov rax, 32
    lea rdi, [rip + user_cap_bad]
    mov rsi, 8
    int 0x80
5:
    test r13, r13
    jnz 3f
    cmp r14, 42
    jne 3f
    mov rax, 32
    lea rdi, [rip + user_message_c]
    mov rsi, 9
    int 0x80
    jmp 2b
3:
    mov rax, 32
    lea rdi, [rip + user_message_d]
    mov rsi, 10
    int 0x80
    jmp 2b

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
