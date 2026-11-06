# ViFastOs — Báo cáo vòng 75 (virtio-rng: entropy source)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `e992579` (sau vòng 74 gate script)
**Mốc:** Thêm driver entropy: guest đọc được byte ngẫu nhiên thật từ QEMU.

## 1. Thay đổi

- `vifast-virtio`: `DEVICE_RNG=4`, `rng_init(base)` (reset/ACK/DRIVER/
  FEATURES_OK với VERSION_1, queue 0 pad 64 byte ghi được, QueueReady=1,
  DRIVER_OK, kick) và `rng_read(buf)` (đăng buffer, kick, poll used, copy ra
  ngoài; tối đa 8 byte mỗi lần).
- Platform riscv64/aarch64: scan device 4 → init → đọc 2 lần 8 byte, yêu cầu
  hai lần khác nhau → `VIRTIO RNG OK <4 byte hex>`, sau đó
  `VIRTIO RNG READY base=…`; nếu trùng/đọc hụt → `VIRTIO RNG BAD`.
- QEMU: thêm `-device virtio-rng-device` cho riscv64/aarch64.
- Harness: `VIRTIO RNG READY` exact-one 2 arch, `VIRTIO RNG BAD` bị cấm;
  501 → **504 cases**.

## 2. Bằng chứng

- riscv64: `VIRTIO RNG OK 30c58cd7`, `VIRTIO RNG READY base=0x10004000`.
- aarch64: `VIRTIO RNG OK daf9ae30`, `VIRTIO RNG READY base=0xa003600`.
- Hai lần đọc khác nhau chứng minh nguồn thật (không phải hằng số).

## 3. Cổng

- `scripts/gate.sh`: build, QEMU 3/3×3, persist 7×3, harness 504, fmt, docs —
  **all steps passed**.
