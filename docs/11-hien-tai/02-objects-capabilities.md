# 11.2. Object, capability và Untyped

Nguồn: `crates/vifast-kernel/src/object.rs`,
`crates/vifast-kernel/src/lib.rs`.

## 1. Object table

Kernel dùng một bảng cố định:

```rust
MAX_OBJECTS = 128
MAX_TCBS    = 16
MAX_CAPS    = 64
MAX_QUEUE   = 8
MAX_FDS     = 8
MAX_MAPPINGS = 32
```

Mỗi object có:

```rust
pub struct Object {
    pub gen: u32,
    pub kind: Kind,
    pub owner: Option<ObjRef>, // Untyped đã tạo object
    pub creator: Option<SlotAddr>, // cap đã dùng để retype
}
```

`ObjRef { index, gen }` là con trỏ logic. Khi một slot được tái sử dụng,
`gen` tăng lên; mọi `ObjRef` cũ bị từ chối. Đây là cách phát hiện
use-after-free mà không cần con trỏ thô trong lõi portable.

`Kind` hiện hỗ trợ:

| Kind | Mục đích |
|---|---|
| `Free` | Slot trống |
| `Endpoint` | Send/Recv queue, Call/Reply |
| `Notification` | Word + waiter |
| `Tcb` | Thread state, entry/stack/VSpace, cspace, fds, resume |
| `Untyped` | Arena cấp phát vật lý |
| `Frame` | Trang vật lý có thể map |
| `VSpace` | Shadow mapping table + root page table |
| `PageTable` | Metadata page table page |

## 2. Capability

```rust
pub struct Cap {
    pub obj: ObjRef,
    pub ty: ObjType,
    pub rights: Rights,
    pub badge: u64,
    pub parent: Option<SlotAddr>,
}
```

Mỗi TCB có `cspace: [Option<Cap>; MAX_CAPS]`. Slot 0 không dùng; hợp lệ là
`1..MAX_CAPS-1`.

### Cài đặt và tra cứu

- `install_cap(tcb, slot, cap)`: kiểm tra slot, object còn sống, slot trống.
- `lookup_cap(tcb, slot, want_type, need_rights)`:
  - cap phải tồn tại;
  - type phải khớp nếu có `want_type`;
  - `rights` phải chứa `need_rights`;
  - object đích phải còn sống.
- `copy_cap(src_tcb, src_slot, dst_tcb, dst_slot, rights, badge)`:
  - derive quyền;
  - gắn `parent` để revoke;
  - cài vào đích.

### Revoke

`revoke_cap` tìm mọi capability có quan hệ cha–con với slot gốc, xoá các
descendant, giữ capability gốc. Nếu slot gốc là `Untyped`, kernel còn:

- unmap mọi `Frame` con khỏi tất cả VSpace và trả danh sách VA cần flush;
- detach `Tcb` con khỏi runqueue/endpoint queue, xoá caller trỏ tới nó;
- đánh thức waiter/sender/receiver của Endpoint/Notification bằng
  `ObjectDestroyed`;
- đặt object con thành `Kind::Free`, tăng `gen`;
- sau đó mới reset `bump = 0`, `objects = 0`.

Độ sâu revoke được giới hạn bởi `MAX_OBJECTS * MAX_CAPS`.

Với capability **không phải Untyped**, revoke chỉ xoá các capability con; object
gốc vẫn sống. Object chỉ chết khi đường destroy/free tương ứng được gọi.

## 3. Untyped và retype

`Untyped` là một vùng vật lý:

```rust
pub struct Untyped {
    pub base: usize,
    pub size: usize,
    pub bump: usize,
    pub page_size: usize,
    pub max_objects: usize,
    pub objects: usize,
}
```

`retype` hiện hỗ trợ:

- `Frame`: cấp một page, `pa` căn theo `page_size`;
- `PageTable`: cấp một page, `root = pa`, `level = 3`;
- `Endpoint`: tiêu 256 byte Untyped;
- `Notification`: tiêu 128 byte Untyped;
- `Tcb`: tiêu 1024 byte Untyped.

Mỗi object đều tiêu `bump` và một slot `objects`; nếu không đủ byte hoặc
không đủ `max_objects`, trả `OutOfMemory`. Revoke Untyped destroy object con
thật, đặt `Kind::Free`, tăng `gen`, rồi reset `bump = 0`, `objects = 0`.

## 4. Frame, VSpace, PageTable

### Frame

```rust
pub struct Frame {
    pub pa: usize,
    pub size: usize,
}
```

Chỉ là metadata vật lý. Kernel không tự đọc/ghi nội dung frame trừ khi
platform map nó hoặc copy user image.

### VSpace

```rust
pub struct VSpace {
    pub root: u64,
    pub aux_pa: Option<u64>,
    pub page_size: u64,
    pub base: u64,
    pub next_va: u64,
    pub end: u64,
    pub code_base: u64,
    pub code_len: u64,
    pub stack_base: u64,
    pub stack_len: u64,
    pub mappings: [Option<Mapping>; MAX_MAPPINGS],
}
```

- `root` là handle mờ với kernel: Sv39 root, TTBR0 root hoặc CR3.
- `mappings` là reverse map để kiểm tra overlap/W^X/unmap; page table thật là nguồn sự thật.
- `Object.creator` lưu cap đã dùng để retype; revoke một bản copy Untyped
  không được hủy object tạo qua cap gốc.
- `PageTable { root, mapped }`; level nằm trong mapping do PageTableMap gán, và không thay entry 0 dùng chung.
- `next_va` dùng để cấp VA tuần tự trong range `[base, end)`.

### Mapping

```rust
pub struct Mapping {
    pub va: u64,
    pub pa: u64,
    pub len: u64,
    pub rights: Rights,
}
```

## 5. Object lifecycle

Object table **chưa có API free tổng quát**. Đường `revoke Untyped` đã destroy
Frame/TCB/Endpoint/Notification con, unmap Frame và tăng generation; ngoài
đường đó chưa có syscall free object/frame độc lập.

## 6. Invariant checker

`Kernel::check_invariants()` kiểm tra:

- runqueue không vượt `MAX_TCBS`, không trùng TCB;
- TCB queued phải thực sự có trong runqueue;
- `current` phải `Running` và không queued;
- cspace trỏ object sống, đúng type;
- fd trỏ file còn tồn tại;
- VSpace mapping đúng alignment, overlap, W^X;
- Frame/PageTable alignment;
- Untyped bump không vượt size;
- notification waiter còn sống.

Hàm này chạy trong self-test. Trong tương lai có thể chạy định kỳ hoặc từ
watchdog.
