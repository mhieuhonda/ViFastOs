# ViFastOs — Báo cáo vòng 40 (VirtIO scan API, hoãn log MMIO)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `11768eb`
**Mốc:** Pha 3 bước 1 — API scan xong, log hoãn do MMIO.

## 1. Thay đổi

- `vifast-virtio`: giữ `DEVICE_NET/BLOCK` + `scan_devices()` + `net_mac()`
  (API sạch, host test PASS).
- Platform: TẮT gọi scan trong `init_block` — đọc MMIO sau `init()` treo
  QEMU (timeout sau `VIRTIO DEV`; cả khi không gắn net). Nguyên nhân có thể
  là MMIO slot trống/vùng config cần DRIVER_OK; cần debug bằng monitor
  `info qdm` ở vòng sau.
- QEMU cmdline: revert về blk-only (gắn net cũng treo tương tự).

## 2. Bài học

Không đọc MMIO VirtIO sau khi device đã claim mà chưa hiểu rõ side-effect.
Vòng sau: probe net TRƯỚC khi init blk, hoặc dùng monitor xác nhận địa chỉ.

## 3. Số liệu cổng

- Host 203, QEMU 3/3×3, persist 7×3, harness 483, 0 warning.
- `fmt --check` PASS; `check-docs.py` PASS.
