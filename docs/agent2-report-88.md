# ViFastOs — Báo cáo vòng 88 (P0 notification: hàng đợi waiter)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `1093544`
**Bối cảnh:** audit #4 tìm P0 thật trong chính bản vá vòng 86.

## 1. P0 — waiter bị bỏ mặc

- `wait_as` đặt `BlockedNotification` **trước** khi kiểm slot; cả hai slot đầy
  → `WouldBlock` trả về trong khi task vẫn Blocked và không nằm trong slot nào
  → task không bao giờ được chọn lại (treo + rò rỉ VSpace).
- Test cũ chỉ assert `Err`, không assert state → xanh giả.

## 2. Vì sao mọi bản sửa trước hỏng boot

Root cố ý park **4 worker** trên cùng notification slot 2 (không bao giờ được
signal): `vifast-root/src/main.rs:420,455,469,511`. Bất kỳ thiết kế nào trả
`WouldBlock` (hoặc chỉ 2 slot) đều làm các worker đó rơi vào `yield` loop,
đổi timing SMP → boot dừng ngay sau `SHELL READY` trên cả ba kiến trúc.
Đây là lý do các vòng 86/87 chỉ dám sửa phía `signal`.

## 3. Cách sửa

- `Notification.waiters: [Option<ObjRef>; 6]` (`NOTIFICATION_WAITERS = 6`).
- `wait_as`: giành slot **trước** khi đổi state; cùng tcb = refresh; hàng đợi
  đầy = `WouldBlock` với state nguyên vẹn.
- `signal`: đánh thức **tất cả** waiter đang `BlockedNotification`; không có ai
  thì tích bit vào `word`.
- `detach_tcb`, `destroy_notification`, `suspend_tcb`: gỡ TCB khỏi mọi slot.
- Test mới `notification_wakes_every_waiter_in_the_queue` (4 waiter), giữ
  `signal_skips_waiters_that_are_not_blocked`, `signal_with_dead_waiter_keeps_the_bits`,
  `destroying_a_server_wakes_a_client_blocked_in_reply`.

**140 test kernel.**

## 4. Cùng vòng

- **FS**: `read_dir_entry` giữ index thẳng hàng khi gặp entry hỏng
  (`[a,corrupt,c]` từng trả c cho cả index 1 và 2); test mới, FS **25 test**.
- **x86**: thêm IDT[0xFF] + stub `iretq` cho vector spurious (SVR trỏ 0xFF mà
  IDT chưa có entry → #NP).
- **IPI**: `Platform::send_ipi` giờ gửi thật (GIC SGIR trên aarch64, LAPIC ICR
  trên x86) thay vì forward vào stub rỗng.

## 5. Cổng

- `cargo test -p vifast-kernel`: 140 passed; `cargo test -p vifast-fs`: 25 passed.
- QEMU: riscv64 2/2, aarch64 2/2, x86_64 2/2; `test-persist.py` PASS.
- `gate.sh --with-tcp --with-input`: all steps passed.

## 6. Còn mở

P1 từ audit #4: PIC EOI của hart 1, `fault::suspend` không lưu frame, idle
race, `enqueue` im lặng, SGI 2 panic chưa enable, TSS hard-code.
