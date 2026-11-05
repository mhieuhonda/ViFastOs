# ViFastOs — multi-architecture kernel

ViFastOs là một microkernel kiểu seL4, viết bằng Rust, với **lõi portable** dùng
chung cho nhiều kiến trúc và **lớp arch riêng** cho từng nền tảng.

Trạng thái hiện tại: **kernel core + 3 port QEMU đã boot được**, chạy self-test
capability/IPC/notification/scheduler, **timer interrupt + preemptive scheduler
SMP 2 hart** (RISC-V HSM, AArch64 PSCI, x86_64 AP qua ACPI MADT/LAPIC), ready
queue theo hart + work stealing, BKL ticket, maintenance IPI và **per-VA TLB
shootdown**. User mode có **root/server/client/fault TCB từ ELF Rust**, VSpace
riêng, IPC + chuyển capability, W^X, ASID/PCID, Untyped/retype có accounting,
software IOMMU, **VFS server user-space** với FS v3 journal, **hard link,
symlink, double-indirect, O_APPEND, block cache write-back data,
fsync/flush thật**, persistence 7 phase,
**Sleep deadline + BSP idle TCB**, **SchedContext budget/period**, `TcbRestart`,
`TcbBindNotification`, và **console input + shell nhỏ** (`ls`, `cat`, `stat`,
`sync`...). Có 49 syscall number (49 implemented, 0 placeholder). CNode phẳng
+ CNodeInstall, Timer IRQ
và UART IRQ dùng object IRQ + notification; shell block trên UART IRQ.

Đây vẫn chưa phải một hệ điều hành dùng được hàng ngày: chưa có driver
network/GPU, compositor, hardware IOMMU, CNode guard/nhiều cấp, ứng dụng
desktop hay formal verification. Phần lớn driver
hiện tại là QEMU-specific và userland vẫn là chương trình demo.

## Kiến trúc

```
                    ┌──────────────────────────────────────┐
                    │   user mode: ring3 / EL0 / U-mode      │
                    ├──────────────────────────────────────┤
                    │  vifast-kernel (portable, no_std)    │
                    │  object · capability · IPC · sched  │
                    │  generic walk · ramfs · ABI checks   │
                    ├──────────────────────────────────────┤
                    │  vifast-runtime (generic boot/syscall)│
                    ├──────────────────────────────────────┤
                    │  vifast-platform-* (BSP integration) │
                    │  trap · MMU · timer · IRQ · SMP      │
                    ├──────────────────────────────────────┤
                    │  vifast-arch-* (hardware primitives) │
                    │  UART · clock · IRQ mask · shutdown  │
                    ├──────────┬──────────┬────────────────┤
                    │ riscv64  │ aarch64  │ x86_64 (UEFI)  │
                    │ OpenSBI  │ QEMU virt│ OVMF           │
                    └──────────┴──────────┴────────────────┘
```

| Crate | Vai trò | `no_std` | `unsafe` |
|---|---|---|---|
| `vifast-abi` | Wire ABI: syscall number, error, quyền, message | có | cấm |
| `vifast-arch` | Trait `Arch`, `BootInfo`, primitive hardware contract | có | cấm |
| `vifast-kernel` | Object/capability/IPC/scheduler/VSpace/ramfs portable | có | cấm |
| `vifast-runtime` | Boot sequence, syscall dispatch, timer/preemption dùng chung | có | cấm |
| `vifast-user` | Syscall wrapper + user layout cho 3 arch | có | có (asm syscall) |
| `vifast-pte` | Encode/decode PTE Sv39/x86/AArch64 | có | cấm |
| `vifast-elf` | Parser ELF64 PT_LOAD, validation W^X/range/overlap | có | cấm |
| `vifast-root` | Root task Rust ELF, `_start` asm, linker ROOT_ELF_BASE | có | có (start asm) |
| `vifast-arch-riscv64` | UART 16550, `rdtime`, SBI shutdown | có | có (MMIO/asm) |
| `vifast-arch-aarch64` | PL011, `cntpct_el0`, PSCI shutdown | có | có (MMIO/asm) |
| `vifast-arch-x86_64` | COM1, `rdtsc`, QEMU debug-exit | có | có (port I/O) |
| `vifast-platform-riscv64` | Sv39, trap, SBI timer, HSM SMP | có | có (asm/MMIO) |
| `vifast-platform-aarch64` | 4-level MMU, GICv2, generic timer, PSCI SMP | có | có (asm/MMIO) |
| `vifast-platform-x86_64` | GDT/TSS/IDT, PML4 riêng, PIC/PIT, ring3 | có | có (asm/MMIO) |
| `vifast-kernel-riscv64` | Entry + linker + user image asset | có | có (entry/asset) |
| `vifast-kernel-aarch64` | Entry + linker + user image asset | có | có (entry/asset) |
| `vifast-kernel-x86_64-uefi` | UEFI entry + user image asset | có | có (entry/asset) |

Nguyên tắc: `vifast-kernel` **không** biết địa chỉ UART, thanh ghi, page size
thực tế, số cấp page table hay IRQ controller. Các chi tiết đó nằm ở
`vifast-arch-*` (primitive) và `vifast-platform-*` (BSP/MMU/trap/timer).
Binary `vifast-kernel-*` chỉ còn entry, linker và user image.

## Chạy

Yêu cầu:

- Rust stable **1.99.1** (bản mới nhất tại thời điểm cập nhật) + các target:
  ```bash
  rustup target add x86_64-unknown-none aarch64-unknown-none riscv64gc-unknown-none-elf
  ```
- QEMU: `qemu-system-riscv64`, `qemu-system-aarch64`, `qemu-system-x86_64`.
- Với x86_64: OVMF (`edk2-x86_64-code.fd` + `edk2-i386-vars.fd`). Script tự tìm
  các đường dẫn phổ biến trên macOS/Linux; có thể đặt `OVMF_CODE`.

Build tất cả:

```bash
./scripts/build.sh
```

Chạy self-test trên cả 3 kiến trúc:

```bash
./scripts/test-qemu.sh
```

Chạy toàn bộ gate cục bộ (format, host test, QEMU 3 lần/arch, persistence,
harness, docs):

```bash
./scripts/check-all.sh
```

Hoặc chạy riêng từng arch:

```bash
# RISC-V (OpenSBI + QEMU virt)
cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release
qemu-system-riscv64 -machine virt -m 128M -nographic -bios default \
  -kernel target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64

# AArch64 (QEMU virt)
cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release
qemu-system-aarch64 -machine virt,gic-version=2 -cpu max -m 128M -nographic \
  -kernel target/aarch64-unknown-none/release/vifast-kernel-aarch64

# x86_64 (UEFI/OVMF; xem scripts/test-qemu.py để có lệnh đầy đủ)
cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release
```

Kết quả mong đợi:

```
ViFastOs booting on riscv64
...
[ok]   install endpoint capability
...
SELFTEST PASS (ipc_tx=3 ipc_rx=2 ntfn=2 ctx=0 ticks=0)
entering user mode ...
Fault
USER FAULT ...
Zero OK
Server
Mapped
ViFastOs ramfs
ViFastOs write
Client
SLEEP OK
Cap OK
Reply OK
PREEMPT OK (ticks=20, switches=39)
```

## Self-test và user mode hiện có

Kernel chạy self-test trong kernel mode trước khi chuyển sang user mode:

- Cài capability endpoint.
- Copy capability với quyền giảm dần; kiểm tra không tăng quyền.
- Revoke xoá cap con, giữ cap gốc.
- Send/Recv đồng bộ: sender block khi chưa có receiver.
- Call/Reply: caller slot, reply một lần, reply lần hai bị từ chối.
- Notification: wait/signal, gộp word, poll.
- Chuyển capability qua IPC: kiểm tra quyền `GRANT`, cài vào slot receiver,
  không cho chuyển tiếp quyền `GRANT`.
- Untyped/retype: cấp frame và page-table page từ arena, kiểm tra hết arena.
- VSpace/Frame/PageTable: generic walk, PageTableMap level động, kiểm tra overlap, **W^X theo Frame** (không thể map RW một alias rồi RX alias khác), PA uniqueness và **sync I-cache khi map EXEC**.
- ramfs: mở/đọc `/motd`, tạo/ghi/seek/stat `/scratch`, đọc lại, báo
  `NotFound`, đóng fd.
- SMP bootstrap: RISC-V SBI HSM và AArch64 PSCI khởi động hart thứ hai; hart
  này hiện đứng idle, chưa nhận runqueue.
- Scheduler ưu tiên: chọn priority cao nhất, round-robin khi cùng mức.
- Generation của object table phát hiện `ObjRef` cũ.
- Kiểm tra biên slot capability.
- Negative test: cap sai type/đã revoke, W^X, slot cap đích đã dùng.
- Invariant checker cho object/capability/runqueue/VSpace/fd.

Sau đó runtime chỉ tạo **một root TCB**, nạp timer 100 Hz và chuyển sang user
mode. Root task tự tạo server/client:

- **RISC-V64**: `sret` → U-mode, `ecall`; timer SBI, `stvec`, context switch qua
  `TrapFrame`; trap dùng kernel trap stack qua `sscratch`.
- **AArch64**: `eret` → EL0, `svc #0`; GICv2 + generic timer PPI 30, vector IRQ
  riêng, trap dùng `SP_EL1`; page table 4 cấp.
- **x86_64**: UEFI/OVMF, GDT + TSS + IDT, `iretq` → ring3, `int 0x80`; PIC + PIT
  IRQ0, trap stack `TSS.RSP0`.

Root task nhận BootInfo qua arg0, tự `Retype` endpoint, hai TCB server/client,
bốn fault TCB, notification cho client và notification riêng, dựng VSpace
client riêng + cấp Untyped con, `CapCopy` cap cho server/client, `TcbConfigure`
entry/stack/VSpace rồi `TcbResume`. Server có cap endpoint
`READ`, gọi `Recv` rồi `Reply`; client có cap `WRITE`, gọi `Call` và nhận reply. Client còn giữ cap `Untyped`, gọi `Retype`
để tạo frame, rồi `FrameMap`/`FrameUnmap`; user ghi `Mapped\n` vào page vừa map
và đọc lại qua `DebugWrite`. Ở `Call` đầu tiên, client chuyển một cap
notification có quyền `GRANT` vào slot 4 của server; server nhận cap, `Signal`,
rồi `Reply`; các Call sau không chuyển cap nữa để slot không bị chiếm. Root
sau khi resume các TCB thì block bằng `Wait`. Fault thread cố ý load từ
`0x8000_0000`; kernel log `USER FAULT`, suspend thread và chạy tiếp. Client
`Poll` và in `Cap OK`. Client cũng mở `/motd`, đọc vào page vừa map, in nội dung qua
`DebugWrite`, tạo `/scratch`, ghi `ViFastOs write`, seek/đọc lại, rồi đóng fd
và unmap. Khi IPC block, kernel lưu context, chọn TCB
khác, rồi áp dụng `resume` message khi đánh thức. Timer preempts; binary test
bật feature `test-exit` để sau 20 tick in `PREEMPT OK` và thoát QEMU. Chạy host test:

```bash
cargo test -p vifast-kernel
cargo test -p vifast-pte
cargo test -p vifast-elf
# build.sh cũng build root ELF 3 target và chạy validator
```

## Những gì **chưa** có

Đây là danh sách trung thực để tránh hiểu nhầm “boot được” = “OS hoàn chỉnh”:

- Userland vẫn là chương trình demo (root/server/client/fault/worker/shell),
  chưa có libc, shell hệ thống đầy đủ, dynamic linking hay ứng dụng thật.
- CNode mới là bảng phẳng 64 slot (CNodeMint/CNodeInstall), chưa có guard,
  nhiều cấp hay zombie reaping. Timer IRQ và UART IRQ đã có
  object/notification/ack + platform routing trên cả 3 kiến trúc.
  SchedContext budget/period đã có object và scheduler thật.
- Chưa có hardware IOMMU (VT-d/SMMU); `IoMap`/`IoUnmap` chỉ là mô hình phần mềm.
- Chưa có network, GPU/framebuffer, compositor hay ViFastKey. Shell đã
  IRQ-driven cho input nhưng chưa có job control.
- FS còn giới hạn: ảnh 4 MiB, 64 inode, tên 28 byte, chưa có mount/pager
  hay barrier phần cứng thật; driver disk là
  VirtIO-MMIO/ATA PIO QEMU-specific.
- SMP mới test 2 hart (`MAX_HARTS = 4`); BSP có idle TCB, hart phụ vẫn dùng
  platform idle loop; chưa có khóa mịn/x2APIC.
- Object table cố định (144 object), chưa có allocator vật lý tổng quát hay
  free object ngoài đường revoke.
- x86_64 vẫn identity map 0..1 GiB; chưa có memory map đầy đủ.
- Chưa có CI (GitHub không dùng), formal verification/Kani/Verus; mới có
  invariant checker và seeded syscall fuzz.
- Tài liệu `01`–`07` và `legacy/`, `python-model/` là tham chiếu lịch sử, không
  phải trạng thái hiện tại.

Trạng thái chi tiết: `docs/00-hien-trang.md`. Tài liệu thành phần hiện tại:
[`docs/11-hien-tai/00-index.md`](docs/11-hien-tai/00-index.md). Các tài liệu
thiết kế cũ trong `docs/` và mô hình Python trong `python-model/` vẫn được giữ
làm tham chiếu lịch sử; chúng mô tả nhiều thành phần chưa được port sang bản
bare-metal này.

## Hướng phát triển

1. ~~Trap + user mode tối giản~~ — đã có.
2. ~~Timer interrupt + preemptive scheduler một hart~~ — đã có.
3. ~~MMU cơ bản~~ — đã có VSpace/PageTable/Frame object, map/unmap, W^X và
   flush TLB một hart. Còn TLB shootdown đa hart và page-table allocator tổng quát.
4. ~~SMP bootstrap~~ — RISC-V HSM/AArch64 PSCI đã khởi động hart phụ; còn
   x86_64 AP, BKL, per-hart runqueue, IPI scheduler và TLB shootdown.
5. ~~Untyped/retype cơ bản~~ — đã có frame/page-table page + accounting metadata;
   còn allocator vật lý đầy đủ và thu hồi object.
6. **Root task + vifast-user** — root Rust ELF đã được nạp vào runtime 6.4;
   còn client VSpace riêng, IPC buffer/shared memory; BootInfo và Untyped con đã xong.
7. **Driver framework** user space: VirtIO console/block/net.
8. **FS thật + block driver, name server, pager, supervisor, compositor,
   ViFastKey** — ramfs phẳng đã có.
9. **Formal/verification**: mở rộng invariant checker, fuzz, Kani/Verus.

## Ghi chú thiết kế

- Một kernel **nguồn** cho nhiều arch, nhưng **mỗi arch một binary**. Không có
  binary duy nhất chạy x86_64 + ARM + RISC-V.
- `vifast-kernel` là `#![forbid(unsafe_code)]` và không còn dependency
  `vifast-arch`; `unsafe` chỉ nằm trong arch/boot.
- Fixed-capacity object table (128 object, 16 TCB, 64 cap/TCB, 32 mapping/VSpace)
  để chứng minh lõi không cần heap; `Kernel` đặt trong static storage per port
  (khoảng 397 KiB), không đặt trên boot stack. Đây là reference implementation, chưa phải allocator cuối.
- Stack kernel hiện đặt 256 KiB trong linker script cho RISC-V/AArch64 vì
  `Kernel` + format machinery cần nhiều hơn 64 KiB. RISC-V dùng trap stack qua
  `sscratch`; AArch64 dùng `SP_EL1`, x86_64 dùng `TSS.RSP0`. Stack per-thread vẫn là
  việc tiếp theo.
- x86_64 tự dựng PML4 và page table 4 KiB cho user; kernel/driver vẫn chạy
  identity map 0..1 GiB. Chưa có AP/SMP scheduler.
- x86_64 dùng UEFI/OVMF vì QEMU `-kernel` không nhận ELF64 multiboot. Legacy
  multiboot crate đã được xoá khỏi workspace.
- QEMU x86 dùng `isa-debug-exit`; kernel ghi port `0x501`, QEMU thoát với mã 3.
  RISC-V dùng SBI SRST; AArch64 dùng PSCI SYSTEM_OFF.
