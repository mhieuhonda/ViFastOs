# ViFastOs — Báo cáo vòng 96 (x86 image range thật + hết flake DNS)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `3941eb0`
**Bối cảnh:** audit #8; soak 20 vòng trước đó bắt được 1 flake riscv64.

## 1. Flake soak: `VIRTIO NET DNS sent` (1/20 lần, riscv64)

- Nguyên nhân: `net_send_udp` trả `None` khi TX ring còn đang drain đúng thời
  điểm boot; demo DNS in `VIRTIO NET DNS FAIL` thay vì marker mà harness đòi.
- Sửa: thử lại tối đa 8 lần với spin ngắn trước khi báo FAIL (cả riscv64 và
  aarch64). Soak lại **12/12 cả ba kiến trúc** (trước đó 19/20).

## 2. x86: `kernel_image_end()` có giá trị thật

- Audit #8 chỉ ra `kernel_image_end()` trả 0 nên check `KERNEL IMAGE OUTSIDE`
  trong runtime **không bao giờ chạy** trên x86.
- Nay UEFI đọc `EFI_LOADED_IMAGE_PROTOCOL` (base + SizeOfImage), gọi
  `vifast_platform_x86_64::platform::set_image_range(base, size)`;
  `kernel_image_end()` trả `image_range().1`.
- Siết `check_loaded_image` từ **4 GiB** xuống **1 GiB** cho khớp identity map
  (audit: map chỉ phủ 1 GiB; thông điệp lỗi cũng đã ghi 1 GiB).

## 3. Vì sao không thêm linker script

Audit xác nhận kernel x86 dùng **lld-link/COFF**; `rust-lld -flavor link -T x.ld`
báo lỗi nên không thể dùng linker script kiểu aarch64/riscv. Hướng đúng là đọc
PE section table lúc boot (3 dải text hôm nay: 0x1000, 0x1CB000, 0x1CC000) rồi
NX toàn ảnh trừ text — để vòng sau cùng `kernel_wx_ok()`.

## 4. Kiểm chứng

- QEMU x86_64 2/2 (marker `KERNEL DATA NX OK` đúng 1 lần, không có
  `KERNEL IMAGE OUTSIDE`).
- aarch64 2/2, riscv64 2/2.
- Soak 12 vòng: 12/12 cả ba kiến trúc.
- `gate.sh --with-tcp --with-input`: all steps passed.
