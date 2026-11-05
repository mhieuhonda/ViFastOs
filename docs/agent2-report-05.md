# ViFastOs — Báo cáo Agent 2, vòng 05

**Ngày:** 2026-11-05  
**Baseline:** `74b7d75` (code), `535ad16` (báo cáo vòng 04)  
**Người thực hiện:** Agent 2

## 1. Bảng bước A–H

| Bước | Trạng thái | Commit |
|---|---|---|
| A. Xác minh R3/R4/A.3 | ✅ xong | không sửa code |
| B. Revoke 2 pha / test | ✅ chỉ thêm test | `31f023b` |
| C. Maintenance đối xứng | ✅ xong | `c4854d1` |
| D. Hart phụ RISC-V idle | ✅ xong (deviation WFI) | `76f1c41` |
| E. Hart phụ chạy task thật | ❌ chưa bắt đầu | — |
| F. Mở affinity | ❌ chưa bắt đầu | — |
| G. AArch64 hart phụ | ❌ chưa bắt đầu | — |
| H. Docs | ✅ xong | docs cuối vòng |

Mục tiêu tối thiểu A–D đã xong. E/F/G chưa làm.

## 2. A.1 — Revoke Untyped có kiểm tra current trước mutation không?

Có. Đường code:

- `Kernel::revoke_cap` — `crates/vifast-kernel/src/lib.rs:2719`.
- `collect_untyped_subtree` — dòng 2651, được gọi tại 2730, chỉ **đọc**
  object table và ghi danh sách local.
- Kiểm tra current nằm ở **dòng 2738–2746**, sau collect và **trước**
  sort/destroy, trước khi xóa cap (`victim_bits` ở 2749+) và trước
  `destroy_object` (2796).
- Nếu có TCB current trên bất kỳ hart nào trong danh sách, trả
  `IllegalOperation` ngay.

Vì vậy không cần sửa logic B; chỉ thêm test.

## 3. A.2 — s0 của user có được lưu trước `mv s0, tp`?

Có.

- Lưu user `s0` (x8): `crates/vifast-platform-riscv64/src/trap.S:21`
  `sd x8, 64(sp)`.
- Sau đó mới `mv s0, tp`: dòng 54.
- Khôi phục: dòng 81 `ld x8, 64(sp)`.
- Trong khoảng giữa, `s0` là CpuState pointer và là callee-saved nên Rust
  handler giữ nguyên.

Không cần commit sửa `s0`.

## 4. A.3 — Nơi gửi maintenance và điều kiện hart

### RISC-V64

- `send_maintenance` — `platform.rs:155`; assert BKL owner, bỏ qua hart chưa
  online, mailbox `MAINT_SEQ/ACK/KIND`.
- `request_ipi` — 183; chọn target `hart == 0 ? 1 : 0`; không còn điều kiện
  cấm hart 1 gửi.
- `poll_maintenance` — 124; **không còn** `hart == 0` / `hart != 0`.
- `wake_hart` — 602; chỉ gửi reschedule tới hart 1 khi hart 1 online.
- `flush_all_user` — 485; `sync_icache` — 497; chỉ gửi khi đang giữ BKL.
- `start_smp` — 912; lấy BKL trước khi gửi IPI thử.
- Các chỗ còn `hart == 1`: `maintenance_target_online` (151) và `wake_hart`
  (602) — đây là target online/affinity, không phải điều kiện cấm gửi/poll.

### AArch64

- `send_maintenance` — `platform.rs:120`.
- `request_ipi` — 142; target 0/1 đối xứng.
- `poll_maintenance` — 88; không còn điều kiện hart.
- `start_smp` — 884–885; lấy BKL trước khi gửi IPI thử.
- Trap/IRQ handlers — 935, 962; dùng BKL poll mailbox.
- Không còn `hart == 0` / `hart != 0` trong đường gửi/poll.

## 5. B — Revoke test

Commit `31f023b` chỉ thêm test:

```text
revoke_untyped_rejects_child_current_on_other_hart_before_mutating
```

Test tạo TCB sâu trong cây Untyped, cho TCB current trên hart 1, snapshot
`untyped_usage` + cap root, gọi `revoke_cap`, xác nhận:

- trả `IllegalOperation`;
- `untyped_usage` không đổi;
- TCB vẫn tồn tại;
- cap root không đổi;
- invariants vẫn đúng.

## 6. C — Mailbox maintenance đối xứng

Commit `c4854d1`.

- Bỏ `IPI_PENDING`/`IPI_ACK` toàn cục.
- Thêm theo hart đích:
  - `MAINT_SEQ[hart]` — số yêu cầu;
  - `MAINT_ACK[hart]` — số đã xử lý;
  - `MAINT_KIND[hart]` — bit TLB / I-cache / reschedule.
- `send_maintenance(target, kind)`:
  - assert đang giữ BKL;
  - bỏ qua target chưa online hoặc chính mình;
  - tăng seq, OR kind, gửi IPI;
  - chờ ACK >= seq, trong lúc chờ tự poll mailbox của mình.
- `poll_maintenance` xử lý mọi hart, không giới hạn hart != 0.
- Host test `vifast-arch`: khóa hai chiều, mỗi bên giữ khóa và spinner poll
  maintenance; timeout 5s; PASS.
- QEMU 3/3 PASS trước khi commit C.

## 7. D — Hart phụ RISC-V idle

Commit `76f1c41`.

### Affinity

- `Tcb.affinity: u8`, mặc định `1` (hart 0).
- `pick_on(hart)` bỏ qua TCB không có bit hart.
- `has_ready_on(hart)` cho runtime quyết định reschedule.
- Host test `affinity_filters_pick_on_hart`.

### Idle sequence

```text
init_cpu(1)
set stvec trap_entry
enable sie.STIE | sie.SSIE
set_timer(now + TIMER_INTERVAL)
SECONDARY_ONLINE++
loop:
  if !SCHED_READY: poll_maintenance(); spin
  bkl = BKL.lock_with(poll_maintenance)
  pick_on(1)                  # luôn None ở bước D
  drop(bkl)
  idle step                   # spin ngắn (xem deviation)
  đọc sip:
    STIP -> timer_wakes++, set_timer(now+interval)
    SSIP -> ipi_wakes++, clear SSIP, poll_maintenance()
```

- Không xóa pending trước idle step.
- `SCHED_READY` được set trong `enter_user` của hart 0 **sau** assert BKL
  release, tránh hart 1 giữ BKL làm panic lúc boot.
- Reschedule IPI: runtime kiểm tra `has_ready_on(1)` sau syscall và gọi
  `wake_hart(1)`; mailbox kind `MAINT_RESCHEDULE`. Ở bước D chưa có TCB
  affinity hart 1 nên nhánh này không kích hoạt; IPI wake hiện đến từ
  maintenance thử lúc SMP start.

### Deviation: WFI

WFI với `sstatus.SIE=0` không ổn định trên QEMU/OpenSBI trong test:
- có lần timeout 20s, hart 1 không wake/ACK;
- có lần illegal instruction tại `sret` khi thử dùng S-mode trap timer.

Đã thử:
1. S-mode timer trap + `sret`: lỗi illegal instruction tại `sret`
   (`scause=0x2`, `stval=0x10200073`).
2. Ép `sstatus.SPP=1` trước `sret`: vẫn lỗi.
3. WFI + SIE=0 + poll SIP: 10 lần đầu PASS, nhưng lần sau timeout.

Giải pháp chốt: **idle spin ngắn thay WFI**, vẫn giữ timer SBI/IPI wake và
đếm đúng. Đây là deviation so với step D; ghi rõ để Agent 0 quyết định.

### Kết quả 10 lần

```text
run1  hart1_wakeups=25
run2  hart1_wakeups=25
run3  hart1_wakeups=25
run4  hart1_wakeups=25
run5  hart1_wakeups=25
run6  hart1_wakeups=25
run7  hart1_wakeups=25
run8  hart1_wakeups=24
run9  hart1_wakeups=24
run10 hart1_wakeups=25
```

Harness RISC-V yêu cầu đúng 1 `SMP IDLE OK` và `K > 0`.

## 8. E — Double-fetch / task hart 1

E chưa bắt đầu. Chưa có `smp-worker`, chưa in `SMP SCHED OK`.

Danh sách sơ bộ các syscall copy buffer user (để rà soát ở E):

- `Open` — `syscall::dispatch` dòng 765: `read_user` một lần vào `path`.
- `Write` — dòng 793: `read_user` một lần vào `buf`.
- `FrameWrite` — dòng 810: `read_user` một lần vào `buf`.
- `BlockWrite` — dòng 829: `read_user` một lần vào `buf`.
- `BlockRead` — dòng 838: `read_user` (đọc output path) một lần.
- `Debug` — platform `read_user` trong trap; cần kiểm tra khi có hart phụ.

Chưa kết luận syscall nào double-fetch thực sự; E sẽ làm danh sách đầy đủ.

## 9. Test cuối vòng

```text
./scripts/build.sh                 PASS, không warning
vifast-kernel                      86/86 PASS
vifast-arch                         5/5 PASS
vifast-pte                         11/11 PASS
vifast-elf                         17/17 PASS
cargo fmt --all -- --check         PASS
QEMU 10/10 PASS sau D
test-persist.py                    RISC-V/AArch64/x86_64 PASS
```

QEMU RISC-V cuối:

```text
SMP OK (harts=2, ipi=1)
SMP IDLE OK (hart1_wakeups=K), K > 0
PERCPU CLOBBER OK
```

## 10. Headroom / SHA

```text
RISC-V kernel_end = 0x80390120
headroom          = 458400 byte (0x6ff30)
ViFastOs-architecture-summary.txt:
38c77fcf62731f22894520cb7a0227b4552a35a17b81c5b581498bae10bf1067
```

## 11. Nợ mới

- E/F/G chưa làm; hart 1 RISC-V chưa chạy user task.
- WFI bị thay bằng idle spin; cần thiết kế trap S-mode an toàn hơn hoặc
  dùng wakeup khác.
- AArch64 chưa có hart 1 scheduler/GIC CPU interface.
- x86 AP vẫn SKIP.
- Per-VA TLB, IPI kick revoke, FS v2, IOMMU phần cứng, net/input,
  compositor vẫn ngoài vòng.
