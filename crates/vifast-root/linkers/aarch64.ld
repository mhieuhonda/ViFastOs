ENTRY(_start)
SECTIONS
{
    . = ROOT_ELF_BASE;
    .text : ALIGN(4096) {
        KEEP(*(.text._start))
        *(.text .text.*)
    }
    .rodata : ALIGN(4096) {
        *(.rodata .rodata.*)
    }
    .data : ALIGN(4096) {
        *(.data .data.*)
        *(.got .got.*)
    }
    .bss : ALIGN(4096) {
        *(.bss .bss.*)
        *(COMMON)
    }
    /DISCARD/ : {
        *(.eh_frame*)
        *(.comment*)
        *(.note*)
        *(.dynamic)
        *(.dynsym)
        *(.dynstr)
        *(.rela*)
    }
}
