# ViFastOs — Báo cáo vòng 61 (khóa input task vào harness)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `9718dce`
**Mốc:** Đưa phần "input" của M4 vào gate tự động.

## 1. Thay đổi

- `vifast-client`: shell in `SHELL READY` khi vào vòng lặp input
  (`console_read` + UART IRQ notify) — chứng minh input task sống.
- `harness_spec.py`: `SHELL READY` vào `EXACT_ONE` toàn cục (shell chạy cả
  3 arch, không như net chỉ 2 arch). Fixture tự sinh từ EXACT_ONE nên không
  cần sửa tay; harness âm 483→**489** cases.
- Không đổi serial chardev (rủi ro gate) — gửi input thật qua socket để vòng
  sau khi có chardev hai chiều ổn định.

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở 8B), `fmt` PASS.
- QEMU 3/3×3 (serial thật có `SHELL READY`), persist 7×3, harness 489, docs.

## 3. Việc tiếp theo (pha 4 input)

Chardev socket hai chiều cho serial để test gửi `echo` thật và yêu cầu
output — input end-to-end.
