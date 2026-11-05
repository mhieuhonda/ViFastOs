# ViFastOs — Báo cáo vòng 22

**Ngày:** 2026-11-06  
**Baseline:** sau `b72183c` (vòng 21 + CI gate cục bộ)  
**Trạng thái:** 50/50 syscall implemented; 3/3 kiến trúc QEMU PASS.

## 1. Tóm tắt

Vòng 22 hoàn tất các mảnh còn thiếu của ABI và mở driver input thật:

1. SchedContext budget/period (`BlockedBudget`).
2. IRQ object + timer IRQ delivery.
3. UART IRQ trên cả 3 kiến trúc + shell IRQ-driven.
4. CNode phẳng 64 slot, CNodeMint, CNodeInstall, demo end-to-end.
5. VFS `O_APPEND`.
6. Block cache write-back cho data, write-through journal/metadata.
7. Demand paging/fault-resume demo (`PAGER OK`).
8. `TcbDestroy` + fault-slot identity + reap fault/demo TCB.
9. `O_EXCL`/`O_DIRECTORY` và test mở file.
10. Pager server phục vụ hai client, in `PAGER2 OK`.

Không còn syscall placeholder; các phần lớn còn lại là hardware IOMMU,
network/GPU/compositor, CNode guard/nhiều cấp, pager/mount và formal.

## 2. Commit chính

| Commit | Nội dung |
|---|---|
| `aaafbe0` | SchedContextConfigure/Bind, budget/period, BlockedBudget |
| `814e28f` | IRQ object, IrqGet/SetNotification/Ack, timer IRQ |
| `c521528` | UART IRQ RISC-V PLIC / AArch64 GIC SPI 33 / x86 PIC IRQ4 |
| `6d3129f` | CNode phẳng, CNodeMint, CNodeInstall |
| `b014c8b` | CNode demo TCB + `CNODE OK` |
| `e9b8286` | VFS O_APPEND |
| `363574a` | Write-back data block cache |
| `801fb13` | Test CNode delete/revoke |
| `46e8ad6` | TcbDestroy/reap, fault-slot identity, O_EXCL/O_DIRECTORY, pager hai client |
| docs | Các commit `docs: sync ...` sau mỗi mốc |

## 3. Chi tiết

### 3.1 SchedContext

- `Kind::SchedContext { budget, period, remaining, period_start, bound }`.
- `SchedContextConfigure`: `1 <= budget <= period`.
- `SchedContextBind`: một context ↔ một TCB, idempotent cùng cặp.
- `tick()` trừ budget của TCB Running; hết budget → `BlockedBudget`,
  `wake_at = hết period`.
- `wake_budgeters()` nạp lại budget, đưa Ready; không ghi resume register vì
  đây là preemption bất đồng bộ.
- Destroy/revoke context tự unbind và đánh thức TCB đang budget-blocked.
- Root bind worker C 1000/1000, in `SCHED CTX OK`.

### 3.2 IRQ

- ABI `ObjType::Irq`; logical `IRQ_TIMER=0`, `IRQ_UART=1`.
- `IrqGet` root-only; một object cho mỗi logical IRQ.
- `IrqSetNotification` signal ngay nếu IRQ đang pending; `IrqAck` xoá pending.
- `irq_raise` coalesce edge; timer tick raise `IRQ_TIMER`; handler UART raise
  `IRQ_UART`.
- Root cấp timer IRQ + notification cho client; client Wait/Ack in `IRQ OK`.

### 3.3 UART IRQ

- `RxQueue` lock-free 64 byte trong `vifast-arch`; `stash_rx` drain từ handler.
- RISC-V: map PLIC 0x0c000000 + block context 0x0c200000; source 10, context 1;
  claim/complete đúng offset `+0x4`.
- AArch64: GIC SPI 33, target CPU0, PL011 RXIM; handler drain/ICR/EOI.
- x86_64: IDT vector 0x24, unmask PIC IRQ4, COM1 IER bit0; handler drain/EOI.
- Shell block trên notification UART rồi `ConsoleRead`; không còn yield-poll.
- Manual test `-serial stdio -monitor none` với kernel no-default đã echo
  input trên cả RISC-V, AArch64, x86_64.

### 3.4 CNode

- Bảng phẳng 64 slot, `Tcb.cnode` thay thế cspace nhúng.
- `CNodeMint`: cnode cap `WRITE`, dest 1..63, source cap trong cspace caller,
  rights giảm dần, badge một lần.
- `CNodeInstall`: cài/bỏ CNode cho TCB Inactive/Suspended/Faulted.
- CNode đang dùng không destroy được; revoke/delete/accounting/invariant đã nối.
- Demo: root mint endpoint vào slot 1, cài CNode cho TCB mới; worker chỉ thấy
  cap đó, gửi `CNODE` label; root in `CNODE OK`.
- Chưa có guard, nhiều cấp, zombie reaping.

### 3.5 VFS

- `O_APPEND = 0x10`: server giữ flags fd, ghi tại `Fs::info(ino).size`.
- `CachedDev`: data LBA >= `DATA_START` dirty; metadata/journal write-through.
- Eviction/commit/fsync ghi dirty; `commit()` flush trước khi clear journal
  commit; replay cũng flush trước khi clear.
- `O_TRUNC` vẫn hoạt động; test `/append` kiểm "a"+"b" → "ab", size 2.
- `O_EXCL=0x20`: `O_CREATE|O_EXCL` trả `EEXIST` nếu file đã có;
  `O_DIRECTORY=0x40`: chỉ mở directory, file trả `ENOTDIR`, kết hợp create
  trả `EINVAL`. Test `FS OPEN FLAGS OK`.

### 3.6 Pager/fault resume

- Pager TCB root VSpace đọc `PAGER_VA` chưa map; fault endpoint riêng.
- User-space pager server (TCB riêng) nhận fault, `FrameMap` frame vào
  VSpace, `TcbResume` pager; pager chạy lại entry, đọc value, gửi label;
  server in `PAGER OK`.
- Root cấp cho server endpoint/TCB/VSpace/Frame caps; page table được tạo
  trước bằng map/unmap để FrameMap của server không cần PageTable mới.
- Server giữ cap cho hai cặp TCB/frame; chọn cặp theo fault address.
- Fault message có `data[2] = fault endpoint slot`, `data[3] = TCB index`;
  root dùng slot để reap đúng TCB.
- `TcbDestroy` giải phóng object TCB sau khi reparent cspace; root reap 4
  fault TCB + CNode demo, in `TCB REAP OK`.
- Harness yêu cầu 6 USER FAULT (4 cũ + 2 pager), `PAGER OK` và `PAGER2 OK`
  đúng một lần.
- Chưa có pager server tổng quát cho mọi VSpace/fault.

## 4. Kiểm tra

```text
vifast-kernel   112/112
vifast-fs        22/22
vifast-arch       7/7
vifast-pte       11/11
vifast-elf       17/17
vifast-acpi       8/8
QEMU --repeat 3  3/3 cả 3 kiến trúc
persistence      7 phase ×3 PASS
harness âm       417 case PASS
cargo fmt/check  PASS, 0 warning
```

RISC-V headroom 200.016 byte; object table 147, TCB tối đa 18, metadata
80 KiB / data 304 KiB. Root data untyped 74/76 page, 70/76 object (RISC-V).

## 5. Còn lại

- Hardware IOMMU (VT-d/SMMU); `IoMap` vẫn software.
- Network, GPU/framebuffer, compositor, ViFastKey.
- CNode guard/nhiều cấp/zombie; pager tổng quát/mount; barrier phần cứng.
- Khóa mịn/x2APIC, test >2 hart.
- Formal verification/CI hosted.
- Userland đầy đủ (libc, shell hệ thống, dynamic linking).
