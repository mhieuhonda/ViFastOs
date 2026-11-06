# ViFastOs — Báo cáo vòng 56 (header FDT đầy đủ, pha 4 bước 5)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `47261ec`
**Mốc:** Đọc trọn header FDT trong vùng đã kiểm chứng an toàn.

## 1. Thay đổi

- RISC-V `init_platform`: log thêm `strings_off` (off 12) + `version`
  (off 20) — cùng 7 word header đã đọc an toàn từ vòng 54, không chạm
  struct block (treo vòng 55).
- Kết quả: `FDT size=10319 struct_off=56 strings_off=5716 version=17` —
  FDT version 17 chuẩn, layout hợp lệ (struct 56→5716, strings sau đó).
  Đủ thông tin để vòng sau copy DTB (~10 KiB) vào vùng đã map rồi parse.

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.

## 3. Việc tiếp theo (pha 4)

Copy DTB vào frame đã map (root task) rồi parse struct/strings tìm node
framebuffer lấy địa chỉ ramfb.
