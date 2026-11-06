# ViFastOs — Báo cáo vòng 54 (DTB đọc được trên RISC-V, pha 4 bước 3)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `a5a9e83`
**Mốc:** Xác minh device tree đọc được — đường an toàn thay fw_cfg MMIO.

## 1. Thay đổi

- `init_platform` RISC-V/AArch64: nếu `info.dtb != 0`, đọc u32 đầu và log
  `DTB magic` (from_be). Đặt trong platform vì `vifast-runtime` cấm
  `unsafe_code` hoàn toàn — bài học vị trí code.
- Kết quả: RISC-V `magic=0xd00dfeed` (đúng FDT, DTB ở `0x87e00000` do
  OpenSBI map sẵn, đọc được ngay). AArch64 `magic=0x0` ở `0x40233010` —
  vùng chưa map trong page table kernel (không crash, chỉ log sai); cần map
  vùng DTB trước khi đọc ở vòng sau.

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở giữ 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.

## 3. Việc tiếp theo (pha 4)

Map vùng DTB trên AArch64 (dùng page table hiện có), parse FDT tìm node
`framebuffer`/`virtio` lấy địa chỉ ramfb; RISC-V parse trước.
