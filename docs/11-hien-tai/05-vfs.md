# 11.5. ramfs và file descriptor

Nguồn: `crates/vifast-kernel/src/fs.rs`,
`crates/vifast-runtime/src/lib.rs`.

## 1. Mục tiêu

Đây là VFS tối giản, **không heap**, đủ để chứng minh đường user → syscall →
file. Nó chưa phải filesystem dùng cho production.

Giới hạn:

```text
MAX_FILES     = 8
MAX_FILE_NAME = 24 byte
MAX_FILE_SIZE = 512 byte
MAX_FDS       = 8 mỗi TCB
MAX_PATH      = 64 byte
```

## 2. RamFile

```rust
pub struct RamFile {
    pub name: [u8; 24],
    pub name_len: u8,
    pub data: [u8; 512],
    pub len: usize,
    pub used: bool,
    pub writable: bool,
}
```

- `format()` xoá toàn bộ và tạo:
  - `/motd` = `"ViFastOS ramfs\n"`;
  - `/hello` = `"hello from ViFastOs\n"`.
- `create(name, data)` tạo file mới nếu còn slot.
- `find(name)` so sánh chính xác từng byte; không có path walker.
- `read`/`write` thao tác trên offset tuyệt đối.
- `write` cập nhật `len` nếu ghi vượt cuối file.

## 3. File descriptor

Mỗi TCB có:

```rust
fds: [Option<Fd>; 8]
```

```rust
pub struct Fd {
    pub file: u8,
    pub offset: usize,
    pub flags: u8,
}
```

- `flags` chỉ lưu access mode (`O_RDONLY`, `O_WRONLY`, `O_RDWR`).
- `O_CREATE` chỉ dùng lúc `Open`, không lưu.
- `Read` trên `O_WRONLY` trả `NoRights`.
- `Write` trên `O_RDONLY` trả `NoRights`.

## 4. Syscall file

| Syscall | Hành vi |
|---|---|
| `Open(path, len, flags)` | Tìm file; nếu `O_CREATE` thì tạo; cấp fd |
| `Read(fd, ptr, len)` | Đọc vào user data page, tăng offset |
| `Write(fd, ptr, len)` | Đọc từ user code/data page, ghi file |
| `Seek(fd, offset)` | Đặt offset tuyệt đối, không vượt `len` |
| `Stat(fd)` | Trả `len` file |
| `Close(fd)` | Giải phóng fd |

## 5. Đường đi của `Read`

```text
user Read(fd, ptr, len)
  → runtime UserAction::Read
  → Platform::read_user? không, Kernel::read_fd vào buffer kernel
  → Platform::write_user(ptr, buf)
  → trả n
```

`Read` hiện chỉ cho ghi vào **data page** của user, không cho stack tùy ý.

## 6. Đường đi của `Open`

```text
user Open(path, len, flags)
  → runtime UserAction::Open
  → Platform::read_user path
  → Kernel::open_path
  → RamFs::find/create
  → cấp fd
```

## 7. Demo user

Client thực hiện:

1. mở `/motd`, đọc và in;
2. mở `/scratch` bằng `O_CREATE | O_RDWR`;
3. ghi `"ViFastOS write\n"`;
4. seek 0, đọc lại, in;
5. đóng fd, unmap page.

Vì vậy log QEMU có cả:

```text
ViFastOS ramfs
ViFastOS write
```

## 8. Chưa có

- thư mục, path nhiều cấp;
- quyền file/uid/gid;
- mount, symlink, rename, unlink;
- block device, cache, pager, fsync;
- filesystem bền vững;
- đồng bộ nhiều TCB trên cùng file (hiện chỉ có offset riêng từng fd).
