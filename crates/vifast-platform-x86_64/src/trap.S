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
    cld                    # vòng 86: DF của user không được sống vào ring 0
    SAVE_REGS
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    mov rcx, rsp          # efiapi arg1: *mut TrapFrame
    call x86_trap_handler
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    RESTORE_REGS
    iretq

    // Vector 0xFF (spurious LAPIC): handler chỉ cần iretq vì CPU không đẩy
    // frame nào. Vòng 88 (audit #4 P1-3): SVR trỏ 0xFF nhưng IDT chưa có entry
    // nên một lần spurious sẽ #NP và kernel exit(1).
    .globl spurious_entry
spurious_entry:
    iretq

    .globl pf_entry
.globl exc0
exc0:
    push 0
    push 0
    jmp exc_common
.globl exc1
exc1:
    push 0
    push 1
    jmp exc_common
.globl exc2
exc2:
    push 0
    push 2
    jmp exc_common
.globl exc3
exc3:
    push 0
    push 3
    jmp exc_common
.globl exc4
exc4:
    push 0
    push 4
    jmp exc_common
.globl exc5
exc5:
    push 0
    push 5
    jmp exc_common
.globl exc6
exc6:
    push 0
    push 6
    jmp exc_common
.globl exc7
exc7:
    push 0
    push 7
    jmp exc_common
.globl exc8
exc8:
    push 8
    jmp exc_common
.globl exc9
exc9:
    push 0
    push 9
    jmp exc_common
.globl exc10
exc10:
    push 10
    jmp exc_common
.globl exc11
exc11:
    push 11
    jmp exc_common
.globl exc12
exc12:
    push 12
    jmp exc_common
.globl exc13
exc13:
    push 13
    jmp exc_common
.globl exc14
exc14:
    push 14
    jmp exc_common
.globl exc15
exc15:
    push 0
    push 15
    jmp exc_common
.globl exc16
exc16:
    push 0
    push 16
    jmp exc_common
.globl exc17
exc17:
    push 17
    jmp exc_common
.globl exc18
exc18:
    push 0
    push 18
    jmp exc_common
.globl exc19
exc19:
    push 0
    push 19
    jmp exc_common
.globl exc20
exc20:
    push 0
    push 20
    jmp exc_common
.globl exc21
exc21:
    push 21
    jmp exc_common
.globl exc22
exc22:
    push 0
    push 22
    jmp exc_common
.globl exc23
exc23:
    push 0
    push 23
    jmp exc_common
.globl exc24
exc24:
    push 0
    push 24
    jmp exc_common
.globl exc25
exc25:
    push 0
    push 25
    jmp exc_common
.globl exc26
exc26:
    push 0
    push 26
    jmp exc_common
.globl exc27
exc27:
    push 0
    push 27
    jmp exc_common
.globl exc28
exc28:
    push 0
    push 28
    jmp exc_common
.globl exc29
exc29:
    push 29
    jmp exc_common
.globl exc30
exc30:
    push 30
    jmp exc_common
.globl exc31
exc31:
    push 0
    push 31
    jmp exc_common

exc_common:
    cld                    # DF có thể đến từ user (exception sau khi nó set DF)
    SAVE_REGS
    test byte ptr [rsp + 144], 3
    jz 1f
    swapgs
1:
    mov rcx, rsp          # efiapi arg1: frame
    mov rdx, [rsp + 120]  # efiapi arg2: vector
    mov r8, [rsp + 128]   # efiapi arg3: error
    call x86_exception_handler
1:
    hlt
    jmp 1b

pf_entry:
    cld
    SAVE_REGS
    test byte ptr [rsp + 136], 3
    jz 1f
    swapgs
1:
    mov rcx, rsp          # efiapi arg1: frame
    mov rdx, [rsp + 120]  # efiapi arg2: CPU error code
    call x86_page_fault
1:
    hlt
    jmp 1b

    .globl irq0_entry
irq0_entry:
    cld
    SAVE_REGS
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    mov rcx, rsp          # efiapi arg1: *mut TrapFrame
    call x86_irq_handler
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    RESTORE_REGS
    iretq

    .globl irq4_entry
irq4_entry:
    cld
    SAVE_REGS
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    mov rcx, rsp          # efiapi arg1: *mut TrapFrame
    call x86_uart_irq_handler
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    RESTORE_REGS
    iretq

    .globl ap_ipi_entry
ap_ipi_entry:
    mov rdx, 0x30
    jmp ap_irq_common

    .globl ap_timer_entry
ap_timer_entry:
    mov rdx, 0x31
    jmp ap_irq_common

ap_irq_common:
    cld
    SAVE_REGS
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    mov rcx, rsp          # efiapi arg1: *mut TrapFrame
    call x86_ap_irq_handler
    test byte ptr [rsp + 128], 3
    jz 1f
    swapgs
1:
    RESTORE_REGS
    iretq

    .globl ap_halt_entry
ap_halt_entry:
    cli
1:
    hlt
    jmp 1b

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

.section .rodata
.balign 8
.globl x86_exception_table
x86_exception_table:
    .quad exc0
    .quad exc1
    .quad exc2
    .quad exc3
    .quad exc4
    .quad exc5
    .quad exc6
    .quad exc7
    .quad exc8
    .quad exc9
    .quad exc10
    .quad exc11
    .quad exc12
    .quad exc13
    .quad exc14
    .quad exc15
    .quad exc16
    .quad exc17
    .quad exc18
    .quad exc19
    .quad exc20
    .quad exc21
    .quad exc22
    .quad exc23
    .quad exc24
    .quad exc25
    .quad exc26
    .quad exc27
    .quad exc28
    .quad exc29
    .quad exc30
    .quad exc31
