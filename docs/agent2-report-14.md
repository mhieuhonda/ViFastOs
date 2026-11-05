# ViFastOs — Báo cáo Agent 2, vòng 14

**Ngày:** 2026-11-05  
**Baseline:** `03e8c1d` (code), bundle vòng 13

## 1. Bảng bước

| Bước | Trạng thái | Commit |
|---|---|---|
| 1. AP bring-up chắc: log order, TSS/GS, IPI thật | ✅ phần lớn | `6ed6cde` |
| 2. AP LAPIC timer + idle IRR polling | ✅ | `03cb048` |
| 3. Worker/scheduler trên AP | ❌ chưa làm | — |
| 4. Đo BKL | ❌ chưa làm | — |
| 5. Docs R6 + crate coverage | ✅ | docs vòng 14 |

R4 (TSC hiệu chuẩn PIT) và R5 (stage byte đầy đủ) **chưa làm**; xem nợ.
Task 3 chưa làm nên `sched_harts` x86 vẫn `0b01`.

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
test-harness-spec.py              178 case âm PASS
test-persist.py                   3/3 PASS
cargo fmt --check                 PASS
check-docs.py                     PASS
```

Headroom RISC-V: `454192` byte (`kernel_end=0x803911d0`).

## 3. Task 1 — AP bring-up

- **R1:** một chuỗi INIT-SIPI-SIPI; AP không tự in long mode. BSP in
  `AP REAL MODE OK`, rồi `AP LONG MODE OK`, rồi `SMP OK`; log 50 lần đúng thứ tự.
- **R2:** BSP ghi mailbox + gửi LAPIC IPI vector `0x30`; AP poll IRR, xử lý
  mailbox, EOI, ACK. `ipi=1` chỉ in khi `AP_IRR_SEEN > 0`. Đây là IPI LAPIC
  thật, không còn chỉ mailbox.
- **R3:** AP có GDT riêng và TSS riêng (`TSSS[1]`), `RSP0` = trap stack CPU1,
  IST1 riêng; `ltr` selector `0x28`; assert `str == 0x28` và
  `IA32_GS_BASE == &CPU_STATES[1]`. AP dùng GS/swapgs convention.
- **R4:** chưa hiệu chuẩn TSC bằng PIT; delay INIT/SIPI vẫn dùng vòng TSC xấp xỉ.
- **R5:** chưa thêm byte stage đầy đủ vào trampoline.

Log:

```text
AP TRAMPOLINE OK (page=0x9e000)
LAPIC OK (id=0)
AP REAL MODE OK (apic=1)
AP LONG MODE OK (cpu=1)
SMP OK (harts=2, ipi=1)
```

## 4. Task 2 — LAPIC timer/idle

AP cấu hình LAPIC timer periodic vector `0x31`, đọc IRR khi IF=0, EOI và
tăng counter. AP loop không dùng `hlt`; dùng `pause`.

10 lần x86 (T, I, Wt, Ws):

```text
T  = 105911, 180601, 168640, 143679, 154085,
     150357, 158281, 153449, 186297, 124774
I  = 132613, 212776, 192191, 173869, 178502,
     179162, 183081, 177231, 208219, 151208
Wt = 105894, 180580, 168619, 143657, 154061,
     150338, 158256, 153427, 186277, 124751
Ws = 130481, 210378, 189935, 171460, 175088,
     176923, 180986, 174430, 205253, 148722
```

`--arch x86_64 --repeat 50`: **50/50**. AP chưa gọi `pick_on(1)` trong vòng
idle vì lần thử đưa BKL/scheduler vào AP gây panic; Task 3 chưa làm.

## 5. Task 5 — Docs

- Sửa R6: bỏ dòng `crates/vifast-acpi/src/lib.rs` lạc trong mục 4; thêm
  `vifast-acpi` vào mục 19; thêm `cargo test -p vifast-acpi` vào mục 14.
- Sửa mục 9/18 #11/#25 về `PDPT[0]/PDPT[3]` x86 và `L1[0..1]` AArch64.
- Thêm D3, vòng 14; cập nhật mục 13/16/17/20.
- `check-docs.py` kiểm tra mọi package có host test (`PACKAGES`) có trong mục 19
  và có lệnh test trong mục 14. Lần chạy đầu thiếu ACPI nên fail; đã bổ sung.

## 6. Nợ

- Task 3: x86 AP scheduler/worker, `enter_user` AP, timer IRQ user, reschedule.
- Task 2 chưa gọi `pick_on(1)` trên AP; BKL/AP scheduler path cần debug.
- TSC calibration bằng PIT và stage byte trampoline.
- Vector IDT IPI khi AP ở ring3 chưa cài.
- `CapDelete` placeholder, per-hart runqueue, khóa mịn, per-VA TLB, FS v2,
  IOMMU phần cứng, net/input, compositor.
