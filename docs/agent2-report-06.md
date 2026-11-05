# ViFastOs — Báo cáo Agent 2, vòng 06

**Ngày:** 2026-11-05  
**Baseline:** `76f1c41` (code), `f9e7e98` (docs vòng 05)  
**Người thực hiện:** Agent 2

## 1. Bảng task 0–5

| Task | Trạng thái | Commit |
|---|---|---|
| 0. Bundle một file | ✅ | `067b0d9` |
| 1. Hart 1 RISC-V chạy task | ✅ | `b76e930` |
| 2. AArch64 hart phụ | ⏸ dừng có lý do | không commit |
| 3. Điều tra WFI RISC-V | ❌ chưa bắt đầu | không commit |
| 4. Mở affinity | ❌ chưa bắt đầu | không commit |
| 5. Docs + bundle | ✅ | docs cuối vòng |

## 2. Task 0 — Bundle

- Thêm `scripts/make-bundle.py`.
- Sinh `docs/bundles/ViFastOs-round-05.md` cho baseline hiện có.
- Bundle gồm:
  1. toàn văn report;
  2. quick status (code commit, docs commit, test, headroom, summary SHA);
  3. toàn văn summary trong khối `text`;
  4. dòng cuối SHA-256 của report và summary.
- Script kiểm tra report/summary tồn tại và SHA summary khớp
  `docs/overnight-log.md`; sai thì exit 1.
- Final bundle vòng 06 được sinh lại bằng script, không sửa tay.

## 3. Task 1 — Hart 1 RISC-V chạy task

### Cách tạo worker

Chọn **dùng chung root VSpace** thay vì tạo VSpace/ELF mới:

- Root tạo TCB `smp-worker`.
- Cấp một stack riêng trong root VSpace tại
  `WORKER_STACK_BASE = CODE_VA + 0xd000`, top `+0xe000`.
- Worker entry là `smp_worker_entry` trong chính root ELF; root truyền địa chỉ
  hàm này qua `TcbConfigure`.
- Không copy cap; worker chỉ gọi `Yield`, nên không cần cspace riêng.
- Worker chạy 50 `Yield`, sau đó loop `Yield` mãi. **Không gọi Exit** vì
  `UserAction::Exit` tắt toàn máy, sẽ cắt test-exit của hart 0.

Lý do: tránh thêm ELF/module mới và tránh thay đổi layout server/client;
worker chỉ cần chứng minh context switch/preemption trên hart 1.

### Affinity

- Dùng syscall có sẵn `TcbSetAffinity = 20`.
- Kernel:
  - `set_tcb_affinity(tcb, mask)`.
  - Mask 0 hoặc bit ngoài `MAX_HARTS` trả `InvalidArg`.
  - Dispatch kiểm tra cap `Tcb` + quyền `WRITE`.
- User wrapper: `vifast_user::tcb_set_affinity`.
- Root đặt worker `0b10`; root/server/client/fault giữ mặc định `0b01`.
- Vì kernel chưa biết hart online, mask chỉ kiểm tra theo `MAX_HARTS`; ràng buộc
  online được thực hiện bằng assert `online_harts <= 2` trong mailbox.

### Console

- Worker không in gì.
- `SMP SCHED OK` được in trong timer test-exit khi đang giữ BKL.
- `DebugWrite` root/client đi qua trap + BKL.
- Các đường panic vẫn in trực tiếp ngoài BKL (không đổi trong vòng này).

### Kết quả 10 lần

```text
run1  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run2  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run3  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run4  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run5  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run6  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run7  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run8  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run9  SMP SCHED OK (hart0=1, hart1=1, resched=1)
run10 SMP SCHED OK (hart0=1, hart1=1, resched=1)
```

`resched=1` đến từ IPI reschedule thật khi root `TcbResume(worker)` làm
`has_ready_on(1)` true. Harness RISC-V bắt buộc đúng 1 dòng, `hart1 > 0`,
`resched > 0`.

### Double-fetch

| Syscall/đường | Đọc user mấy lần | Rủi ro | Kết luận |
|---|---:|---|---|
| `DebugWrite` | 1 lần vào buffer kernel | Không | An toàn |
| `Open` path | 1 lần vào `path` | Không | An toàn |
| `Write` buf | 1 lần vào `buf` | Không | An toàn |
| `FrameWrite` buf | 1 lần vào `buf` | Không | An toàn |
| `BlockWrite` | 1 lần vào `buf` | Không | An toàn |
| `BlockRead` | 0 đọc; chỉ ghi output | Không | An toàn |
| `TcbSetIpcBuffer` | 0 deref; chỉ validate mapping + lưu frame/VA | Không | An toàn |
| IPC buffer copy | frame-to-frame, không đọc user pointer | Không | Unmap cùng frame xóa `ipc_buffer` |
| `TcbSetAffinity` | 0 pointer | Không | An toàn |
| `Wait/Signal/Poll` | 0 pointer | Không | An toàn |

Không phát hiện double-fetch mới. BKL giữ mapping ổn định giữa validate/copy.

## 4. Task 2 — AArch64 hart phụ: dừng

Đã thử triển khai:

- GIC CPU interface cho CPU1 (`GICC_PMR`, `GICC_CTLR`).
- Timer per-CPU + VBAR cho secondary.
- Idle loop BKL/pick, reschedule SGI, worker affinity hart 1.
- BKL owner per-hart và `current_on(hart)`.

Kết quả lần chạy:

```text
SELFTEST PASS ...
KERNEL EXCEPTION ESR=0x0 ELR=0x4008ebb4 FAR=0x0
```

`ELR=0x4008ebb4` nằm trong `poll_maintenance`, ngay sau khi bật GIC CPU1/timer
và trong lúc mailbox IPI được xử lý. Không có `SMP OK`, nên lỗi xảy ra trước
khi in SMP. Giả thuyết: exception từ secondary (GIC/DAIF/MMU hoặc store atomic
trong mailbox), chưa đủ dữ liệu để kết luận.

Đã **revert toàn bộ** thay đổi AArch64; baseline Task 1 vẫn xanh. Không commit
Task 2.

## 5. Task 3 — WFI RISC-V

Chưa bắt đầu trong vòng này. Không có dữ liệu `info registers` mới.

Dữ liệu cũ từ vòng 05 vẫn giữ: WFI+SIE=0 có lần timeout, và S-mode trap dùng
chung `trap_entry` bị illegal `sret` (`scause=0x2`, `stval=0x10200073`).
Hiện vẫn dùng idle spin.

## 6. Task 4 — Mở affinity

Chưa bắt đầu. Chỉ worker có affinity hart 1; server/client/fault/root vẫn hart 0.

## 7. Test cuối vòng

```text
./scripts/build.sh                 PASS, không warning
vifast-kernel                      87/87 PASS
vifast-arch                         5/5 PASS
vifast-pte                         11/11 PASS
vifast-elf                         17/17 PASS
cargo fmt --all -- --check         PASS
QEMU 10/10 PASS (Task 1)
test-persist.py                    RISC-V/AArch64/x86_64 PASS
```

RISC-V:

```text
SMP OK (harts=2, ipi=1)
SMP IDLE OK (hart1_wakeups=4)
SMP SCHED OK (hart0=1, hart1=1, resched=1)
PERCPU CLOBBER OK
```

## 8. Headroom / SHA

```text
RISC-V kernel_end = 0x80390120
headroom          = 458352 byte (0x6fef0)
ViFastOs-architecture-summary.txt:
df8f8378facb932c46327187ccf8da5792f678d1010a2c0d56dd12ad069d1d28
```

## 9. Nợ mới

- AArch64 secondary scheduler: exception `ELR=0x4008ebb4`, ESR=0; cần QEMU
  monitor/registers trên hart 1.
- WFI RISC-V chưa điều tra dứt điểm; vẫn spin.
- Affinity mới chỉ dùng worker; server/client/fault chưa chạy hart 1.
- x86 AP vẫn SKIP.
- Mailbox giới hạn 2 hart bằng assert; chưa tổng quát hóa.
- Per-VA TLB, IPI kick revoke, FS v2, IOMMU phần cứng, net/input,
  compositor vẫn ngoài vòng.
