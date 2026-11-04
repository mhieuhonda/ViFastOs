# 11.10. `vifast-user` và user layout

Nguồn: `crates/vifast-user/src/lib.rs`, `crates/vifast-abi/src/lib.rs`.

## 1. Mục đích

`vifast-user` là crate `no_std` cung cấp wrapper syscall cho chương trình user
trên cả ba kiến trúc. Nó chỉ encode register convention; mọi kiểm tra quyền,
capability và con trỏ vẫn nằm trong `vifast-kernel`.

Chương trình mẫu hiện tại vẫn là assembly nhúng trong `user.S`; crate này là
bước đệm để chuyển root task và các server sang Rust.

## 2. User layout

Layout được đặt một lần trong `vifast-abi::user_layout`:

```text
code   = 0x0000_0010_0000_0000
data   = code + 0xa000
stack  = code + 0x1000 .. code + 0x8fff
```

Cả RISC-V64, AArch64 và x86_64 dùng cùng layout. Platform chỉ cần map page
tables tại đúng VA; không tự định nghĩa lại địa chỉ user.

## 3. Quy ước syscall

| Kiến trúc | Number | Args | Return |
|---|---|---|---|
| RISC-V64 | `a7` | `a0..a5` | `a0=error`, `a1=value` |
| AArch64 | `x8` | `x0..x5` | `x0=error`, `x1=value` |
| x86_64 | `rax` | `rdi,rsi,rdx,rcx,r8,r9` | `rax=error`, `rdi=value` |

`SyscallResult` trả `KError` + `u64` value.

## 4. Wrapper hiện có

- `debug_write`
- `get_time`
- `yield_now`
- `retype`
- `cap_copy`
- `tcb_configure`
- `tcb_resume`
- `exit`

## 5. Hạn chế

- Chưa có wrapper `Send/Call/Recv/Reply` dạng typed message.
- Chưa có IPC buffer/shared memory.
- Chưa có runtime/allocator cho user Rust binary.
- Chưa thay thế assembly mẫu; chưa có ELF loader.
