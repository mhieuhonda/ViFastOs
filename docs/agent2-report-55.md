# ViFastOs — Báo cáo vòng 55 (quét FDT treo, giữ header-only)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `c0dcc55`
**Mốc:** Thử parse struct block FDT; ghi nhận giới hạn, giữ gate xanh.

## 1. Đã thử và kết quả

- Đọc header OK: `FDT size=10319 struct_off=56` (RISC-V).
- Thêm vòng quét token tìm node `framebuffer` (bounded 256 token/4 KiB,
  giới hạn tên 64B + bounds check): treo guest, không log `fb_present`,
  rc=-9 timeout. Cả bản chưa và đã giới hạn đều treo — không phải vòng lặp
  vô hạn đơn thuần; có thể đọc struct block chạm vùng chưa map chắc chắn ở
  thời điểm `init_platform`, hoặc alignment/MMU side-effect.
- Revert về header-only (magic + size + struct_off): gate 3/3×3 xanh lại.
- Bài học pha 4: parse FDT cần làm sau khi MMU/page table ổn định hoàn toàn,
  hoặc copy DTB vào vùng nhớ đã map rõ ràng rồi parse.

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở 8B), `fmt` PASS.
- QEMU 3/3×3 (aarch64 có 1 lần flaky 2/3 ở lần chạy giữa, chạy lại 3/3),
  persist 7×3, harness 483, docs PASS.

## 3. Việc tiếp theo (pha 4)

Copy DTB (totalsize ~10 KiB) vào frame đã map trong root task rồi parse
ở user-space; hoặc parse sau `init_platform` khi page table đầy đủ.
