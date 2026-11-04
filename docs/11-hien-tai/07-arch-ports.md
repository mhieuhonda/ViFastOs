# 11.7. Các port kiến trúc

## 1. Bảng tổng hợp

| Thành phần | RISC-V64 | AArch64 | x86_64 |
|---|---|---|---|
| Target | `riscv64gc-unknown-none-elf` | `aarch64-unknown-none` | `x86_64-unknown-uefi` |
| Boot | OpenSBI `-kernel` | QEMU `virt -kernel` | OVMF UEFI app |
| Console | 16550 UART `0x1000_0000` | PL011 `0x0900_0000` | COM1 `0x3F8` |
| Clock | `time` CSR | `cntpct_el0` | `rdtsc` |
| Timer | SBI timer | GICv2 PPI 30 + generic timer | PIC + PIT |
| MMU | Sv39 tĩnh | 4-level tĩnh | UEFI page table |
| Trap | `stvec` + `TrapFrame` | vector table + `SP_EL1` | IDT + TSS.RSP0 |
| Syscall | `ecall` | `svc #0` | `int 0x80` |
| User transition | `sret` → U | `eret` → EL0 | `iretq` → ring3 |
| SMP bootstrap | SBI HSM | PSCI CPU_ON | `SMP SKIP` |
| Shutdown | SBI SRST | PSCI SYSTEM_OFF | `isa-debug-exit` |
| Exit code test | 0 | 0 | 3 |

## 2. RISC-V64

Files:

- `crates/vifast-arch-riscv64/src/lib.rs`: UART, `rdtime`, IRQ mask, SBI exit.
- `crates/vifast-platform-riscv64/src/platform.rs`: Sv39, trap, timer, SMP.
- `crates/vifast-platform-riscv64/src/trap.S`: trap entry/restore.
- `crates/vifast-platform-riscv64/src/smp.S`: secondary entry + stack.
- `crates/vifast-kernel-riscv64/src/boot.S`: primary entry.
- `crates/vifast-kernel-riscv64/src/user.S`: user image.

Trap:

- Timer cause `0x800...5` → `runtime::timer::on_timer`.
- `ecall` cause `8` → lưu `sepc + 4`, dispatch syscall.
- Cause khác → log và exit.
- Trap dùng kernel trap stack qua `sscratch`; user stack chỉ ở U-mode.

## 3. AArch64

Files:

- `crates/vifast-arch-aarch64/src/lib.rs`: PL011, `cntpct_el0`, PSCI
  shutdown.
- `crates/vifast-platform-aarch64/src/platform.rs`: 4-level MMU, GICv2,
  generic timer, PSCI SMP.
- `trap.S`: vector table, save/restore, IRQ lower.
- `smp.S`: secondary entry.
- Binary: `boot.S` (EL2→EL1) + `user.S`.

Trap:

- `sync_lower` → `aarch64_trap_handler`; `EC=0x15` là SVC.
- `irq_lower` → đọc GIC IAR, xử lý timer, EOI.
- Trap stack là `SP_EL1`; user stack là `SP_EL0`.
- Code user read-only + executable; stack/data RW + NX.

## 4. x86_64 UEFI

Files:

- `crates/vifast-arch-x86_64/src/lib.rs`: COM1, `rdtsc`, debug-exit.
- `crates/vifast-platform-x86_64/src/platform.rs`: GDT/TSS/IDT, PIC/PIT,
  PML4/page table riêng, ring3. AP bootstrap hiện chưa có code hoạt động.
- `trap.S`: `int80_entry`, `irq0_entry`, reload segments.
- Binary: UEFI entry + `user.S`.

Đặc điểm:

- `init_platform` chỉ lưu thông tin; `prepare_user` mới cài GDT/TSS/IDT.
- `start_timer` cài PIC/PIT.
- `build_page_tables` dựng PML4/PDPT/PD/PT 4 KiB; `enter_user` chuyển CR3
  sang bảng riêng, không sửa page table UEFI.
- Code RX, data/stack NX, unmap thật.
- AP bootstrap qua UEFI MP Services đã được thử nhưng OVMF hiện tại không trả
  interface; code MP đã được gỡ khỏi build để tránh làm hỏng firmware. Vì vậy
  `start_smp` chỉ in `SMP SKIP`.

## 5. Thêm kiến trúc mới

1. Tạo `vifast-arch-<arch>` với `Arch` primitive.
2. Tạo `vifast-platform-<arch>` implement `Platform`.
3. Tạo `vifast-kernel-<arch>` entry/linker/user image.
4. Thêm vào workspace, build script, test script.
5. Không thêm `cfg(target_arch)` vào kernel/runtime.
