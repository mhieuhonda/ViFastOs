# ViFastOs — Báo cáo vòng 86 (P0 notification + `cld` + gate trung thực)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `d376975`
**Bối cảnh:** audit #3 (`docs/audits/2026-11-07-round85.md`) tìm 1 P0 và 5 P1.

## 1. P0 — waiter notification bị ghi đè

- Trước: `wait_as` đặt `ntfn.waiter = Some(tcb)` không kiểm tra slot; hai task
  cùng wait → task đầu **BlockedNotification vĩnh viễn**. `signal` còn nuốt
  bit nếu waiter đã chết.
- Sau:
  - `Notification` có thêm `waiter2` (hai slot).
  - `wait_as`: điền `waiter` rồi `waiter2`; cả hai đầy → `WouldBlock` và
    **không** đổi state của caller (đây là nguyên nhân một bản thử trước làm
    hỏng boot x86: set `Ready` khi TCB đang Running).
  - `signal`: chỉ đánh thức waiter đang thật sự `BlockedNotification`; nếu
    không có ai, bit được tích vào `word`.
  - `detach_tcb`: gỡ waiter khỏi cả hai slot khi TCB bị destroy.
- Test: `notification_serves_two_waiters_and_rejects_a_third`,
  `signal_with_dead_waiter_keeps_the_bits`.
- **139 test kernel**.

## 2. P1 — DF của user sống vào ring 0 (x86)

- CPU không xoá DF khi đổi privilege; user set `std` rồi syscall → mọi
  `rep movs` trong copy path chạy lùi.
- Thêm `cld` ở 6 entry: `int80_entry`, `exc_common`, `pf_entry`, `irq0_entry`,
  `irq4_entry`, `ap_*_entry`.

## 3. P1 — gate trung thực

- `gate.sh` trước chỉ chạy `cargo test -- --ignored`; 139 test thường không
  nằm trong cổng. Nay có bước `kernel-test` (chạy toàn bộ) và
  `tests-present` (đếm ≥ 138 test liệt kê được, chặn trường hợp 2 test
  `#[ignore]` bị đổi tên).
- `--quick` không chạy QEMU/persist nữa và in rõ
  `all executed steps passed (--quick: QEMU and persistence were SKIPPED)`.

## 4. Xác nhận P1-3 (destroy TCB ↔ client BlockedReply)

- Audit ghi "needs confirmation"; test mới
  `destroying_a_server_wakes_a_client_blocked_in_reply` xác nhận
  `detach_tcb` đã gọi `wake_error(client, ObjectDestroyed)` — hành vi đúng,
  không cần sửa.

## 5. Chưa làm

- FP/SIMD context switch (rò rỉ q/xmm giữa task) — cần fxsave/fxrstor hoặc
  tắt EL0 FP; sẽ làm vòng sau cùng kiểm chứng memcpy của user softfloat.
- P2: `send_ipi` no-op trên x86/aarch64, TSS hard-code, readdir index lệch,
  soak coi (0,0) là PASS.

## 6. Cổng

- `cargo test -p vifast-kernel`: 139 passed.
- `gate.sh --with-tcp --with-input`: all steps passed.
