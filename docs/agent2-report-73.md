# ViFastOs — Báo cáo vòng 73 (boot splash: font 5x7 trên framebuffer)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `e992579`
**Mốc:** Display MVP có nội dung thật: dòng chữ **ViFastOs** vẽ bằng font
5x7 lên framebuffer, kiểm chứng bằng screendump.

## 1. Thay đổi

- `vifast-virtio`: thêm font 5x7 (`gpu_glyph`) cho các ký tự `V i F a s t O`,
  `gpu_fill_rect`, `gpu_draw_text(text, x, y, scale, color)`, và
  `gpu_gradient_pixel` (tách phần nền cũ).
- `gpu_draw_test_pattern()` giữ gradient nền, thêm hộp tối 0x101028 và dòng
  chữ `ViFastOs` màu trắng, scale 2 tại (10,10).
- `--gpu-check` kiểm thêm 2 pixel: trong hộp tối (7,7) = (16,16,40) và nét
  đầu chữ V (10,10) = (255,255,255); các pixel gradient cũ giữ nguyên.

## 2. Bằng chứng

- `--gpu-check` PASS trên riscv64 và aarch64: `gpu PASS (160x120 pixels
  verified)`.
- Ảnh framebuffer: `docs/assets/vifast-display-160x120.png` (PNG 640x480,
  phóng 4x từ screendump PPM) — hiển thị đúng "ViFastOs" trên nền gradient.

## 3. Cổng

- QEMU 3/3×3, persist 7×3, harness 501, build 0 warning, `fmt` PASS.
