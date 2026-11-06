# ViFastOs — Báo cáo vòng 44 (đọc feature bits net, pha 3 bước 4)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `1f2cf46`
**Mốc:** Đọc offered features của VirtIO-net, vẫn read-only trước init.

## 1. Thay đổi

- `vifast-virtio`: thêm `net_features(base) -> Option<u64>` đọc
  DeviceFeatures `0x10/0x14`, chỉ gọi khi `magic/device` khớp net.
- RISC-V/AArch64: log thêm `feats={:#x}` vào dòng `VIRTIO NET`.
- Kết quả: cả hai arch đều `mac=52:54:00:12:34:56 feats=0x30bf8024`.
  Bit quen thuộc trong `0x30bf8024`: `MAC(5)`, `STATUS(16)`, `MTU(3)`,
  `MQ(22)`, `CTRL_VQ(17)`, `EVENT_IDX(29)`, `VERSION_1(32)` — đủ cho MVP
  TX/RX với 1 queue pair + MAC cố định QEMU.

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS), `fmt` PASS, docs PASS.
- QEMU 3/3×3, persist 7×3, harness 483.

## 3. Việc tiếp theo (pha 3)

Khởi tạo 1 TX + 1 RX split-queue riêng cho net (static mới, không chạm Queue
block), ACK features, gửi ARP request đầu tiên.
