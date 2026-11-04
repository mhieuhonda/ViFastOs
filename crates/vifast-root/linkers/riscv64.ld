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
        *(.srodata .srodata.*)
    }
    .data : ALIGN(4096) {
        *(.data .data.*)
        *(.sdata .sdata.*)
        *(.got .got.*)
    }
    .bss : ALIGN(4096) {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
    }
    __global_pointer$ = . + 0x800;
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
