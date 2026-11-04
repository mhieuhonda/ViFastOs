# 00. Hiện trạng triển khai (2026-11)

Tài liệu này là nguồn trạng thái hiện tại. Các tài liệu `01`–`07` là thiết kế
tham chiếu từ mô hình Python ban đầu; một số chi tiết trong đó chưa được port
sang bản bare-metal.

## Đã chạy được

| Hạng mục | RISC-V64 | AArch64 | x86_64 |
|---|---|---|---|
| Boot QEMU | OpenSBI | QEMU `virt` | OVMF/UEFI |
| Console sớm | 16550 | PL011 | COM1 |
| Self-test portable | PASS | PASS | PASS |
| User mode | U-mode `sret` | EL0 `eret` | ring3 `iretq` |
| Syscall vào kernel | `ecall` | `svc #0` | `int 0x80` |
| User chạy `DebugWrite` | có | có | có |
| Timer interrupt | SBI timer | GICv2 + generic timer | PIC + PIT |
| Preemptive root + server/client + fault | có | có | có |
| User IPC Call/Reply | có | có | có |
| User `Retype` + `FrameMap/Unmap` | có | có | có |
| ramfs VFS + file syscall | có | có | có |
| SMP bootstrap | HSM, hart phụ idle | PSCI, hart phụ idle | chưa AP; hướng ACPI MADT + LAPIC/SIPI |
| W^X user code/data | có (RX + NX stack) | có (RO+X, NX data) | có (RX + NX data) |
| Chuyển capability + notification qua IPC | có | có | có |
| User fault suspend (không panic) | có | có | có |
| Frame retype zero page | có | có | có |
| User gọi `Exit` | có | có | có |

Kiến trúc hiện tại: `vifast-kernel` portable + `vifast-runtime` generic +
`vifast-platform-*` BSP + `vifast-arch-*` hardware primitive + binary
`vifast-kernel-*` mỏng.

Kernel portable hiện có:

- object table cố định 128 object, 64 cap/TCB, 32 mapping/VSpace; `Kernel`
  khoảng 397 KiB đặt trong static storage của platform;
- capability install/copy/derive/revoke, chuyển cap qua IPC với quyền `GRANT`;
- endpoint Send/Recv, Call/Reply một caller;
- notification wait/signal/poll;
- scheduler ưu tiên + round-robin;
- invariant checker cho object/capability/runqueue/VSpace/fd;
- `Frame`, `VSpace`, `PageTable` object; map/unmap, overlap check, W^X, shadow
  mapping table;
- chuyển capability qua `Send`/`Call`, cài vào slot receiver, kiểm tra `GRANT`
  và không chuyển tiếp quyền `GRANT`;
- `Untyped`/`retype` cho Frame/PageTable và metadata object, có tính phí
  Endpoint/Notification/Tcb; hết Untyped trả `OutOfMemory`;
- root task nhận VSpace/Untyped và tự tạo server/client bằng Retype/CapCopy/
  TcbConfigure/TcbResume;
- `validate_user_range()` cho mọi syscall có buffer user;
- ramfs phẳng trong kernel, fd table theo TCB, syscall `Open`/`Read`/`Write`/
  `Close`/`Seek`/`Stat`, tạo file bằng `O_CREATE`;
- SMP bootstrap: RISC-V SBI HSM, AArch64 PSCI; hart phụ hiện idle;
- syscall user: IPC, `Retype`, `CapCopy`, `TcbConfigure`, `TcbResume`,
  `FrameMap`, `FrameUnmap`, `Open`, `Read`, `Write`, `Close`, `Seek`, `Stat`,
  `DebugWrite`, `GetTime`, `Exit`;
- timer interrupt + preemptive scheduler một hart; root task tạo server/client
  và hai fault thread;
- IPC user subset `Send/Call/Recv/Reply/Signal/Wait/Poll`.

## Chưa có

- TLB shootdown đa hart, page-table page allocator tổng quát, range VA allocator.
- SMP scheduler: x86_64 AP bring-up, BKL, per-hart runqueue, IPI, timer đa hart.
- Revoke Untyped đã destroy object con; còn allocator vật lý đầy đủ, free toàn
  bộ object ngoài revoke và IOMMU.
- `vifast-user` mới có syscall wrapper, chưa thay chương trình assembly;
  chưa có ELF loader, boot info page, ABI encode/decode đầy đủ, shared memory.
- VFS đầy đủ (block device, cây thư mục, quyền, mount, pager, FS bền vững).
- Driver, mạng, GPU, compositor, ViFastKey.
- Kiểm chứng hình thức, fuzz, invariant checker mở rộng/định kỳ.

## Hạn chế bring-up cần nhớ

- RISC-V trap đã dùng kernel trap stack qua `sscratch`; AArch64 dùng `SP_EL1`,
  x86_64 dùng `TSS.RSP0`.
- x86_64 tự dựng PML4 + page table 4 KiB; code RX, data/stack NX, unmap thật.
  Kernel chạy identity map 0..1 GiB; AP/SMP scheduler chưa có.
- AArch64 dùng page table tĩnh 4 cấp; user code read-only+executable, stack
  read-write + NX; PAN nếu CPU hỗ trợ.
- RISC-V và AArch64 flush TLB một hart (`sfence.vma`/`tlbi`); chưa có IPI
  shootdown. Page-table root vẫn là bảng tĩnh của port.
- Hart phụ RISC-V/AArch64 mới chạy vòng idle, không vào scheduler; x86_64 chưa
  khởi động AP; hướng là ACPI MADT + LAPIC/SIPI.
- Chương trình user là mã nhúng; `vifast-user` mới là syscall wrapper.
- Server/client/root dùng chung một VSpace và stack window; chưa cách ly bộ nhớ
  giữa các task.
- User layout chung 64 GiB trong `vifast-abi`; x86 identity map 0..1 GiB.
- DebugWrite/Open/Read/Write kiểm tra buffer user; kernel address bị từ chối.

## Lệnh kiểm tra

```bash
./scripts/build.sh
python3 scripts/test-qemu.py
```

Kỳ vọng: cả 3 dòng `riscv64`, `aarch64`, `x86_64` đều `PASS`, mỗi log có
`SELFTEST PASS`, `Server`, `Mapped`, `ViFastOS ramfs`, `ViFastOS write`,
`Client`, `Fault`, `USER FAULT`, `Zero OK`, `Cap OK`, `Reply OK` và `PREEMPT OK`.

## Tài liệu thành phần

Chi tiết từng crate, API, luồng syscall, MMU, VFS và kiểm thử:

- [`docs/11-hien-tai/00-index.md`](11-hien-tai/00-index.md)
