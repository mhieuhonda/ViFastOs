/* r249 (T13): script cho CHƯƠNG TRÌNH NGƯỜI DÙNG (client + chương trình ngoài).
 *
 * Bản gốc `riscv64.ld` ALIGN(4096) từng section, nên một ảnh client nhỏ có tới
 * BỐN segment căn trang ⇒ 4 trang "đệm" trong file ⇒ 4 Frame staging lúc boot.
 *
 * Ở đây gộp `.rodata` vào CHÍNH output section `.text`: rodata trở thành
 * executable (chấp nhận được — hằng chuỗi, không có bí mật) nhưng bất biến
 * W^X vẫn giữ vì không có segment nào vừa ghi vừa chạy. Nhờ vậy còn 2 segment:
 * một R-X và một RW.
 *
 * Vì sao KHÔNG bỏ hẳn căn trang: `vifast_elf::parse` làm tròn `memory_end` của
 * mỗi segment lên trang rồi đòi các khoảng KHÔNG chồng nhau (loader map theo
 * trang), nên hai segment nằm chung một trang bị từ chối với `Overlap` —
 * đo được ở r249: bản bỏ hết ALIGN làm boot in `MODULE PARSE BAD` + `ROOT
 * PANIC` trên cả 3 arch.
 */
ENTRY(_start)
SECTIONS
{
    . = ROOT_ELF_BASE;
    .text : ALIGN(4096) {
        KEEP(*(.text._start))
        *(.text .text.*)
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }
    .data : ALIGN(4096) {
        *(.data .data.*)
        *(.sdata .sdata.*)
        *(.got .got.*)
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
