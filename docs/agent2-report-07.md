# ViFastOs — Báo cáo Agent 2, vòng 07

**Ngày:** 2026-11-05  
**Baseline:** `b76e930` (code), `5d9a785` (docs vòng 06)  
**Người thực hiện:** Agent 2

## 1. Bảng task 0–5

| Task | Trạng thái | Commit |
|---|---|---|
| 0. Bundle một file | ✅ (từ vòng 06, giữ nguyên) | `067b0d9` |
| 1. Bằng chứng SMP thật | ✅ | `e05775a`, `ce7dbc9` |
| 2. RISC-V idle WFI | ✅ | `b7f26ef` |
| 3. AArch64 hart phụ | ⏸ dừng, đã revert | không commit |
| 4. Docs | ✅ | docs cuối vòng |
| 5. Mở affinity | ❌ chưa bắt đầu | — |

Task 1 có thêm `ce7dbc9` để giới hạn worker về RISC-V, tránh tạo worker
không có scheduler trên AArch64.

## 2. Test cuối vòng

```text
./scripts/build.sh                 PASS, không warning
vifast-kernel                      87/87 PASS
vifast-arch                         5/5 PASS
vifast-pte                         11/11 PASS
vifast-elf                         17/17 PASS
cargo fmt --all -- --check         PASS
QEMU Task 1                       10/10
QEMU Task 2 (WFI)                 10/10
test-persist.py                    RISC-V/AArch64/x86_64 PASS
```

RISC-V:

```text
SMP WORKERS OK (a=50, b=50)
SMP IDLE OK (hart1_wakeups=4)
SMP SCHED OK (hart0=N, hart1=M, resched=R)
```

## 3. Task 1 — Hai worker và bộ đếm thật

### Bộ đếm

`user_entries[hart]` tăng ở **hai chỗ**:

1. `enter_user`: lần đầu vào user.
2. `riscv_trap_handler`: mỗi trap từ user (`sstatus.SPP=0`) trước khi xử lý,
   nên mỗi lần quay lại user bằng `sret` đều được tính.

Do đó `N`/`M` phản ánh mọi lần về user, không còn là 1.

### Hai worker

- `smp_worker_a`, `smp_worker_b` dùng chung root VSpace, entry riêng.
- Stack:
  - `WORKER_A_STACK_BASE = CODE_VA + 0xd000`
  - `WORKER_B_STACK_BASE = CODE_VA + 0xe000`
  - mỗi stack 1 page, top +0x1000.
- ABI có const assert: A > `DATA_VA`, A không chồng B, B top <= CODE_VA+0x10000.
- Root cấp 2 TCB, 2 frame; mỗi TCB có endpoint cap slot 1; affinity `0b10`.
- Mỗi worker `Yield` 50 lần, gửi message label riêng tới endpoint root
  (`data[0]=50`), rồi loop `Yield` (không Exit).
- Root nhận endpoint (không dùng notification) vì root đang phải phục vụ
  fault endpoint; chọn endpoint tránh block notification làm mất fault4.
- Root đủ hai label + data 50 thì in `SMP WORKERS OK (a=50, b=50)`.

### N, M, R 10 lần

```text
run1  N=72851  M=104403 R=72740
run2  N=51900  M=78928  R=51789
run3  N=80640  M=115368 R=80529
run4  N=73836  M=104415 R=73725
run5  N=87933  M=129699 R=87822
run6  N=66348  M=97414  R=66237
run7  N=77375  M=110572 R=77262
run8  N=73980  M=105130 R=73869
run9  N=93332  M=136618 R=93221
run10 N=74472  M=107909 R=74361
```

Harness yêu cầu `SMP WORKERS OK` đúng 1 lần, `N > 1`, `M >= 100`, `R > 0`.

### Double-fetch

| Đường | Đọc user | Kết luận |
|---|---:|---|
| `DebugWrite` | 1 lần vào buffer kernel | An toàn |
| `Open` path | 1 lần | An toàn |
| `Write` buf | 1 lần | An toàn |
| `FrameWrite` | 1 lần | An toàn |
| `BlockWrite` | 1 lần | An toàn |
| `BlockRead` | 0 đọc, chỉ ghi output | An toàn |
| `TcbSetIpcBuffer` | 0 deref; validate mapping, lưu frame/VA | An toàn |
| IPC buffer copy | frame-to-frame | Unmap cùng frame xóa buffer |
| `TcbSetAffinity` / Wait / Signal / Poll | 0 pointer | An toàn |

Không phát hiện double-fetch mới.

## 4. Task 2 — WFI với interrupt bị che

- `sstatus.SIE=0`; `sie.SSIE|STIE` bật.
- Trình tự idle:
  1. lấy BKL;
  2. `pick_on(1)`;
  3. nhả BKL;
  4. `wfi`;
  5. đọc `sip`;
  6. STIP → `set_timer` mới; SSIP → xóa sau khi `poll_maintenance`.
- Không xóa pending trước WFI; không đổi `stvec`; không nhận trap kernel.
- Kết quả: 10/10 PASS, không treo. Đã bỏ idle spin.
- Giá trị N/M/R 10 lần nằm ở mục 3 (cùng worker).

## 5. Task 3 — AArch64 hart phụ: dừng

Đã thử 3a: sửa vector table để mỗi entry truyền offset vector vào
`aarch64_kernel_fault`, in thêm vector/MPIDR/SPSR/DAIF.

Kết quả: AArch64 boot hỏng, output root bị lặp/`ROOT PANIC`, không đạt
harness. Toàn bộ thay đổi AArch64 đã revert; không commit. Không có log vector
hợp lệ để kết luận.

3b/3c chưa bắt đầu. Lý do: 3a không xanh, nên không có nền chẩn đoán đáng tin;
giữ baseline xanh thay vì commit nửa vời.

## 6. Task 4 — Docs

Đã sửa summary:

- Bỏ các chuỗi cũ `chỉ idle`, `chưa vào scheduler`, `chưa chọn task`,
  `86 test`.
- Thêm syscall 20 `TcbSetAffinity`.
- Thêm D1: không nhận interrupt khi kernel; WFI với interrupt bị che.
- Thêm assertion `SMP WORKERS OK`, `N > 1`, `M >= 100`, `R > 0`.
- Cập nhật nợ R5 (panic không dừng hart khác), reschedule IPI đã có.
- Sinh bundle round 07 bằng `scripts/make-bundle.py`.

Grep cuối:

```text
chỉ idle              0
chưa vào scheduler    0
chưa chọn task        0
86 test               0
```

## 7. Headroom / SHA

```text
RISC-V kernel_end = 0x80390120
headroom          = 458352 byte
ViFastOs-architecture-summary.txt:
fc9910f3090379fc6bcc99a8eb50e910290bd75f99cdbea188cb3d60e7201094
```

## 8. Nợ mới

- AArch64 secondary: 3a diagnostics làm hỏng boot; cần làm lại cẩn thận,
  ưu tiên kiểm tra vector table offsets.
- 3b/3c chưa làm.
- Task 5 mở affinity chưa làm.
- Panic một hart không dừng hart khác (R5).
- x86 AP, per-hart runqueue, per-VA TLB, FS v2, IOMMU phần cứng, net/input,
  compositor vẫn ngoài vòng.
