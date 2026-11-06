# ViFastOs — Báo cáo vòng 41 (xác minh MMIO VirtIO, giữ nguyên pha 3)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `b5e227f`
**Mốc:** Xác minh giả thuyết treo MMIO, không đổi code.

## 1. Đã thử

- Gắn `virtio-net-device` vào QEMU riscv64/aarch64: net hiện ở slot mới
  (log `VIRTIO NET base=0x10007000 mac=52:54:...` một lần) nhưng boot treo
  sau `VIRTIO DEV` — revert về blk-only, gate xanh lại.
- Thêm probe MMIO trước `init_range`: log không xuất hiện dù binary có
  string — phát hiện serial test đọc (`/tmp/vifast-riscv64.serial` 07:32)
  khác với serial test thật ghi; cần xác minh đường log trước khi kết luận.

## 2. Kết luận

Giữ nguyên pha 3 ở mức API (vòng 39–40). Không đổi code vòng này để bảo
toàn gate xanh. Vòng sau cần: xác định đúng serial test, dùng QEMU monitor
`info qdm` thay vì đoán MMIO.

## 3. Số liệu cổng (giữ nguyên vòng 40)

- Host 203, QEMU 3/3×3, persist 7×3, harness 483, 0 warning.
