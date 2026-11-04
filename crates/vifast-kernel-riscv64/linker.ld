OUTPUT_ARCH(riscv)
ENTRY(_start)

SECTIONS
{
    . = 0x80200000;

    .text : ALIGN(4K) {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    .bss : ALIGN(4K) {
        __bss_start = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
        __bss_end = .;
    }

    .stack (NOLOAD) : ALIGN(16) {
        __stack_bottom = .;
        . += 0x40000;
        __stack_top = .;
        __kernel_end = .;
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
