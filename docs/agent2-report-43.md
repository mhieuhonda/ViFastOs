# ViFastOs — Báo cáo vòng 43 (log MAC net cả 2 arch, pha 3 bước 3)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `14a6c3a`
**Mốc:** Đọc MAC VirtIO-net qua config space trước init.

## 1. Thay đổi

- RISC-V: trong probe trước init, khi `device == DEVICE_NET` gọi
  `net_mac(base)` đọc 6 byte config `0x100` và `klog` `VIRTIO NET`.
- AArch64: tương tự trong quét 32 slot.
- Kết quả: cả hai arch đều log `VIRTIO NET base=.. mac=52:54:00:12:34:56`
  (MAC mặc định QEMU user-net). Xác minh `net_mac` (vòng 40) hoạt động.

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS), `fmt` PASS, docs PASS.
- QEMU 3/3×3, persist 7×3, harness 483.

## 3. Việc tiếp theo (pha 3)

Khởi tạo hàng đợi net (TX/RX split-queue riêng, không dùng chung Queue
block), đàm phán features `MAC + STATUS`, gửi ARP request đầu tiên.
