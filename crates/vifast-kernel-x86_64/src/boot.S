/* Multiboot 1 header.  QEMU's -kernel loader understands this format. */
.section .multiboot, "a"
.align 4
.long 0x1BADB002
.long 0x00000003
.long -(0x1BADB002 + 0x00000003)

.section .text32, "ax"
.code32
.global _start
_start:
    cli
    mov esp, offset stack_top

    /* Zero the three page tables. */
    xor eax, eax
    mov edi, offset pml4
    mov ecx, 3 * 1024
    rep stosd

    /* PML4[0] -> PDPT */
    mov eax, offset pdpt
    or eax, 0x3
    mov DWORD PTR [pml4], eax
    mov DWORD PTR [pml4 + 4], 0

    /* PDPT[0] -> PD */
    mov eax, offset pd
    or eax, 0x3
    mov DWORD PTR [pdpt], eax
    mov DWORD PTR [pdpt + 4], 0

    /* Identity map the first 1 GiB with 2 MiB pages. */
    mov edi, offset pd
    mov eax, 0x83
    mov ecx, 512
1:
    mov DWORD PTR [edi], eax
    mov DWORD PTR [edi + 4], 0
    add eax, 0x200000
    add edi, 8
    loop 1b

    /* Load PML4. */
    mov eax, offset pml4
    mov cr3, eax

    /* Enable PAE. */
    mov eax, cr4
    or eax, 0x20
    mov cr4, eax

    /* Enable long mode in EFER. */
    mov ecx, 0xC0000080
    rdmsr
    or eax, 0x100
    wrmsr

    /* Enable paging and protection. */
    mov eax, cr0
    or eax, 0x80000001
    mov cr0, eax

    lgdt [gdt64_ptr]

    /* Far jump into the 64-bit code segment. */
    .byte 0xEA
    .long long_mode
    .word 0x08

.section .rodata, "a"
.align 8
gdt64:
    .quad 0
    .quad 0x00AF9A000000FFFF
    .quad 0x00AF92000000FFFF
gdt64_end:
gdt64_ptr:
    .word gdt64_end - gdt64 - 1
    .long gdt64

.section .text, "ax"
.code64
.global long_mode
long_mode:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax

    lea rsp, [rip + stack_top]
    call rust_main

1:
    hlt
    jmp 1b

.section .bss, "aw", @nobits
.align 4096
pml4:
    .skip 4096
pdpt:
    .skip 4096
pd:
    .skip 4096
.align 16
stack_bottom:
    .skip 262144
stack_top:
