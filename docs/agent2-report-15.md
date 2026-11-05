# ViFastOs — Báo cáo Agent 2, vòng 15

**Ngày:** 2026-11-05  
**Baseline:** `03cb048` (code), bundle vòng 14  
**Code commit:** `58016c8`

## 1. Bảng bước

| Bước | Trạng thái | Ghi chú |
|---|---|---|
| 1. IRR diag | ✅ có log | `irr_after_eoi=0, isr=0`, khác kỳ vọng D3 |
| 2. TSC/PIT + LAPIC timer + D4 idle | ✅ | 50/50 x86, rate/ipi count có cận |
| 3. BKL + `pick_on(1)` trên AP | ✅ | 50/50, không STALL |
| 4. Worker AP + `sched_harts=0b11` | ✅ | 50/50, `SMP WORKERS OK`, `SMP POST IDLE OK` |
| 5. Stage trampoline + feature `test-ap-stall` | ✅ | stage 1..4; stall dừng ở stage 2 |
| 6. Docs / R6 / R7 | ✅ | order được sinh; D3 hủy, D4 thêm |

Task 2–5 nằm trong một commit code tích hợp `58016c8` vì cùng sửa
`vifast-platform-x86_64`; báo cáo ghi rõ thay vì tách commit giả.

## 2. Test cuối

```text
build.sh                          PASS, không warning
vifast-kernel                     88/88
vifast-arch                        5/5
vifast-pte                        11/11
vifast-elf                        17/17
vifast-acpi                        8/8
QEMU --repeat 10                  10/10 cả 3 kiến trúc
x86_64 --repeat 50                50/50
RISC-V boot-handoff --repeat 10   10/10
test-harness-spec.py              201 case âm PASS
test-persist.py                   3/3 PASS
cargo fmt --check                 PASS
check-docs.py                     PASS
```

RISC-V headroom: `454192` byte, `kernel_end=0x803911d0`.

## 3. Task 1 — xác nhận R1/D3

AP đọc IRR vector 0x31 **trước** EOI = 1, sau EOI = 0, ISR = 0. Log nguyên văn:

```text
IRR DIAG (irr_after_eoi=0, isr=0)
IRR BEFORE=1
```

Kỳ vọng D3 là `irr_after_eoi=1, isr=0`; QEMU trả 0. Vì vậy D3 bị hủy và
Task 2 dùng D4. Ghi nhận khác biệt thay vì tiếp tục biện minh cho poll IRR.
Log này lấy từ build chẩn đoán trước khi gỡ; code cuối không còn poll IRR.

## 4. Task 2 — hiệu chuẩn và D4

- TSC/PIT channel 2 một lần lúc boot; QEMU 10 lần: `TSC HZ` khoảng
  `1.000.084.913` (mẫu cuối), sai số ~0.01%.
- LAPIC timer one-shot đối chiếu TSC, đặt periodic ~100 Hz:
  `LAPIC TIMER HZ` khoảng `1.000.784.762` (mẫu cuối).
- AP không còn poll IRR để xử lý interrupt. Vector 0x30/0x31 có entry IDT
  ring 0; handler assert `in_idle_window`, EOI, `iretq`. Ring 3 giữ BKL +
  preempt.
- Idle AP: `sti; hlt; cli`; `Wt <= T`, `Ws <= Y`, `I >= 1`.

10 lần x86 (T, B, X, Y, Wt, Ws, I):

```text
T  = 16,18,18,18,17,17,18,18,18,18
B  = 20,20,20,20,20,20,20,20,20,20
X  = 3,3,3,3,3,3,3,3,3,3
Y  = 3,3,3,3,3,3,3,3,3,3
Wt = 16,18,18,18,17,17,18,18,18,18
Ws = 3,3,3,3,3,3,3,3,3,3
I  = 20,22,21,21,20,20,21,22,21,22
```

Harness mới chặn số rác: `B/2 <= T <= 2B`, `B >= 5`, `1 <= Y <= X`,
`Wt <= T`, `Ws <= Y`, cộng các case âm `T=1000*B`, `Y=0`, `Y>X`,
`Wt>T`, `Ws>Y`.

## 5. Task 3 — BKL và scheduler AP

Tái hiện được panic khi thêm BKL/`pick_on(1)`:

```text
PANIC
PANIC LOC crates/vifast-platform-x86_64/src/platform.rs:2031:5
```

Nguyên nhân trực tiếp: các assert `!BKL.is_held()` sau syscall/IRQ dùng
trạng thái **toàn cục**, nên sai khi AP đang có ticket chờ; AP giữ/đợi BKL là
hợp lệ. Đồng thời `assert_current_tcb()` so `current()` hart 0 trên AP; sửa
thành `current_on(hart)`. Sau đó AP lấy BKL, `pick_on(1)` (None khi
`sched_harts=0b01`), nhả BKL rồi vào cửa sổ D4. Kết quả 50/50, không STALL.

Ngoài ra Task 2 phát hiện một lỗi nền quan trọng: trampoline đặt stack AP tại
`TRAP_STACKS + 0x4000`, chồng stack CPU0; sửa thành `TRAP_STACKS[1] + 0x4000`.
Đây là nguyên nhân các lần BSP page-fault vào địa chỉ stack trước đó.

## 6. Task 4 — worker trên AP

- `sched_harts` x86 = `0b11`; AP chạy worker A/B affinity `0b10`.
- Timer LAPIC ở ring 3 preempt; `enter_user` AP dùng CR3/GS/TR riêng.
- `SMP WORKERS OK (a=50, b=50)`, `SMP SCHED OK`, `SMP POST IDLE OK` đều có.

10 lần x86 (N, M, S, W, I, Tp, Ip, Wtp):

```text
N  = 303856,330810,322587,364791,334747,266413,305337,306902,311713,366094
M  = 104,104,104,104,104,104,104,104,104,104
S  = 2,2,2,2,2,2,2,2,2,2
W  = 3,3,3,3,3,3,3,3,3,3
I  = 20,22,21,21,20,20,21,22,21,22
Tp = 16,18,18,18,17,17,18,18,18,18
Ip = 17,19,18,18,17,17,18,19,18,19
Wtp= 16,18,18,18,17,17,18,18,18,18
```

## 7. Task 5 — stage trampoline

Trampoline ghi stage 1 (real), 2 (protected), 3 (long trong `ap_entry`),
4 (online). BSP chờ stage và in timeout `SMP FAIL (ap timeout stage=N)`.
Build feature `test-ap-stall`, chạy một lần:

```text
AP TRAMPOLINE OK (page=0x9e000)
AP REAL MODE OK (apic=1)
SMP FAIL (ap timeout stage=2)
```

QEMU exit code 7 (isa-debug-exit), không treo. Build feature không warning;
sau đó rebuild normal.

## 8. Task 6 — docs

- R6: `harness_spec.py` trước đây không có điều kiện thứ tự. Đã thêm `ORDER`
  x86 (`AP REAL < AP LONG < SMP OK`), case âm đảo thứ tự, generator in và
  `check-docs.py` đếm cả order.
- R7: sửa mục 2/8/9/13/16; gộp ý x86; thêm D4; đánh dấu D3 đã hủy kèm lý do.
- Thêm chuỗi cấm prose cũ; `check-docs.py` PASS.

## 9. Nợ

- IRQ0 legacy broadcast tới AP: AP ack PIC rồi bỏ qua; chưa che hoàn toàn
  nguồn PIC ở AP.
- AP timer hiện 100 Hz, chưa có deadline/TSC-deadline; chưa x2APIC.
- Chưa có per-hart runqueue, khóa mịn, per-VA TLB shootdown; panic chưa dừng
  toàn hệ thống.
- `CapDelete` vẫn placeholder; FS v2, IOMMU phần cứng, net/input, compositor
  chưa làm.
