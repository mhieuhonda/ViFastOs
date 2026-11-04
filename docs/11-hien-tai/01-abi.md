# 11.1. ABI và syscall

Nguồn: `crates/vifast-abi/src/lib.rs`.

## 1. Thuộc tính

`vifast-abi` là `#![no_std]` và `#![forbid(unsafe_code)]`. Mọi kiểu dùng trên
biên kernel/user có `#[repr(C)]`, `#[repr(u32)]`, `#[repr(u64)]` hoặc là số
nguyên. Không có `String`, `Vec`, `Option` trong cấu trúc wire.

Hằng số quan trọng:

| Hằng số | Giá trị | Ý nghĩa |
|---|---:|---|
| `DEFAULT_PAGE_SIZE` | 4096 | Trang mặc định của các port hiện tại |
| `MSG_MAX_WORDS` | 8 | Số word tối đa của một message |
| `CAP_NONE` | `u64::MAX` | Không có capability được chuyển |
| `O_RDONLY` | 0 | Mở chỉ đọc |
| `O_WRONLY` | 1 | Mở chỉ ghi |
| `O_RDWR` | 2 | Mở đọc/ghi |
| `O_CREATE` | 4 | Tạo file nếu chưa có |
| `USER_IO_MAX` | 256 | Số byte tối đa một lần copy user |
| `OBJ_ENDPOINT_BYTES` | 256 | Untyped charge cho Endpoint |
| `OBJ_NOTIFICATION_BYTES` | 128 | Untyped charge cho Notification |
| `OBJ_TCB_BYTES` | 4096 | Untyped quota cho Tcb (chỉ là quota) |

User layout dùng chung cho cả 3 kiến trúc:

```text
code   = 0x0000_0010_0000_0000
data   = code + 0xa000
stack  = code + 0x1000 .. code + 0x8fff
```

## 2. Mã lỗi `KError`

| Giá trị | Tên | Dùng khi |
|---:|---|---|
| 0 | `Ok` | Thành công |
| 1 | `InvalidCap` | Cap slot không tồn tại |
| 2 | `WrongType` | Cap trỏ sai loại object |
| 3 | `NoRights` | Thiếu quyền, bao gồm W^X hoặc chuyển cap |
| 4 | `SlotOccupied` | Slot cspace đã có cap |
| 5 | `OutOfMemory` | Hết object table/arena/queue |
| 6 | `InvalidArg` | Tham số syscall sai |
| 7 | `Alignment` | Địa chỉ/PA không căn trang |
| 8 | `AlreadyMapped` | VA đã có mapping |
| 9 | `NotMapped` | VA chưa có mapping |
| 10 | `NoCaller` | Reply khi không có client |
| 11 | `ObjectDestroyed` | `ObjRef` cũ hoặc object đã mất |
| 12 | `IrqInUse` | Dự phòng cho IRQ object |
| 13 | `PrioTooHigh` | Dự phòng cho priority |
| 14 | `Cancelled` | Dự phòng |
| 15 | `MissingPageTable` | Page table chưa có entry |
| 16 | `IllegalOperation` | Invariant/trạng thái sai |
| 17 | `NotImplemented` | Syscall chưa hỗ trợ |
| 18 | `WouldBlock` | Server đang giữ caller khác |
| 19 | `NotFound` | Không tìm thấy file |

## 3. Quyền `Rights`

`Rights` là bitmask 8-bit:

```text
READ  = 0x1
WRITE = 0x2
EXEC  = 0x4
GRANT = 0x8
```

- `derive` chỉ giảm quyền, không bao giờ tăng.
- Chỉ Endpoint/Notification được mang badge qua `derive`.
- Quyền `GRANT` cần để chuyển capability qua IPC.
- Capability nhận qua IPC bị **bỏ `GRANT`**, nên không thể chuyển tiếp tùy ý.

## 4. Object type và thread state

`ObjType`: `Untyped`, `Tcb`, `Endpoint`, `Notification`, `CNode`, `Frame`,
`VSpace`, `PageTable`, `SchedContext`.

`ThreadState`: `Inactive`, `Ready`, `Running`, `BlockedSend`, `BlockedRecv`,
`BlockedReply`, `BlockedNotification`, `Faulted`.

## 5. Message IPC

```rust
#[repr(C)]
pub struct Message {
    pub label: u64,
    pub len: u8,
    pub _pad: [u8; 7],
    pub cap: u64,              // slot gửi, hoặc CAP_NONE
    pub data: [u64; 8],
}
```

- `Message::new(label, &[words])` kiểm tra `len <= 8`.
- `words()` trả slice theo `len` đã được clamp.
- `with_cap(slot)` đánh dấu capability cần chuyển.
- User syscall `Send`/`Call` hiện truyền tối đa **3 data word** và dùng `a5`
  làm slot cap; `Reply` truyền 4 data word và không chuyển cap.
- Receiver phải chỉ định slot nhận cap rõ ràng; kernel không tự chọn slot
  fallback. Nếu slot 0/đã dùng, chuyển cap thất bại.
- `CapRevoke` trên Untyped gọi `PageTableOps` để unmap/flush ngay tại chỗ;
  object con bị free và `gen` tăng. Không còn report danh sách VA.
- User fault không panic kernel: TCB chuyển `Faulted`, runtime chọn TCB khác.
- `DebugWrite`, `Open`, `Read`, `Write` được kernel kiểm tra
  `validate_user_range()` trước khi runtime copy. Địa chỉ kernel/unmapped bị
  trả `InvalidArg`.

## 6. Bảng syscall

Số syscall là ổn định. `-` nghĩa là không dùng.

| Số | Tên | Tham số chính | Kết quả/ghi chú |
|---:|---|---|---|
| 0 | `Send` | a0 endpoint cap, a1 label, a2..a4 data, a5 cap slot | Có thể block |
| 1 | `Call` | như `Send` | Block đến khi server `Reply` |
| 2 | `Recv` | a0 endpoint cap, a1 recv slot cho cap | Trả message hoặc block |
| 3 | `Reply` | a0 bỏ qua, a1 label, a2..a5 data | Trả lời caller hiện tại |
| 5 | `Signal` | a0 notification cap | Set bit, đánh thức waiter |
| 6 | `Wait` | a0 notification cap | Trả word hoặc block |
| 7 | `Poll` | a0 notification cap | Trả word và clear |
| 8 | `Yield` | — | Hiện trả `Ok`, chưa chuyển context |
| 9 | `Retype` | a0 Untyped cap, a1 ObjType, a2 dest slot | Endpoint/Notification/Tcb/Frame/PageTable/VSpace |
| 10 | `CapCopy` | a0 src slot, a1 TCB cap, a2 dst slot, a3 rights | Copy cap sang TCB khác |
| 14 | `CapRevoke` | a0 slot | Revoke cap; Untyped destroy object con + unmap qua PageTableOps |
| 15 | `TcbConfigure` | a0 TCB cap, a1 entry, a2 stack, a3 VSpace cap, a4 priority | Set context |
| 16 | `TcbResume` | a0 TCB cap | Đưa TCB vào runqueue |
| 24 | `PageTableMap` | a0 PageTable cap, a1 VSpace cap, a2 VA | Kernel tự chọn level; trả level; toàn bộ table_span phải nằm trong user range |
| 25 | `FrameMap` | a0 VSpace cap, a1 Frame cap, a2 VA hint, a3 rights | Map, trả VA; rights phải có READ; từ chối W^X theo Frame |
| 26 | `FrameUnmap` | a0 VSpace cap, a1 VA, a2 Frame cap | Unmap + flush; Frame cap phải khớp PA |
| 27–31 | IO/IRQ | — | Chưa triển khai |
| 32 | `DebugWrite` | a0 user ptr, a1 len | In ra console |
| 33 | `GetTime` | — | Trả tick kernel |
| 34 | `Exit` | — | Thoát QEMU |
| 35 | `Open` | a0 path ptr, a1 len, a2 flags | Trả fd |
| 36 | `Read` | a0 fd, a1 buffer ptr, a2 len | Trả số byte |
| 37 | `Write` | a0 fd, a1 buffer ptr, a2 len | Trả số byte |
| 38 | `Close` | a0 fd | Đóng fd |
| 39 | `Stat` | a0 fd | Trả kích thước |
| 40 | `Seek` | a0 fd, a1 offset | Trả offset mới |

## 7. Quy ước thanh ghi

### RISC-V64

- Syscall number: `a7`.
- Tham số: `a0..a5`.
- Kết quả: `a0 = error`, `a1 = value`, `a2 = label`, `a3..a6 = data`.
- Syscall: `ecall`.

### AArch64

- Syscall number: `x8`.
- Tham số: `x0..x5`.
- Kết quả: `x0 = error`, `x1 = value`, `x2 = label`, `x3..x6 = data`.
- Syscall: `svc #0`.

### x86_64

- Syscall number: `rax`.
- Tham số: `rdi`, `rsi`, `rdx`, `rcx`, `r8`, `r9`.
- Kết quả: `rax = error`, `rdi = value`, `rsi = label`, `rdx`, `rcx`,
  `r8`, `r9` là data.
- Syscall: `int 0x80`.

## 8. Quy tắc con trỏ user

- Kernel không đọc trực tiếp con trỏ user.
- Runtime gọi `Platform::read_user` / `write_user`.
- Mỗi port chỉ cho phép các range đã biết:
  - code page;
  - mapped data page;
  - (hiện chưa dùng user stack cho file I/O).
- `DebugWrite`, `Open`, `Write` đọc từ code/data page.
- `Read` chỉ ghi vào data page.

## 9. Giới hạn ABI hiện tại

- Chưa có encode/decode message đầy đủ cho shared memory hoặc IPC buffer.
- Chưa có `Reply` mang capability.
- Chưa có quyền file, uid/gid hay path nhiều cấp.
- `Yield` chưa thực sự nhường CPU; timer preemption đang đảm nhận.
