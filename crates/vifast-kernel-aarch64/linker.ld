OUTPUT_ARCH(aarch64)
ENTRY(_start)

SECTIONS
{
    . = 0x40080000;

    .text : ALIGN(4K) {
        KEEP(*(.text.entry))
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(4K) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
    }

    .stack (NOLOAD) : ALIGN(16) {
        __stack_bottom = .;
        . += 0x40000;
        __stack_top = .;
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
    }
}
