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
7. `Kernel::new` sạch; tạo boot Untyped 128 KiB/32 object + metadata/data Untyped.
8. cài metadata/data Untyped slot 2/3.
9. `build_task_space` dựng root VSpace (code user.S RX, không stack/data);
   map 4 stack page fault/client; parse root ELF và build_task_space_elf(Some)
   thêm root ELF + stack Rust 4 trang + guard; cài VSpace slot 1.
10. dựng server VSpace (code dùng chung Frame root, stack +0x2000), cài slot 12.
11. `check_kernel_mappings` root+server; log boot Untyped sau server.
12. `prepare_user` (x86).
13. tạo context root.
14. set kernel pointer.
15. enqueue root, pick root, `ensure_current_root` -> ROOT SWITCH 1.
16. start timer.
17. `enter_user(root)`.

## 3. User image

Binary kernel chứa một `user.S`:

- Jump table `USER_ENTRY_*`: server/client/fault1/2/3.
- `user_thread_a`: server IPC.
- `user_thread_b`: client.
- Root task là Rust ELF riêng (`vifast-root`), không còn trong user.S.

`UserImage` chứa:

```rust
pub struct UserImage {
    pub code: &'static [u8],
    pub entry_a: usize,
    pub entry_b: usize,
    pub root_elf: &'static [u8],
}
```

Runtime parse `root_elf` trong cửa sổ 1 MiB và lấy entry từ ELF;
server/client/fault dùng jump table `USER_ENTRY_*`. Builder copy image
vào Frame lấy từ boot Untyped; các message nằm trong code page RX.
Root task nhận cap:
- slot 1 root VSpace;
- slot 2 metadata Untyped;
- slot 3 data Untyped;
- slot 12 server VSpace.

Root lần lượt Retype endpoint/2 TCB/notification/3 fault TCB, CapCopy cho
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
TcbConfigure server (entry, stack 0x1000003000, VSpace slot 12)
TcbConfigure client (entry, stack 0x1000009000, VSpace slot 1)
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
FrameUnmap(VSpace slot 2, VA, Frame slot 4)

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
