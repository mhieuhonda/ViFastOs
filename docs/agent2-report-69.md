# ViFastOs — Báo cáo vòng 69 (demand-load trên cả 3 kiến trúc)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `ff37780`
**Mốc:** Demand-load chạy được cả x86_64 nhờ **retype frame ngay trong
server** thay vì root cấp frame trước.

## 1. Thiết kế mới (đơn giản hơn và tổng quát hơn)

Trước đây root phải retype sẵn một Frame rồi copy cap sang pager server — trên
x86_64 việc này thất bại vì cạn Untyped (page table 4 cấp).

Bây giờ pager server tự `retype(PAGER_SERVER_UNTYPED, Frame, FRAME4)` **đúng
lúc fault**, nạp backing image rồi map. Untyped con 2 trang này đã tồn tại trên
mọi arch (1 trang dùng cho evict, 1 trang cho demand-load) nên:
- Không cần root cấp frame ⇒ x86_64 chạy được.
- Đúng nghĩa "demand": object được tạo tại thời điểm fault.

## 2. Kết quả

- riscv64, aarch64, **x86_64** đều có `DEMAND LOAD OK` và fault
  `0x1001003000` đúng 1 lần.
- Harness âm 494 → 495: x86_64 bỏ loại trừ `0x1001003000`, thêm marker
  `DEMAND LOAD OK` vào `ARCH_EXACT_ONE` và fixture.

## 3. Kèm theo

- `scripts/soak.py`: chạy lặp gate QEMU N vòng, lưu serial/log lần hỏng vào
  `artifacts/soak/` (dùng cho đợt chạy 2 ngày cuối tuần).

## 4. Cổng

- QEMU 3/3×3, persist 7×3, harness 495, build 0 warning, `fmt` PASS.
- AArch64 static không đổi (khe 8 byte), RISC-V headroom 343360.
