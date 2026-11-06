# ViFastOs — Báo cáo vòng 39 (VirtIO scan API, pha 3 bước 1)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `e1f305b`
**Mốc:** Pha 3 bước 1 — API liệt kê VirtIO device, chuẩn bị net.

## 1. Thay đổi

- `vifast-virtio`: thêm `DEVICE_NET=1`, `DEVICE_BLOCK=2` và
  `scan_devices(start, end, stride, visit)` duyệt MMIO window, gọi callback
  với `(base, device_id)` khi magic/version hợp lệ.
- Platform AArch64/RISC-V: gọi `scan_devices` trong `init_block` và `klog`
  `VIRTIO DEV base=.. device=..` cho mỗi slot (hiện chỉ blk device=2;
  net device=1 sẽ hiện khi QEMU thêm `-device virtio-net`).

## 2. Số liệu cổng

- Host 203, QEMU 3/3×3, persist 7×3, harness 483, 0 warning.
- `fmt --check` PASS; `check-docs.py` PASS.

## 3. Việc tiếp theo (pha 3)

Thêm `-device virtio-net-device` vào QEMU cmdline, đọc MAC qua config space
(0x100+), TX/RX split-queue tối thiểu (ARP reply trước, rồi ICMP/UDP).
