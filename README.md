# ViFastOs — multi-architecture kernel

ViFastOs là một microkernel kiểu seL4, viết bằng Rust, với **lõi portable** dùng
chung cho nhiều kiến trúc và **lớp arch riêng** cho từng nền tảng.

Trạng thái hiện tại: **kernel core + 3 port QEMU đã boot được**, chạy self-test
capability/IPC/notification/scheduler, **timer interrupt + preemptive scheduler**
với **root task tự tạo server/client bằng Retype/CapCopy/TcbConfigure/TcbResume**,
**IPC từ user space**, **Frame/VSpace map–unmap có W^X**, **untyped/retype có
accounting**, chuyển capability + notification qua IPC, **ramfs VFS** với syscall
`Open`/`Read`/`Write`/`Close`/`Seek`/`Stat`, **kiểm tra con trỏ user** trong lõi
portable, và **SMP bootstrap** (RISC-V HSM / AArch64 PSCI; x86_64 AP còn chờ)
cùng `DebugWrite` / `GetTime` / `Exit`.

Đây vẫn chưa phải một hệ điều hành dùng được hàng ngày: scheduler vẫn một
hart, chưa có TLB shootdown đa hart, x86_64 chưa khởi động AP, chưa có driver,
mạng, compositor hay ứng dụng. Chương trình user
hiện vẫn là assembly nhúng; crate `vifast-user` đã có syscall wrapper nhưng
chưa thay thế chương trình mẫu.

## Kiến trúc

```
                    ┌──────────────────────────────────────┐
                    │   user mode: ring3 / EL0 / U-mode      │
                    ├──────────────────────────────────────┤
                    │  vifast-kernel (portable, no_std)    │
                    │  object · capability · IPC · sched  │
                    │  VSpace shadow · ramfs · ABI checks  │
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
ViFastOS booting on riscv64
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
ViFastOS ramfs
ViFastOS write
Client
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
- VSpace/Frame: map/unmap, kiểm tra overlap và **W^X**.
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

Root task nhận VSpace + metadata/data Untyped ở slot 1/2/3, tự `Retype`
endpoint, hai TCB và notification, `CapCopy` cap cho server/client,
`TcbConfigure` entry/stack/VSpace rồi `TcbResume`. Server có cap endpoint
`READ`, gọi `Recv` rồi `Reply`; client có cap `WRITE`, gọi `Call` và nhận reply. Client còn giữ cap `Untyped`, gọi `Retype`
để tạo frame, rồi `FrameMap`/`FrameUnmap`; user ghi `Mapped\n` vào page vừa map
và đọc lại qua `DebugWrite`. Ở `Call` đầu tiên, client chuyển một cap
notification có quyền `GRANT` vào slot 4 của server; server nhận cap, `Signal`,
rồi `Reply`; các Call sau không chuyển cap nữa để slot không bị chiếm. Root
sau khi resume các TCB thì block bằng `Wait`. Fault thread cố ý load từ
`0x8000_0000`; kernel log `USER FAULT`, suspend thread và chạy tiếp. Client
`Poll` và in `Cap OK`. Client cũng mở `/motd`, đọc vào page vừa map, in nội dung qua
`DebugWrite`, tạo `/scratch`, ghi `ViFastOS write`, seek/đọc lại, rồi đóng fd
và unmap. Khi IPC block, kernel lưu context, chọn TCB
khác, rồi áp dụng `resume` message khi đánh thức. Timer preempts; binary test
bật feature `test-exit` để sau 20 tick in `PREEMPT OK` và thoát QEMU. Chạy host test:

```bash
cargo test -p vifast-kernel
```

## Những gì **chưa** có

Đây là danh sách trung thực để tránh hiểu nhầm “boot được” = “OS hoàn chỉnh”:

- User mode vẫn là **bring-up tối giản**, chưa phải userland đầy đủ; đã có
  crate `vifast-user` với syscall wrapper 3 kiến trúc, nhưng chương trình mẫu
  vẫn là assembly; chưa có ELF loader.
- IPC user có subset `Send/Call/Recv/Reply/Signal/Wait/Poll`; chuyển cap đã có
  end-to-end cho `Send`/`Call` (không cho `Reply`), chưa có shared memory,
  reply object/ReplyRecv, IPC buffer hay fault endpoint.
- VSpace/PageTable object và map/unmap đã có, nhưng page-table root vẫn là bảng
  tĩnh của từng port; root/server/client hiện dùng chung VSpace và stack window,
  nên chưa cách ly bộ nhớ giữa các task; chưa có page-table page allocator tổng quát, TLB shootdown
  đa hart, hay cấp phát địa chỉ ảo tự động theo range lớn.
- Untyped/retype đã tính phí Endpoint/Notification/Tcb và reset bump khi revoke
  Untyped; chưa có allocator vật lý đầy đủ, IOMMU hay thu hồi object.
- RISC-V dùng kernel trap stack qua `sscratch` và `sscratch=0` ở kernel;
  AArch64 dùng `SP_EL1` + PAN nếu CPU hỗ trợ; x86_64 dùng `TSS.RSP0` +
  SMEP/SMAP nếu CPUID hỗ trợ. Copy user bật SUM/AC/PAN tạm thời.
- SMP mới ở mức bootstrap: RISC-V/AArch64 có hart thứ hai idle; x86_64 chưa
  khởi động AP. Hướng x86 là ACPI MADT + LAPIC/SIPI, không dùng UEFI MP
  Services sau ExitBootServices. Chưa có per-hart runqueue, BKL, IPI scheduler
  hay TLB shootdown đa hart.
- Timer/scheduler vẫn chạy một hart qua SBI/GICv2/PIC.
- x86_64 đã tự dựng PML4/page table riêng: code RX, data/stack NX, unmap thật.
  User window 64 GiB nằm ngoài identity map 0..1 GiB; chưa có AP và data kernel
  chưa được đặt NX.
- RISC-V/AArch64/x86_64 đều map code read-only + executable và data/stack NX.
- VFS mới là ramfs phẳng trong kernel: chưa có block device, cây thư mục,
  quyền file, mount, pager hay filesystem bền vững.
- Chưa có driver, network, GPU, compositor, ViFastKey.
- Đã có invariant checker cơ bản; chưa có kiểm chứng hình thức, fuzz diện rộng
  hay Kani/Verus.

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
6. **Root task + vifast-user** — root task đã có; crate wrapper đã có; còn
   chuyển user mẫu sang Rust, ELF loader, IPC buffer/shared memory.
7. **Driver framework** user space: VirtIO console/block/net.
8. **FS thật + block driver, name server, pager, supervisor, compositor,
   ViFastKey** — ramfs phẳng đã có.
9. **Formal/verification**: mở rộng invariant checker, fuzz, Kani/Verus.

## Ghi chú thiết kế

- Một kernel **nguồn** cho nhiều arch, nhưng **mỗi arch một binary**. Không có
  binary duy nhất chạy x86_64 + ARM + RISC-V.
- `vifast-kernel` là `#![forbid(unsafe_code)]` và không còn dependency
  `vifast-arch`; `unsafe` chỉ nằm trong arch/boot.
- Fixed-capacity object table (32 object, 8 TCB, 12 cap/TCB) để chứng minh lõi
  không cần heap. Đây là reference implementation, chưa phải allocator cuối.
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
