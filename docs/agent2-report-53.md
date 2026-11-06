# ViFastOs — Báo cáo vòng 53 (ghi selector fw_cfg treo, revert an toàn)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `d1662b9`
**Mốc:** Thử đọc danh mục fw_cfg; ghi nhận giới hạn, giữ gate xanh.

## 1. Đã thử và kết quả

- Thêm ghi selector u16 `0x19` (file dir) ở `fw_base+8` rồi đọc data ở `+0`:
  RISC-V treo sau `ROOT ELF entry` (rc=-9 timeout 20s), ARM chưa kiểm tra
  nhưng revert cả 2 arch cho đồng nhất.
- Phát hiện: lần đọc sig thứ 2 trở đi trả về 0x0 — fw_cfg MMIO không có
  thanh ghi signature cố định; đọc offset 0 trả DATA của selector hiện tại.
  Ghi selector có side-effect (có thể cần DMA hoặc thứ tự khác) — treo guest.
- Revert về sig-only (vòng 52 đã xanh): gate 3/3×3 xanh lại ngay.
- Bài học pha 4: không ghi MMIO fw_cfg khi chưa hiểu rõ sequencing; vòng sau
  nghiên cứu fw_cfg DMA access hoặc dùng device tree để lấy framebuffer.

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS), `fmt` PASS, docs PASS.
- QEMU 3/3×3, persist 7×3, harness 483.

## 3. Việc tiếp theo (pha 4)

Nghiên cứu fw_cfg DMA (QEMU docs) hoặc parse device tree `chosen/framebuffer`
để lấy địa chỉ ramfb — đọc DTB an toàn hơn ghi MMIO.
