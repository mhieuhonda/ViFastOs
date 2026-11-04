# 10. Kiến trúc đa nền tảng (x86_64 / AArch64 / RISC-V64)

Tài liệu này mô tả cách bản bare-metal hiện tại tách lõi kernel khỏi kiến trúc,
những gì đã chạy được và những gì còn thiếu.

## 1. Ranh giới

```
vifast-abi  (wire ABI)
   ↑
vifast-arch (trait Arch + primitive hardware)
   ↑                         ↑
vifast-kernel (logic)   vifast-runtime (generic boot/syscall/timer)
                              ↑
                    vifast-platform-*  (BSP: trap, MMU, timer, IRQ, SMP)
                              ↑
                    vifast-arch-*      (UART, clock, shutdown)
                              ↑
                    vifast-kernel-*    (entry, linker, user image)
```

- `vifast-kernel` không biết:
  - địa chỉ UART/PL011/COM1;
  - page size thực tế, số cấp page table;
  - trap vector, thanh ghi syscall;
  - timer, interrupt controller, IPI, SMP bring-up.
- `vifast-arch-*` chỉ còn primitive phần cứng: console UART, clock, IRQ
  mask, shutdown, port I/O.
- `vifast-platform-*` sở hữu trap frame, page table, TLB flush, timer/GIC/PIC/
  PIT, user entry và SMP. Nó implement `vifast_runtime::Platform`.
- `vifast-runtime` sở hữu boot sequence, user image copy, syscall action
  dispatch, context switch, timer tick dùng chung.
- Binary `vifast-kernel-*` chỉ còn entry assembly, linker script và user image;
  hiện khoảng 48–49 dòng `main.rs` mỗi port.

## 2. Vì sao không dùng một binary duy nhất?

Ba ISA có instruction set, trap model, page table format và boot protocol khác
nhau. Một kernel binary chứa mã máy của một ISA không thể thực thi trên ISA khác
nếu không có VM/emulation. Vì vậy:

- **Cùng mã nguồn**, **biên dịch riêng** cho từng target.
- ABI syscall/ngữ nghĩa object có thể giống nhau; user app vẫn phải recompile.
- Nếu cần app portable: dùng WebAssembly/bytecode hoặc Linux VM.

## 3. Bảng port hiện tại

| Thành phần | RISC-V64 | AArch64 | x86_64 |
|---|---|---|---|
| Target Rust | `riscv64gc-unknown-none-elf` | `aarch64-unknown-none` | `x86_64-unknown-uefi` |
| Boot | OpenSBI `-kernel` | QEMU `virt -kernel` | OVMF UEFI app |
| Load address | `0x8020_0000` | `0x4008_0000` | do UEFI |
| Console | 16550 UART `0x1000_0000` | PL011 `0x0900_0000` | COM1 `0x3F8` |
| Clock | `time` CSR | `cntpct_el0` | `rdtsc` |
| Shutdown | SBI SRST | PSCI SYSTEM_OFF | `isa-debug-exit` |
| User entry | `sret` → U-mode | `eret` → EL0 | `iretq` → ring3 |
| Syscall | `ecall` | `svc #0` | `int 0x80` |
| Timer | SBI timer | GICv2 + generic timer | PIC + PIT |
| Preemption | root + server/client/fault | root + server/client/fault | root + server/client/fault |
| User IPC | Call/Reply | Call/Reply | Call/Reply |
| User frame map/unmap | một page, Sv39 | một page, 4-level | một page, PML4 4 KiB |
| W^X | code RX, stack NX | code RO+X, data NX | code RX, data/stack NX |
| Untyped/retype | Frame/PageTable/metadata có charge | Frame/PageTable/metadata có charge | Frame/PageTable/metadata có charge |
| Cap transfer user | Send/Call + Signal | Send/Call + Signal | Send/Call + Signal |
| ramfs + file syscall | có | có | có |
| SMP bootstrap | HSM | PSCI | chưa AP; ACPI MADT + LAPIC/SIPI |
| User page table | Sv39 tĩnh | 4-level tĩnh | PML4/PT 4 KiB riêng |
| Trap stack | kernel stack qua `sscratch` | `SP_EL1` | TSS.RSP0 |
| Stack | linker `.stack` 256 KiB | linker `.stack` 256 KiB | UEFI stack |
| Exit code test | 0 | 0 | 3 |

## 4. Những gì đã kiểm chứng

- `cargo test -p vifast-kernel`: 44 test portable trên host.
- `cargo test -p vifast-pte`: 9 test host cho cả 3 format PTE.
- `cargo test -p vifast-user`: wrapper compile.
- QEMU RISC-V64 (`-smp 2`): self-test + U-mode IPC + frame map/unmap + SMP
  bootstrap + timer preemption.
- QEMU AArch64 (`-smp 2`, `gic-version=2`): self-test + EL0 IPC + frame
  map/unmap + SMP bootstrap + timer.
- QEMU x86_64/OVMF (`-smp 2`): self-test + ring3 IPC + frame map/unmap + PIC/PIT;
  AP bring-up chưa có.
- `scripts/test-qemu.py` chạy lặp lại và kiểm tra `SELFTEST PASS`, banner,
  `Server`, `Mapped`, `ViFastOS ramfs`, `ViFastOS write`, `Client`, `Fault`,
  `USER FAULT`, `Zero OK`, `Cap OK`, `Reply OK`, `PREEMPT OK`.

## 5. Những gì cần thêm để thành OS

Thứ tự đề xuất:

1. ~~Trap entry/exit + user mode tối giản~~ — đã có.
2. ~~Timer + preemptive scheduling một hart~~ — đã có.
3. ~~MMU cơ bản~~ — đã có VSpace/PageTable/Frame object, map/unmap, W^X và
   flush TLB một hart; còn page-table allocator và shootdown đa hart.
4. ~~SMP bootstrap~~ — có HSM/PSCI; còn x86 AP, IPI, per-hart runqueue,
   spinlock/BKL và TLB shootdown.
5. ~~Untyped/retype cơ bản~~ — đã có Frame/PageTable page; còn allocator vật lý
   đầy đủ và thu hồi object.
6. **vifast-user + ABI wire format** — wrapper đã có; còn root task Rust,
   ELF loader, IPC buffer/shared memory.
7. **VFS đầy đủ, block driver, net, compositor, ViFastKey** — ramfs đã có.

Mỗi bước phải chạy cùng bộ test trên cả ba arch trước khi coi là “hỗ trợ”.

## 6. Quy tắc khi thêm arch mới

1. Không thêm `cfg(target_arch)` vào `vifast-kernel`.
2. Thêm crate `vifast-arch-<arch>` cho primitive phần cứng.
3. Thêm crate `vifast-platform-<arch>` cài `Arch` + `Platform` + trap/MMU/timer.
4. Thêm crate `vifast-kernel-<arch>` với entry/linker/user asset riêng.
5. Thêm vào `scripts/build.sh` và `scripts/test-qemu.py`.
6. Cập nhật bảng trong tài liệu này.
7. Nếu là Space: cân nhắc chỉ hỗ trợ một arch để giảm phạm vi chứng nhận.

## 7. Tài liệu thành phần hiện tại

Xem [`docs/11-hien-tai/00-index.md`](11-hien-tai/00-index.md) để tra cứu chi tiết
ABI, object/capability, IPC/scheduler, VSpace, VFS, runtime/platform và test.
