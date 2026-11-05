# ViFastOs — Báo cáo Agent 2, vòng 08

**Ngày:** 2026-11-05  
**Baseline:** `ce7dbc9` (code), `184a61d` (docs vòng 07)

## 1. Bảng task

| Task | Trạng thái | Commit |
|---|---|---|
| 1. Sửa bão IPI reschedule | ✅ | `e895bda` |
| 2. BootInfo `sched_harts` | ✅ | `dbe7675` |
| 3a. AArch64 vector diagnostics | ✅ | `e3c4280` |
| 3b/3c. AArch64 idle/worker | ❌ chưa bắt đầu | — |
| 4. Docs + checker | ✅ | `b9bfad9`, docs cuối vòng |
| 5. Panic dừng toàn hệ thống | ❌ chưa bắt đầu | — |

## 2. Test

```text
./scripts/build.sh                 PASS, không warning
vifast-kernel                      87/87
vifast-arch                         5/5
vifast-pte                         11/11
vifast-elf                         17/17
QEMU Task 1                       10/10
QEMU Task 2                        3/3
QEMU Task 3a                       1/1 (sau đó 3 lần cuối Task 4)
test-persist.py                    RISC-V/AArch64/x86_64 PASS
cargo fmt --check                  PASS
scripts/check-docs.py              PASS
```

## 3. Task 1 — reschedule IPI

Nguyên nhân bão IPI:
- `runtime::syscall::dispatch` gửi `wake_hart(1)` sau **mọi syscall** khi
  `has_ready_on(1)`.
- `wake_hart` gọi `send_maintenance(MAINT_RESCHEDULE)` **chờ ACK**, không kiểm
  tra hart 1 có idle hay không.
- Vì vậy `R ≈ N - 111`, mỗi syscall hart 0 sinh IPI tới hart 1.

Đã sửa:
- Thêm `IDLE_HARTS`, `RESCHED_PENDING`, `RESCHED_SENT`, `RESCHED_WAKE`,
  `IDLE_ENTRIES`.
- Chỉ gửi khi hart đích `idle==1` và chưa có reschedule pending; tối đa một
  IPI mỗi cửa sổ idle.
- Reschedule là fire-and-forget, không chờ ACK; TLB/I-cache vẫn chờ ACK.
- Idle đặt `idle=1` khi còn BKL, kiểm tra lại `has_ready_on(1)` trước WFI;
  sau WFI xóa `idle`, xóa pending rồi xử lý `sip`.
- Output mới:
  `SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)`.

### 10 lần

```text
run1  N=322542 M=323064 S=1 W=1 I=5
run2  N=381230 M=381696 S=1 W=1 I=5
run3  N=475869 M=476250 S=1 W=1 I=5
run4  N=368856 M=369398 S=1 W=1 I=5
run5  N=213931 M=216899 S=1 W=1 I=6
run6  N=381208 M=381930 S=1 W=1 I=5
run7  N=545335 M=545936 S=1 W=1 I=5
run8  N=416073 M=416530 S=1 W=1 I=5
run9  N=419535 M=419874 S=1 W=1 I=5
run10 N=178578 M=175496 S=1 W=1 I=5
```

`S <= I` đúng; S/W giảm từ hàng chục nghìn xuống 1.

## 4. Task 2 — BootInfo sched_harts

- `BootInfo` lên version 2, thêm `sched_harts: u32` (offset 64, size 72).
- Runtime:
  - RISC-V trả `0b11`.
  - AArch64/x86 mặc định `0b01`.
  - Assert mask khác 0, có bit 0, không có bit ngoài 0b11.
- Root kiểm tra mask và chỉ tạo worker khi bit 1 bật; đã bỏ cfg theo kiến
  trúc khỏi root.
- Kernel host test `bootinfo_layout_is_stable` cập nhật size/offset.

## 5. Task 3 — AArch64

3a đã làm đúng phạm vi:
- Chỉ sửa 8 entry current-EL; dùng `.org vector_table + 0x80*n` để assembler
  phát hiện entry vượt kích thước.
- Lower-EL giữ nguyên.
- Handler in vector, MPIDR, SPSR, DAIF, ESR, ELR, FAR, ISR.
- QEMU AArch64 PASS, output cũ không đổi.

3b/3c chưa bắt đầu. Lý do: 3a mới xong và cần thêm GIC banked/timer/idle
CPU1; thời gian còn lại ưu tiên Task 1/2/4 và baseline xanh.

## 6. Task 4 — Docs automation

- Thêm `scripts/check-docs.py`; `make-bundle.py` gọi script và dừng nếu lỗi.
- Check positive: `20 TcbSetAffinity`, các chuỗi SMP, stack/BootInfo, counts,
  commit list.
- Check forbidden: `hart phụ idle`, `hart phụ chạy scheduler`, `chỉ idle`,
  `chưa vào scheduler`, `chưa chọn task`.
- Lần chạy đầu phát hiện:
  1. `hart phụ idle` còn 2 dòng.
  2. Thiếu commit `43d5200` trong danh sách.
- Đã sửa cả hai; script PASS.

## 7. Headroom / SHA

```text
RISC-V kernel_end = 0x80390120
headroom          = 458336 byte
summary SHA-256   = daf8b7a87a6e23bddedd8a325a5eade708efa3e800fd1508b12be235cd2efe78
```

## 8. Nợ

- 3b/3c AArch64 chưa làm.
- Task 5 panic toàn hệ thống chưa làm.
- `check-docs.py` chưa parse đầy đủ toàn bộ SyscallId enum; hiện kiểm tra
  positive anchor `20 TcbSetAffinity` và các chuỗi harness.
- x86 AP, per-hart runqueue, per-VA TLB, FS v2, IOMMU phần cứng, net/input,
  compositor vẫn ngoài vòng.
