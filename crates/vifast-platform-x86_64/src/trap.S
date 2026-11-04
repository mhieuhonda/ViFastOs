    .section .text, "ax"
    .macro SAVE_REGS
    push rax
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    .endm

    .macro RESTORE_REGS
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    pop rax
    .endm

    .globl int80_entry
int80_entry:
    SAVE_REGS
    mov rcx, rsp
    call x86_trap_handler
    RESTORE_REGS
    iretq

    .globl pf_entry
pf_entry:
    SAVE_REGS
    mov rdi, rsp
    call x86_page_fault
1:
    hlt
    jmp 1b

    .globl irq0_entry
irq0_entry:
    SAVE_REGS
    mov rcx, rsp
    call x86_irq_handler
    RESTORE_REGS
    iretq

    .globl x86_reload_segments
x86_reload_segments:
    lea rax, [rip + 1f]
    push 0x08
    push rax
    retfq
1:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax
    ret
