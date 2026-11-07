# ViFastOs — Báo cáo vòng 85 (x86_64: quét PCI config space)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `996f126`
**Bối cảnh:** bước tiến parity x86_64 — x86 chưa có VirtIO (dùng ATA PIO), và
trước khi nghĩ tới virtio-pci thì phải chứng minh truy cập PCI config space.

## 1. Thay đổi

- Thêm `outl`/`inl` và `pci_read32(bus, slot, func, offset)` theo cơ chế
  0xCF8/0xCFC.
- `pci_scan()` quét bus 0 (32 slot, function 0), đếm thiết bị có vendor hợp lệ
  và ghi lại thiết bị đầu tiên.
- `init_block()` của x86_64 log:
  `PCI SCAN OK devices=N first=VVVV:DDDD class=CCCC`.
- Harness: `PCI SCAN OK` là marker bắt buộc của x86_64; fixture cập nhật
  (507 case).

## 2. Kết quả

- QEMU x86_64 (OVMF/Q35): `PCI SCAN OK devices=4 first=8086:29c0 class=0600`
  — đúng host bridge Intel Q35, chứng minh đọc được config space thật.
- `gate.sh`: all steps passed (QEMU 3/3 × 3, persist, harness, fuzz, fmt, docs).

## 3. Vì sao quan trọng

- Đây là mảnh còn thiếu để sau này gắn **virtio-pci** (net/gpu) trên x86_64 —
  hiện virtio chỉ có trên riscv64/aarch64 qua virtio-mmio.
- Bước tiếp theo khả thi: đọc BAR0 của một thiết bị virtio-pci giả lập (thêm
  `-device virtio-net-pci` vào cmdline x86) và so sánh vendor/device id.

## 4. Cổng

- QEMU 3/3×3, persist 7×3, harness 507, fuzzer kernel, fmt, docs — xanh.
