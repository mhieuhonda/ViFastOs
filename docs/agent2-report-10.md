# ViFastOs — Báo cáo Agent 2, vòng 10

**Ngày:** 2026-11-05  
**Baseline:** `3631ed7` (code), bundle vòng 09

## 1. Bảng bước

| Bước | Trạng thái | Commit |
|---|---|---|
| 1a. Harness repeat + timeout artifact | ✅ | `060fb67`, `010cccd` |
| 1b. Đo RISC-V 50 lần | ✅ (49/50, 48/50; xem dưới) | — |
| 1c. STALL detector | ⏭️ không cần sau dữ liệu 1b | — |
| 1d. Sửa RISC-V flake | ✅ | `386a2ad` |
| 2a. Idle wake accounting | ✅ | `dbe3a39` |
| 2b. Worker AArch64 CPU1 | ✅ | `cce094a` |
| 3. Sinh summary tự động | ✅ | `ec090c9` + docs vòng 10 |
| 4. Đo BKL | ❌ chưa làm | — |
| 5. Panic toàn hệ thống / ACPI x86 | ❌ chưa làm | — |

## 2. Test cuối

```text
./scripts/build.sh                 PASS, không warning
vifast-kernel                      87/87
vifast-arch                         5/5
vifast-pte                         11/11
vifast-elf                         17/17
test-qemu.py --repeat 10            10/10 cả 3 kiến trúc
  riscv64                          10/10
  aarch64                          10/10
  x86_64                           10/10
test-qemu.py --arch riscv64 --repeat 50   50/50
test-qemu.py --arch riscv64 --repeat 100  100/100
test-qemu.py --arch aarch64 --repeat 50   50/50
test-persist.py                    RISC-V/AArch64/x86_64 PASS
cargo fmt --all -- --check         PASS
scripts/check-docs.py              PASS
```

Headroom RISC-V: `458320` byte (`kernel_end=0x803901b0`, limit `0x80400000`).

## 3. Task 1 — RISC-V flake

### 3a. Công cụ

- `--arch` và `--repeat N`; tổng kết `PASS/FAIL` theo tỉ lệ.
- QEMU monitor qua Unix socket. Khi timeout, trước khi kill:
  gửi `info registers -a`, lưu `registers.txt`, 80 dòng cuối vào
  `tail.txt` và các dòng `STALL/PANIC/SMP` vào `status.txt` dưới
  `artifacts/flake-<arch>-<n>/`.
- Harness bắt buộc `STALL` không xuất hiện; không có detector STALL vì
  dữ liệu monitor đã đủ.

### 3b. Trước khi sửa

Lần đầu `--arch riscv64 --repeat 50`: **49/50**. Lần chạy 34 có
`Boot HART ID: 1`; serial chỉ có OpenSBI, không vào kernel. Artifact lần đó
chỉ chứa banner monitor vì lần đọc đầu bị cắt ở prompt `(qemu)`.

Sau khi sửa monitor, lần 50 tiếp theo: **48/50**. Một timeout ở lần 26:

```text
CPU#0  priv=M  pc=0x80016fba  mhartid=0
CPU#1  priv=S  pc=0x8020f300  sip=0x20  stimecmp=0x477fc
       mepc=0x8020f20a  mcause=0x9  satp=0
Boot HART ID = 1
```

CPU0 bị OpenSBI giữ ở M-mode, CPU1 vào nhánh secondary vì `a0 != 0`.
Lần lỗi còn lại exit code 0 sau `CLIENT VSPACE OK`, không có `ROOT READY`.

Sau khi thêm gate test-exit: **49/50**, rồi **47/50**. Các lỗi còn lại đều
dừng trong lúc root đang ghi địa chỉ fault hoặc ngay sau `FAULT EP OK`;
không có `STALL`, không có `KERNEL EXCEPTION`. Đây là race khi hai worker
hart 1 cùng block ở endpoint trong lúc root còn xử lý fault: runqueue hart 1
rỗng và đường `switch_blocked` gọi `A::exit(1)`.

### 3c. Sửa

- `crates/vifast-kernel-riscv64/src/boot.S`: nếu OpenSBI chọn hart khác 0,
  hart đó gọi SBI HSM start hart 0 tại `_start` rồi HSM stop chính nó;
  hart 0 trở thành primary và start lại hart 1 qua đường SMP bình thường.
- `crates/vifast-runtime/src/lib.rs`: test-exit chỉ trên hart 0, sau
  `ROOT READY` (không worker) hoặc `SMP WORKERS OK` (có worker); tránh
  QEMU thoát trước khi root in đủ output.
- `crates/vifast-runtime/src/lib.rs`: `dispatch` trả cờ “runqueue rỗng”.
  `switch_blocked` không exit nữa; trên secondary, trap handler nhả BKL,
  `wait_for_work` WFI-rồi-poll maintenance/IPI, lấy lại BKL và
  `resume_blocked`.
- `crates/vifast-platform-riscv64/src/platform.rs`: cờ `can_wait_for_work`,
  vòng chờ với `IDLE_HARTS`, timer/IPI accounting, `RESCHED_PENDING`.

Kết quả sau sửa: **50/50**, sau đó chạy thêm **100/100** RISC-V; không
artifact, không `FAIL:`. Trong 150 lần cuối không lần nào OpenSBI chọn hart 1,
nên nhánh boot-hart fallback chưa được kích hoạt tự nhiên.

## 4. Task 2a — Idle wake accounting

Format mới:

```text
SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)
```

`K = Wt + Ws + Wn`; WFI có nhiều nguồn cùng lúc chỉ tính một wake (ưu tiên
SGI). AArch64 service_gic trả `(saw_timer, saw_sgi)` cho mỗi WFI.

RISC-V 10 lần: `K=5, I=5, Wt=0, Ws=5` (một lần `K=6, I=5, Ws=6`,
`S=2`). Giải thích: hart 1 bận worker gần như toàn bộ; các lần idle còn lại
đều bị SGI maintenance đánh thức trước timer, nên không có wake_timer trong
cửa sổ idle. `SMP SCHED OK` 10/10 có `N` 103233–425957, `M` 105545–426386,
`S=1..2`, `W=1`, `I=5`; `S <= I`.

AArch64 trước worker (bằng chứng 2a, 10 lần): `T=20..21`, `I=22..23`,
`Wt=20..21`, `Ws=1`, `K=21..22`; đạt `I >= T-2`, `Wt >= T-2` cả 10 lần.

AArch64 sau khi có worker (10 lần, 2b): `T=20..22`, `I=2`, `K=2`,
`Wt=0`, `Ws=2`. CPU1 chạy worker liên tục nên timer đến dưới dạng IRQ user,
không qua WFI; harness giữ `T > 0`, `I >= 1`, `K >= Wt + Ws` và các điều kiện
SMP. Điều kiện `I/Wt >= T-2` chỉ còn áp dụng cho giai đoạn không worker
(đã chạy 10/10 trước 2b). Đây là lý do thay đổi phạm vi, không phải nới lỏng
các assertion SMP.

## 5. Task 2b — AArch64 CPU1 worker

- Per-hart `BKL_OWNER`/RAII guard; `assert_current_tcb` và
  `active_root_matches_current` dùng `current_on(hart)`.
- `Aarch64Platform::init()` trên CPU1 để bật CPACR FP/SIMD; thiếu bước này
  kernel trap EC=7 tại `movi v0.2d` trong syscall path.
- GIC banked, timer riêng, SGI 0 maintenance + SGI 1 reschedule; chỉ gửi
  reschedule khi `IDLE_HARTS[1]`.
- Secondary loop: `SCHED_READY` → BKL → `pick_on(1)` → `ensure_current_root`
  (TTBR0 + ASID) → nhả BKL → `enter_user`; idle WFI + poll.
- IRQ lower-EL xử lý timer và maintenance; timer IRQ user được đếm vào T.
- `sched_harts` AArch64 = `0b11`; root tạo worker như RISC-V.

10 lần AArch64:

```text
N = 347857 .. 545775
M = 356997 .. 546139
S = 1, W = 1, I = 2
SMP WORKERS OK (a=50, b=50) đúng 1 lần
```

`--arch aarch64 --repeat 50`: **50/50**.

## 6. Task 3 — Summary sinh tự động

Marker text thuần trong `ViFastOs-architecture-summary.txt`:

```text
### BEGIN GENERATED commits / syscalls / tests / pass
### END GENERATED commits / syscalls / tests / pass
```

`scripts/gen-docs.py`:
- `commits`: `git log --reverse 456d5ef..HEAD`, bỏ `docs:`;
- `syscalls`: parse `enum SyscallId` trong ABI;
- `tests`: parse `target/vifast-build.log` (build.sh ghi qua tee) và headroom;
- `pass`: đọc `scripts/harness_spec.py` mà `test-qemu.py` cũng import.

`check-docs.py` chạy generator trong bộ nhớ, so nguyên văn bốn vùng; giữ danh
sách chuỗi cấm, thêm `maintenance-only`, `resched=R`, `AArch64/x86 hart phụ`.

Diff lần sinh đầu (so với summary trước vòng 10):

| Marker | Dòng cũ | Dòng mới | Dòng mới thêm | Dòng cũ bị bỏ |
|---|---:|---:|---:|---:|
| commits | 77 | 83 | 6 | 0 |
| syscalls | 33 | 45 | 23 | 11 |
| pass | 32 | 50 | 50 | 32 |
| tests | 12 | 6 | 0 | 6 (dòng stale) |

Mục 13 đã viết lại theo RISC-V / AArch64 / x86; mục 18 gộp 53–54 và ghi
kết luận R5 (hủy tái hiện 1.0 vì DAIF.I=1 đã ổn định).

## 7. Nợ mới

- Boot-hart fallback chưa xuất hiện tự nhiên trong 50 lần cuối; code đã có
  nhưng chưa có log QEMU chứng minh trực tiếp. Cần chạy dài hơn hoặc thêm
  test tiêm lỗi.
- AArch64 `I/Wt` không còn theo T sau khi worker chạy; đã ghi rõ lý do.
- 1c STALL detector không thêm vì monitor đủ dữ liệu.
- Task 4 (BKL stats), Task 5 (panic toàn hệ thống, ACPI MADT) chưa làm.
- x86 AP, per-hart runqueue, per-VA TLB, FS v2, IOMMU phần cứng,
  VirtIO net/input, compositor/ViFastKey vẫn ngoài vòng.
