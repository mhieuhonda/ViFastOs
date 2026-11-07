# ViFastOs — Báo cáo vòng 94 (NX trung thực + input không false-green)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `6e2302b`
**Bối cảnh:** audit #7.

## 1. NX: thêm `PD_HIGH` và làm lỗi thành ồn ào

- `PD_HIGH` (page table chứa leaf 2 MiB của LAPIC) chưa nằm trong danh sách NX
  → thêm, danh sách thành **20 vùng**.
- Trước đây nếu `split_identity_2m` hết slot: `NX_ENABLED = false` rồi
  `kernel_data_nx()` trả `true` vì `!NX_ENABLED` ⇒ **toàn bộ data kernel thành
  executable mà self-check vẫn xanh**. Nay:
  - thiếu slot ⇒ `klog!("NX FAIL: ...")` + `exit(1)`;
  - `kernel_data_nx()` chỉ chấp nhận trường hợp NX tắt khi CPU **không** hỗ trợ
    (`cpu_has_nx()` đọc CPUID).

## 2. Input: bỏ false-green và sửa đường ack

- `--input-check` với x86_64 trước đây `return True` (coi như đạt) dù x86 không
  có driver bàn phím nào. Nay in
  `--- x86_64 input SKIP (chưa có driver bàn phím: không virtio-input, IRQ1 bị mask) ---`.
- `input_key_ascii` nhận cả `value == 2` (auto-repeat) ⇒ giữ phím sinh nhiều ký
  tự; nay chỉ nhận `value == 1`.
- Client chỉ `irq_ack` trong nhánh `WouldBlock`; nếu byte đọc được ngay thì
  `pending` kẹt và phím sau bị coalesce. Nay ack ngay sau `console_read() == Ok`.

## 3. Kiểm chứng

- QEMU: riscv64 2/2, aarch64 2/2, x86_64 2/2.
- `--input-check`: aarch64 2/2, riscv64 1/1 (vẫn đòi cả `VIRTIO KEY 18` và echo
  `shell> e`).
- `gate.sh --with-tcp --with-input`: all steps passed.

## 4. Còn mở từ audit #7

- x86 `kernel_image_end()` trả 0 và không có linker script ⇒ check "KERNEL IMAGE
  OUTSIDE" không chạy; cần linker.ld + `kernel_wx_ok()` quét leaf W & !NX độc
  lập với danh sách.
- `gpu_echo_key` + marker `INPUT ECHO`; x86 cần IRQ1 (unmask bit 1, đọc 0x60)
  hoặc virtio-pci để có parity.
