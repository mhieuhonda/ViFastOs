# ViFastOs — Báo cáo vòng 66 (M4 display MVP: virtio-gpu, pixel thật)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `1fb31e3`
**Mốc:** **Display MVP hoàn thành** — guest tạo framebuffer 160×120 qua
virtio-gpu, vẽ gradient, và QEMU `screendump` xác nhận **đúng từng pixel**.

## 1. Driver virtio-gpu tối thiểu (`vifast-virtio`)

- `gpu_init(base)`: reset → ACK → DRIVER → FEATURES_OK → control queue
  (queue 0, 8 desc) → DRIVER_OK, rồi `GET_DISPLAY_INFO` để xác nhận kênh.
- `gpu_command(type, payload)`: 2 descriptor (cmd read-only, resp WRITE),
  kick queue 0, poll used ring có bound.
- `gpu_draw_test_pattern()`: RESOURCE_CREATE_2D (B8G8R8X8) →
  RESOURCE_ATTACH_BACKING (framebuffer static) → SET_SCANOUT →
  TRANSFER_TO_HOST_2D → RESOURCE_FLUSH.
- Đóng gói response: OK_NODATA 0x1100 / OK_DISPLAY_INFO 0x1101.

## 2. Kết quả (có bằng chứng pixel)

- riscv64: `VIRTIO GPU base=0x10006000 ver=2` → `READY` → `DRAW OK 160x120`.
- aarch64: `VIRTIO GPU base=0xa003a00 ver=2` → `READY` → `DRAW OK 160x120`.
- `--gpu-check` (opt-in): screendump PPM và so màu:
  - (0,0) = (0,0,64), (80,60) = (127,127,64), (159,119) = (253,252,64) —
    khớp công thức gradient `r=x*255/160`, `g=y*255/120`, `b=0x40`.
  - Cả hai arch: `gpu PASS (160x120 pixels verified)`.

## 3. Vấn đề gặp và cách xử lý

- Framebuffer 320×240 (307 KB) làm **vượt headroom kernel RISC-V**
  (`BELOW 192 KiB`) → build fail âm thầm. Giảm về 160×120 (76.8 KB), build
  xanh trở lại. Bài học: static lớn phải tính vào ngân sách headroom.
- fw_cfg/ramfb đã bị chặn từ vòng 65 → virtio-gpu là đường đúng.

## 4. Cổng

- Harness khóa `VIRTIO GPU READY` + `VIRTIO GPU DRAW OK` cho riscv64/aarch64.
- QEMU 3/3×3, persist 7×3, harness 489, host 207, build 0 warning,
  `fmt` PASS, docs PASS.
