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
5. `Kernel::new` + self-test trên kernel riêng.
6. `Platform::start_smp`.
7. `Kernel::new` sạch; tạo VSpace + metadata/data Untyped.
8. cài cap slot 1/2/3 cho root.
9. copy user image.
10. build page table.
11. `prepare_user` (x86).
12. tạo context root.
13. set kernel pointer.
14. enqueue root, pick root.
15. start timer.
16. `enter_user(root)`.

## 3. User image

Binary kernel chứa một `user.S`:

- `user_root`: root task tạo TCB/endpoint/notification bằng syscall.
- `user_thread_a`: server IPC.
- `user_thread_b`: client.

`UserImage` chứa:

```rust
pub struct UserImage {
    pub code: &'static [u8],
    pub entry_a: usize,
    pub entry_b: usize,
    pub entry_root: usize,
}
```

Runtime chỉ dùng `entry_root`; root task tự lấy địa chỉ server/client bằng
label PC-relative. Các message nằm ngay trong code page.

Root task nhận cap:
- slot 1 VSpace;
- slot 2 metadata Untyped;
- slot 3 data Untyped.

Root lần lượt Retype endpoint/2 TCB/notification/2 fault TCB, CapCopy cho
server/client, TcbConfigure entry/stack/VSpace/priority rồi TcbResume. Sau đó
root block bằng `Wait` trên notification riêng; server/client/fault chạy theo
scheduler.

## 4. Root task

```text
Retype metadata Untyped → Endpoint slot 4
Retype metadata Untyped → TCB server slot 5
Retype metadata Untyped → TCB client slot 6
Retype metadata Untyped → Notification slot 7
CapCopy endpoint → server slot 1 READ, client slot 1 WRITE
CapCopy VSpace → client slot 2 WRITE
CapCopy data Untyped → client slot 3 WRITE
CapCopy notification → client slot 5 READ|WRITE|GRANT
TcbConfigure server/client (entry, stack, VSpace slot 1, priority 5)
TcbResume server/client
Retype fault TCB ×2; TcbConfigure/TcbResume (priority 6)
Retype root notification; Wait(root notification)
```

Fault TCB thứ nhất cố ý load từ kernel VA; fault TCB thứ hai chạy
`ud2`/`hlt`/illegal instruction. Cả hai log `USER FAULT`, bị suspend và
server/client vẫn chạy. Không panic kernel.

## 5. Server A

```text
DebugWrite "Server"
loop:
  Recv(endpoint, recv_slot=4)
  Signal(notification cap slot 4)
  Reply(label=0x55, data=42)
```

## 6. Client B

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
  Call đầu tiên: endpoint, label=0x1234, data=7, cap=notification slot 5
  các Call sau: cùng label/data nhưng không chuyển cap
  Poll(notification slot 5)
  DebugWrite "Cap OK" / "Cap BAD"
  DebugWrite "Reply OK" / "Reply BAD"
```

## 7. Syscall flow

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

## 8. Blocking flow

```text
Recv không có sender
  → TCB BlockedRecv
  → runtime switch_blocked
  → pick TCB khác
  → khi sender đến, resume message được gắn
  → runtime nạp context + write_message
```

## 9. Timer flow

```text
timer IRQ
  → platform ack
  → runtime on_timer
  → save context
  → kernel.tick
  → yield/pick
  → restore context
  → sau 20 tick: PREEMPT OK + exit (chỉ feature test-exit)
```
