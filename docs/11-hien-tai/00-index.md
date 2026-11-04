# 11. Tài liệu thành phần hiện tại

Bộ tài liệu này thay thế các mô tả cũ trong `docs/01`–`docs/07` khi cần tra
cứu **bản bare-metal hiện tại**. Các tài liệu cũ vẫn được giữ để tham chiếu
lịch sử, nhưng nhiều chi tiết trong đó (CNode nhiều cấp, IOMMU, pager,
`vifast-sim`, profile manifest) **chưa được port** sang kernel này.

## Thứ tự đọc

1. [01-abi.md](01-abi.md) — wire ABI, syscall, mã lỗi, quy ước thanh ghi.
2. [02-objects-capabilities.md](02-objects-capabilities.md) — object table,
   capability, Untyped/retype.
3. [03-ipc-scheduler.md](03-ipc-scheduler.md) — endpoint, notification,
   chuyển capability, scheduler và preemption.
4. [04-memory-vspace.md](04-memory-vspace.md) — Frame/VSpace/PageTable, W^X,
   MMU từng kiến trúc.
5. [05-vfs.md](05-vfs.md) — ramfs, fd table và syscall file.
6. [06-runtime-platform.md](06-runtime-platform.md) — `vifast-runtime`,
   `Platform` trait và luồng syscall/timer.
7. [07-arch-ports.md](07-arch-ports.md) — RISC-V64, AArch64, x86_64/UEFI.
8. [08-boot-user-flow.md](08-boot-user-flow.md) — luồng boot và chương trình
   user mẫu.
9. [09-testing.md](09-testing.md) — test host, QEMU và tiêu chí PASS.
10. [10-vifast-user.md](10-vifast-user.md) — wrapper syscall và user layout chung.
11. [11-phase-3b-decisions.md](11-phase-3b-decisions.md) — quyết định VSpace riêng,
    BootInfo; PageTableMap đã triển khai, VSpace động còn lại.
12. [12-vifast-pte.md](12-vifast-pte.md) — encode/decode PTE an toàn cho 3 arch.

## Bản đồ crate

| Crate | Source chính | Trách nhiệm |
|---|---|---|
| `vifast-abi` | `crates/vifast-abi/src/lib.rs` | Kiểu dữ liệu wire, syscall, lỗi, quyền |
| `vifast-arch` | `crates/vifast-arch/src/lib.rs` | Trait `Arch`, primitive phần cứng |
| `vifast-kernel` | `crates/vifast-kernel/src/` | Object, cap, IPC, scheduler, generic page-table walk, ramfs |
| `vifast-runtime` | `crates/vifast-runtime/src/lib.rs` | Boot, syscall dispatch, timer, context switch dùng chung |
| `vifast-user` | `crates/vifast-user/src/lib.rs` | Syscall wrapper cho 3 kiến trúc |
| `vifast-pte` | `crates/vifast-pte/src/lib.rs` | Encode/decode/index PTE an toàn |
| `vifast-arch-*` | `crates/vifast-arch-*/src/lib.rs` | UART/PL011/COM1, clock, IRQ mask, shutdown |
| `vifast-platform-*` | `crates/vifast-platform-*/src/` | MMU, trap, timer, IRQ, SMP, user entry |
| `vifast-kernel-*` | `crates/vifast-kernel-*/src/main.rs` | Entry, linker, user image asset |

## Quy tắc kiến trúc

- `vifast-kernel` và `vifast-runtime` là `#![no_std]` và
  `#![forbid(unsafe_code)]`.
- `vifast-arch` chỉ chứa trait và primitive; không phụ thuộc kernel.
- `vifast-platform-*` là lớp BSP duy nhất được phép dùng `asm!`, MMIO, đọc
  thanh ghi hệ thống và định nghĩa trap frame.
- `vifast-kernel-*` chỉ còn entry assembly, linker script và user image.
