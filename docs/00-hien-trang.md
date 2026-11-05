# 00. Hiện trạng triển khai (2026-11-06)

Tài liệu này là nguồn trạng thái hiện tại. Các tài liệu `01`–`07`, `legacy/`
và `python-model/` là thiết kế tham chiếu lịch sử; một số chi tiết trong đó
chưa được port sang bản bare-metal này.

## Đã chạy được

| Hạng mục | RISC-V64 | AArch64 | x86_64 |
|---|---|---|---|
| Boot QEMU | OpenSBI | QEMU `virt` | OVMF/UEFI |
| Console sớm | 16550 | PL011 | COM1 |
| Self-test portable | PASS | PASS | PASS |
| User mode | U-mode `sret` | EL0 `eret` | ring3 `iretq` |
| Syscall vào kernel | `ecall` | `svc #0` | `int 0x80` |
| Timer interrupt + preempt | SBI timer | GICv2 + generic timer | PIC/PIT + LAPIC |
| SMP 2 hart + scheduler | có (HSM) | có (PSCI) | có (ACPI MADT + LAPIC) |
| Per-hart ready queue + work stealing | có | có | có |
| BKL ticket + maintenance IPI + per-VA TLB | có | broadcast | có |
| Root/server/client VSpace riêng | có | có | có |
| IPC Call/Reply, notification, cap transfer | có | có | có |
| Untyped con + BootInfo + TCB idle/restart/bind | có | có | có |
| VFS v3 journal + directory + permission | có | có | có |
| Hard link / symlink / double-indirect / O_APPEND / O_EXCL / O_DIRECTORY / STAT_PATH | có | có | có |
| Block cache write-back data + fsync/BlockFlush | có | có | có |
| Persistence 7 phase | có | có | có |
| Software IOMMU + DMA map | có | có | có |
| Sleep deadline (BlockedSleep) | có | có | có |
| Console input IRQ + shell | có | có | có |
| CNode phẳng + CNodeInstall demo | có | có | có |
| Demand paging + user-space pager server (2 client) | có | có | có |
| TCB destroy/reap + fault-slot identity | có | có | có |
| User fault suspend (không panic) | có | có | có |

Kiến trúc hiện tại: `vifast-kernel` portable + `vifast-runtime` generic +
`vifast-platform-*` BSP + `vifast-arch-*` hardware primitive + binary
`vifast-kernel-*` mỏng.

Kernel portable hiện có:

- object table cố định **147 object**, 18 TCB, 96 cap root / 64 cap task,
  CNode phẳng 64 slot, 64 mapping/VSpace, metadata 80 KiB / data 304 KiB,
  queue 8; `Kernel` nằm trong static storage của platform;
- capability install/copy/derive/revoke, badge, chuyển cap qua IPC với `GRANT`;
- endpoint Send/Recv, Call/Reply/ReplyRecv, notification signal/wait/poll;
- scheduler ưu tiên + round-robin, ready queue theo hart, work stealing;
- `Sleep` với `BlockedSleep`/`wake_at`; `TcbRestart`, `TcbBindNotification`;
- invariant checker cho object/capability/runqueue/VSpace/fd, W^X theo Frame,
  PA uniqueness;
- Frame/VSpace/PageTable, generic page-table walk, map/unmap, W^X, sync icache;
- Untyped/retype Frame/PageTable/VSpace/CNode/Endpoint/Notification/Tcb/
  SchedContext/Iommu;
- VSpace riêng, ASID/PCID, first-fit VA allocator, per-VA TLB shootdown;
- user-space VFS server, FS v3 journal, hard link/symlink, double-indirect,
  `STAT_PATH`, block cache write-back cho data, write-through journal/metadata,
  `fsync` → `BlockFlush`;
- software IOMMU domain + `IoMap`/`IoUnmap`;
- 50 syscall number: 50 implemented, 0 placeholder; CNode phẳng, SchedContext
  budget/period, timer IRQ/UART IRQ, TCB destroy/reap đã chạy;
- console input IRQ qua UART 16550/PL011/COM1, RxQueue lock-free và shell
  block trên notification UART thay vì polling.
- fault endpoint → user-space pager server map Frame → TcbResume cho hai
  pager TCB; in `PAGER OK`/`PAGER2 OK`. Fault message mang fault-slot + TCB index;
  root reap fault TCB và CNode demo bằng `TcbDestroy`.

## Chưa có

- Hardware IOMMU (VT-d/SMMU); chỉ có mô hình software.
- CNode nhiều cấp; UART IRQ hiện chỉ route BSP/hart 0.
- Network, GPU/framebuffer, compositor, ViFastKey; shell chưa có job control.
- Mount/pager tổng quát (demo pager mới xử lý hai client), barrier phần cứng
  thật, driver disk ngoài QEMU.
- Khóa mịn/x2APIC; scheduler mới test 2 hart.
- Allocator vật lý tổng quát/free object ngoài revoke.
- CI (GitHub không dùng), formal verification/Kani/Verus.
- Userland đầy đủ: libc, shell hệ thống, dynamic linking, ứng dụng thật.

## Hạn chế bring-up cần nhớ

- RISC-V trap dùng kernel trap stack qua `sscratch`; AArch64 dùng `SP_EL1`,
  x86_64 dùng `TSS.RSP0`.
- x86_64 identity map 0..1 GiB; code RX, data/stack NX; AP đã online.
- Hart 0 có idle TCB; hart phụ dùng platform idle loop nên `Sleep` trên hart
  phụ có thể fallback poll `GetTime`.
- FS giới hạn ảnh 4 MiB, 64 inode, tên 28 byte, file logic tối đa 8,4 MiB
  nhưng thực tế bị chặn bởi dung lượng ảnh.
- Driver block là VirtIO-MMIO (RISC-V/AArch64) và ATA PIO (x86), polling.
- QEMU-specific; chưa kiểm chứng máy thật.
- `docs/01`–`07`, `legacy/`, `python-model/` có thể lỗi thời.

## Lệnh kiểm tra

```bash
./scripts/build.sh
python3 scripts/test-qemu.py --repeat 3
python3 scripts/test-persist.py
python3 scripts/test-harness-spec.py
python3 scripts/check-docs.py
```

Kỳ vọng: 3/3 kiến trúc PASS, persistence 7 phase × 3 PASS, host test PASS,
0 warning. Client in `SLEEP OK` và `shell> ` sau demo; QEMU test-exit tự thoát.

## Tài liệu thành phần

- [`docs/11-hien-tai/00-index.md`](11-hien-tai/00-index.md)
- [`ViFastOs-architecture-summary.txt`](../ViFastOs-architecture-summary.txt)
- [`docs/agent2-report-21.md`](agent2-report-21.md)
