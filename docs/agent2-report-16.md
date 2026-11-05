# ViFastOs — Báo cáo Agent 2, vòng 16

**Ngày:** 2026-11-05  
**Baseline:** `58016c8` (code), bundle vòng 15  
**Code commit:** `34db2e0` (nhóm chính `7196a4e`, thêm fix panic-halt/console)

## 1. Bảng task

| Task | Kết quả | Commit / ghi chú |
|---|---|---|
| 1. BKL owner + assert theo hart | ✅ | `7196a4e`; host test 6/6 |
| 2. Mask PIC/LINT trên AP | ✅ | `7196a4e`; x86 50/50 |
| 3. Preempt worker trên CPU phụ | ✅ | `7196a4e`; 50/50 cả 3 |
| 4. Panic dừng toàn hệ thống | ✅ feature | `225ccdb`,`34db2e0`; `test-panic-hart1` 5/5 cả 3 |
| 5. BKL stats | ✅ | line ở test-exit, không assertion |
| 6. Docs R1/R7 | ✅ | bundle/commit lấy từ git log |

Task 1–5 nằm commit chính `7196a4e`, Task 4 thêm fix `225ccdb`/`34db2e0` vì cùng sửa file platform lớn; test
riêng từng task được ghi dưới đây (đúng quy tắc mục 3 của quyết định).

## 2. Kết quả test

```text
build.sh                          PASS, không warning
vifast-kernel                     88/88
vifast-arch                        6/6
vifast-pte                        11/11
vifast-elf                        17/17
vifast-acpi                        8/8
QEMU --repeat 10                  10/10 cả 3 kiến trúc
x86_64 --repeat 50                50/50
riscv64 --repeat 50               50/50
aarch64 --repeat 50               50/50
RISC-V boot-handoff --repeat 10   10/10
test-panic-hart1 --repeat 5       5/5 cả 3
test-harness-spec.py              237 case âm PASS
test-persist.py                   3/3 PASS
cargo fmt --check                 PASS
check-docs.py                     PASS
```

RISC-V headroom: `454192` byte, `kernel_end=0x803911d0`.

## 3. Task 1 — BKL theo hart

`BigKernelLock` có `owner`, `held_by(hart)`, `lock_with_hart(hart, poll)`.
Trước khi lấy khóa assert `!held_by(current_hart)`; guard xóa owner trước khi
tăng `serving`. Các vị trí từng dùng trạng thái toàn cục:

- x86 `enter_user` (platform.rs:1940) — thêm `assert_bkl_released()`.
- x86 trap (2234), IRQ0 (2260), AP user IRQ (2314), page-fault/exception
  (2285/2292) — assert `!BKL.held_by(current_hart)`.
- RISC-V/AArch64 `bkl_lock()` và `assert_bkl_released()` chuyển từ
  `BKL_OWNER`/`is_held()` toàn cục sang `held_by(hart)`.

Host test mới: hai thread, thread A giữ hart 0, thread B chờ hart 1;
`held_by(A)=true`, `held_by(B)=false`, sau khi nhả thì ngược lại. Kết quả
`cargo test -p vifast-arch`: 6/6.

## 4. Task 2 — PIC trên AP

Giá trị LVT đọc được:

- BSP: `LINT0=0x700` (ExtINT), `LINT1=0x400` (NMI, giữ nguyên).
- AP: `LINT0=0x10000` (masked), `LINT1=0x10000` (masked).

Đường AP ack PIC đã bị xóa; nếu AP vẫn nhận vector PIC thì in `AP PIC IRQ` và
panic. Harness cấm chuỗi này. 10 lần Task 2 (trước worker c) có `B=20` lần
lượt; sau khi thêm worker c, dòng final là `B=14..16` vì `kernel.ticks` toàn
cục cũng tăng bởi timer AP; harness dùng cận `B/2 <= T <= 2B`.

## 5. Task 3 — preempt thật

10 lần đầu mỗi kiến trúc (T/U/P/S/W):

```text
RISC-V T=15,15,15,15,15,15,15,14,15,15
        U=6,6,5,6,6,6,5,5,5,5
        P=6,6,5,6,6,6,5,5,5,5
        S=1,1,1,1,1,1,1,1,1,1
        W=1,1,1,1,1,1,1,1,1,1
AArch64 T=15 x10
        U=5,5,6,5,5,5,5,5,5,5
        P=5,5,6,5,5,5,5,5,5,5
        S=1,1,1,2,1,1,1,1,1,1
        W=1,1,1,1,1,1,1,1,1,1
x86_64  T=12,11,13,12,13,13,13,13,12,13
        U=5,5,5,6,5,5,5,5,5,5
        P=5,5,5,6,5,5,5,5,5,5
        S=1,1,1,2,1,1,1,1,1,1
        W=1,1,1,2,1,1,1,1,1,1
```

Worker c đọc `GetTime` tới `start+10` tick (ít nhất 5 tick theo yêu cầu), không
Yield; timer phải cắt nó để a/b chạy. `SMP PREEMPT OK` đúng một lần, `U>=3`,
`P>=1`, `U<=T`; `W<=S` được kiểm tra. `SMP WORKERS OK` đổi thành
`(a=50, b=50, c=1)`.

## 6. Task 4 — panic-halt

Feature `test-panic-hart1` panic có chủ đích trên hart 1 trong timer preempt.
Mỗi kiến trúc 5/5; log một lần:

```text
PANIC (hart=1)
```

Không còn dòng `ROOT`/`SMP`/`USER FAULT` sau panic. Cơ chế: atomic
`vifast_arch::panic::begin()` (hart đầu thắng), IPI halt (RISC SBI, AArch SGI 2,
x86 LAPIC vector 0x32), hart nhận `cli; hlt`/`wfi` vĩnh viễn. Hart thứ hai panic
im lặng. QEMU exit: x86 mã 7; RISC-V/AArch dùng SBI/PSCI shutdown trả mã 0 nên
không phân biệt được mã lỗi — đây là giới hạn nền tảng, không phải test bị bỏ.

## 7. Task 5 — BKL stats

Ba lần mỗi kiến trúc, định dạng `(h0_acq, h0_spin, h1_acq, h1_spin)`:

```text
RISC-V  (592123,96831,154226,1608965) (705442,31692,92941,384235) (190141,10745,60769,488500)
AArch64 (751949,12448,195339,2083257) (441690,39277,48756,392194) (189140,10454,52031,426678)
x86_64  (543969,11259,168324,1781545) (630859,35305,90915,417655) (159274,11073,63211,527585)
```

Không assertion; số lớn chủ yếu do worker c gọi `GetTime` liên tục.

## 8. Docs / nợ

- R1: mục 18 ghi IRR DIAG vòng 15 **không kết luận** do mâu thuẫn SDM/số liệu
  vòng 14; D3 vẫn hủy, D4 giữ.
- R7: bỏ số dòng viết tay; bỏ SMP OK trùng; bỏ “timer đa hart”; sửa C.1, #40,
  mục 20; `make-bundle.py` lấy code commit bằng `git log` (bỏ qua `docs:`).
- Harness mới có 237 case âm, gồm panic-hart1, preempt, W<=S và U<=T.
- Nợ: per-hart runqueue, per-VA TLB, x2APIC, hardware IOMMU, FS v2,
  `CapDelete`, net/input, compositor/ViFastKey. Panic RISC/AArch chưa có mã
  thoát lỗi phân biệt.
