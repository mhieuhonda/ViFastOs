# 11.8. Luồng boot và chương trình user mẫu

## 1. Boot

### RISC-V

```text
OpenSBI
  → _start (boot.S)
  → rust_main(hartid, dtb)
  → vifast_runtime::boot::<RiscvPlatform>
```

### AArch64

```text
QEMU virt
  → _start (boot.S)
  → hạ EL2 → EL1 nếu cần
  → rust_main(dtb)
  → runtime boot
```

### x86_64

```text
OVMF
  → EFI application efi_main(image, system_table)
  → runtime boot
```

## 2. Runtime boot

1. `Arch::init` — console/clock.
2. `irq_disable`.
3. `Platform::init_platform`.
4. log banner.
5. `Kernel::new` + self-test.
6. `Platform::start_smp`.
7. cấp object/cap.
8. copy user image.
9. build page table.
10. `prepare_user` (x86).
11. tạo context.
12. set kernel pointer.
13. enqueue 2 TCB, pick TCB A.
14. start timer.
15. `enter_user(A)`.

## 3. User image

Binary kernel chứa một `user.S`:

- `user_thread_a`: server IPC.
- `user_thread_b`: client.

`UserImage` chứa:

```rust
pub struct UserImage {
    pub code: &'static [u8],
    pub entry_a: usize, // offset
    pub entry_b: usize, // offset
}
```

Runtime copy toàn bộ code vào `Platform::code_page()`. Các message nằm ngay
trong code page để không cần copy bảng rodata riêng.

## 4. Server A

```text
DebugWrite "Server"
loop:
  Recv(endpoint, recv_slot=4)
  Signal(notification cap slot 4)
  Reply(label=0x55, data=42)
```

## 5. Client B

```text
Retype Untyped → Frame cap
FrameMap
write "Mapped" to mapped page
DebugWrite "Mapped"

Open("/motd"), Read, DebugWrite file content
Open("/scratch", O_CREATE|O_RDWR)
Write("ViFastOS write\n"), Seek(0), Read, DebugWrite
Close
FrameUnmap

DebugWrite "Client"
loop:
  Call(endpoint, label=0x1234, data=7, cap=notification slot 5)
  Poll(notification slot 5)
  DebugWrite "Cap OK" / "Cap BAD"
  DebugWrite "Reply OK" / "Reply BAD"
```

## 6. Syscall flow

```text
user instruction
  → trap entry (platform asm)
  → platform trap handler
  → runtime::syscall::dispatch
  → Kernel::user_syscall
  → UserAction
  → runtime/platform action
  → write registers
  → return to user
```

## 7. Blocking flow

```text
Recv không có sender
  → TCB BlockedRecv
  → runtime switch_blocked
  → pick TCB khác
  → khi sender đến, resume message được gắn
  → runtime nạp context + write_message
```

## 8. Timer flow

```text
timer IRQ
  → platform ack
  → runtime on_timer
  → save context
  → kernel.tick
  → yield/pick
  → restore context
  → sau 20 tick: PREEMPT OK + exit
```
