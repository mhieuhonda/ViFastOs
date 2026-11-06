# ViFastOs — Báo cáo vòng 70 (virtio-input: bàn phím vào shell)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `827a9a7`
**Mốc:** M4 input có driver thiết bị thật — bàn phím virtio đưa ký tự vào shell
hiện có mà không phải sửa client.

## 1. Driver virtio-input (`vifast-virtio`)

- Device id 18; `input_init(base)`: reset/ACK/DRIVER/FEATURES_OK (VERSION_1),
  event queue (0) 16 buffer 8 byte đăng sẵn + **kick**, status queue (1) attach
  đúng chuẩn (không kick), DRIVER_OK.
- `input_poll()`: đọc used ring, giải mã `virtio_input_event` (le16 type,
  le16 code, le32 value), **re-post descriptor + kick** sau mỗi event (đúng
  bài học từ net: QueueReady=1, kick sau khi đăng buffer, re-post sau khi dùng).
- `input_key_ascii()`: map mã phím Linux → ASCII (chữ, số, Enter, Backspace,
  Space, Shift), có unit test.

## 2. Tích hợp platform

- riscv64/aarch64: scan thấy device 18 → `input_init`, log
  `VIRTIO INPUT READY base=…`.
- Timer IRQ hart 0 gọi `poll_virtio_input()`: mỗi phím nhấn log
  `VIRTIO KEY <code>` (tối đa 16, bounded), map ASCII và đẩy vào RxQueue UART
  của arch + `irq_raise(IRQ_UART)` — shell hiện có đọc như gõ phím thật.
- Không log trong IRQ ngoài dòng bounded ở trên; không đổi client/ABI.

## 3. Kiểm chứng

- QEMU thêm `-device virtio-keyboard-device` (riscv/aarch64).
- `--input-check`: gửi `sendkey e 20` qua HMP monitor, yêu cầu
  `VIRTIO KEY 18` trong serial — **PASS 2/2 trên cả hai arch**
  (riscv64 input base 0x10005000; aarch64 0xa003800 khi chạy riêng).
- Harness khóa `VIRTIO INPUT READY` (riscv64/aarch64), cấm
  `VIRTIO INPUT INIT FAIL`; fixture cập nhật; 494 → **498 cases**.

## 4. Ổn định

- Thêm thiết bị làm lộ 2 flake cũ: TX net và lệnh GPU có thể trượt poll khi
  QEMU bận (slirp DNS). Đã tăng bound TX 2M→6M, GPU 4M→10M và **đảo thứ tự
  khởi tạo GPU trước net** để tránh tranh chấp; sau đó QEMU 3/3×3 liên tục.

## 5. Cổng

- QEMU 3/3×3, persist 7×3, harness 498, build 0 warning, `fmt` PASS.
