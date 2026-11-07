# ViFastOs — Báo cáo vòng 95 (màn hình tương tác: phím vẽ lên framebuffer)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `35a2bc8`
**Bối cảnh:** mốc "virtio-input/display tương tác" của mục tiêu cuối tuần; audit #7
cũng đề xuất `gpu_echo_key` + marker.

## 1. Trước vòng này

Phím đi được: device → driver → kernel IRQ → notification → `console_read()` →
shell echo (`shell> e`). Nhưng **không có gì cập nhật màn hình** sau phím:
`gpu_draw_test_pattern` chỉ chạy một lần lúc boot, `gpu_draw_text` là private,
font chỉ có glyph cho "ViFastOs".

## 2. Đã thêm

- **Font 5x7 cho chữ số `0..9` và `K`** (11 glyph) để hiển thị mã phím.
- `gpu_flush_region(x, y, w, h)`: TRANSFER_TO_HOST_2D + RESOURCE_FLUSH cho **một
  vùng nhỏ** thay vì cả khung hình.
- `pub fn gpu_echo_key(code: u16) -> bool`: tô nền tối 78x18 tại (6, 34), vẽ
  `K<code>` (thập phân), rồi flush đúng vùng đó. Gọi được trong IRQ vì
  `gpu_command` là poll đồng bộ có bound 10M vòng.
- Hook vào IRQ handler của virtio-input trên **aarch64** và **riscv64**: sau khi
  stash byte, gọi `gpu_echo_key(event.code)` và log `INPUT ECHO <code>`
  (giới hạn 4 phím đầu để bound công việc trong ngắt).

## 3. Kiểm chứng tự động

`--input-check` trước đây chỉ đòi `VIRTIO KEY 18` + `shell> e`. Nay đòi thêm
**`INPUT ECHO 18`** — tức phím phải thực sự vẽ lên framebuffer:

| Kiến trúc | Kết quả |
|---|---|
| aarch64 | 2/2 PASS (có `INPUT ECHO` trong serial) |
| riscv64 | 2/2 PASS |
| x86_64 | SKIP (không có driver bàn phím, in rõ lý do) |

`input_check` cũng được thêm `-device virtio-gpu-device` (trước đó chỉ có
keyboard nên `GPU_READY=false` và echo không thể vẽ).

## 4. Cổng

- `gate.sh --with-tcp --with-input`: all steps passed.
- QEMU chuẩn 3/3 cả ba kiến trúc; kernel 141 test, FS 25, harness 508.

## 5. Còn lại

- x86 parity cho input: cần IRQ1 (unmask bit 1 của PIC, handler đọc port 0x60)
  hoặc virtio-pci; x86 cũng chưa có GPU driver (virtio-pci).
- Audit #8 (linker script x86 + `kernel_wx_ok`) đang chạy — kết quả sang vòng sau.
