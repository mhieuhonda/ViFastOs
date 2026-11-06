# ViFastOs — Báo cáo vòng 62 (input end-to-end, opt-in)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `8940720`
**Mốc:** Gõ lệnh thật vào shell guest qua QEMU và nhận output — đóng phần
"input" của pha 4.

## 1. Thay đổi

- `scripts/test-qemu.py`: thêm `run_qemu_interactive()` + `--interactive`
  (mặc định TẮT nên gate chuẩn không đổi): serial chuyển sang
  `-chardev socket` (vừa log ra file vừa cho test gửi phím), gửi `help`
  rồi `\r`, yêu cầu output `help time echo ls ...` — output khác chữ đã gõ
  nên chứng minh lệnh THỰC SỰ chạy, không chỉ echo ký tự.
- Áp dụng cho riscv64 + aarch64 (x86_64 UEFI/ESP để vòng sau).

## 2. Lỗi đã tìm và sửa (đáng ghi lại)

1. **Socket không được tạo**: TMPDIR macOS dài >104 ký tự — vượt giới hạn
   `AF_UNIX`, QEMU lặng lẽ không tạo socket. Sửa: đặt socket ở `/tmp` ngắn.
2. **`-serial` mồ côi**: bộ lọc chỉ bỏ giá trị `file:...` mà giữ cờ
   `-serial`, QEMU nuốt nhầm đối số kế tiếp. Sửa: bỏ cả cặp cờ+giá trị.
3. **`\n` bị mất trên PL011**: guest nhận và echo ký tự (`shell> echo ...`)
   nhưng dòng không bao giờ được xử lý. Sửa: gửi `help` rồi `\r` riêng,
   lặp tới 12 lần.

## 3. Kết quả

- `--interactive` riscv64 **4/4**, aarch64 **4/4** PASS (sau 3 lần sửa trên;
  trước đó flaky 1/3–2/3).
- Gate chuẩn (interactive off) không đổi: QEMU 3/3×3, persist 7×3,
  harness 489, host 203, 0 warning, `fmt` PASS, docs PASS.

## 4. Việc tiếp theo (pha 4)

Đưa `--interactive` vào gate chuẩn (hoặc job riêng) sau khi chạy ổn định
nhiều vòng; mở rộng x86_64 UEFI; sau đó quay lại net RX reply.
