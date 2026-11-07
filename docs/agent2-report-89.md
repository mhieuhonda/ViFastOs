# ViFastOs — Báo cáo vòng 89 (P1 phần cứng: PIC EOI, SGI 2, idle race)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `4ddd0e9`
**Bối cảnh:** các P1 còn lại của audit #4 (audit #5 lỗi giữa chừng, sẽ chạy lại).

## 1. x86 — hart 1 ghi EOI PIC của BSP

- `ack_timer()` gọi `outb(0x20, 0x20)` và được gọi từ **cả** đường timer của BSP
  lẫn AP (AP: `lapic_eoi()` rồi `on_timer` → `ack_timer`). Hart 1 ghi EOI vào
  PIC của BSP làm xoá cờ in-service của BSP → mất tick PIT hoặc spurious IRQ7
  (vector 7 không có IDT entry ⇒ `#NP` ⇒ exit).
- Sửa: `ack_timer` chỉ EOI khi `hart_id() == 0`; AP đã EOI LAPIC trước đó.

## 2. aarch64 — SGI 2 (panic halt) chưa được enable

- `init_gic` chỉ bật `(1 << TIMER_IRQ) | 0b11` (SGI 0/1). `panic_halt_others`
  gửi **SGI 2** và `service_gic` chờ `irq == 2` ⇒ panic halt là dead code.
- Sửa: bật thêm bit 2. QEMU aarch64 2/2 PASS.

## 3. x86 — race cửa sổ idle (mất IPI)

- `ap_idle_loop` đọc queue, rồi mới set `AP_IDLE_WINDOW`; `wake_hart` chỉ gửi
  IPI khi thấy cờ đó. TCB được `make_ready` giữa hai bước sẽ không có IPI.
- Sửa: sau khi set cờ, kiểm lại `has_ready_on(1)`; nếu đã có việc thì xoá cờ
  và `continue` thay vì `hlt`.

## 4. `enqueue` bỏ rơi TCB — xét lại, không phải lỗi thực

- Audit #4 P1-6: `enqueue` `return` im lặng khi không có hart nào còn chỗ.
- Kiểm tra hằng số: mỗi runqueue chứa `MAX_TCBS = 18` = tổng số TCB, và
  `queued` chặn xếp trùng, nên mọi TCB luôn tìm được hàng. Đã ghi chú tại chỗ
  thay vì thêm nhánh không thể kiểm thử.

## 5. Cổng

- Kernel **141 test**; QEMU: x86_64 2/2, aarch64 2/2 (sau từng bản vá).
- `gate.sh --with-tcp --with-input`: all steps passed.

## 6. Còn mở (P1 audit #4)

`fault::suspend` không lưu trap frame (x86 truyền `TrapFrame::empty()`), TSS
hard-code `[1]` khi `MAX_HARTS = 4`, và các P2 về soak/`--quick` docs.
