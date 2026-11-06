# 11.2. Object, capability và Untyped

Nguồn: `crates/vifast-kernel/src/object.rs`,
`crates/vifast-kernel/src/lib.rs`.

## 1. Object table

Kernel dùng một bảng cố định:

```rust
MAX_OBJECTS  = 147
MAX_TCBS     = 18
MAX_CAPS_ROOT = 96
MAX_CAPS_TASK = 64
MAX_QUEUE    = 8
MAX_FDS      = 8
MAX_MAPPINGS = 64
MAX_IOMMU_MAPPINGS = 32
```

Mỗi object có:

```rust
pub struct Object {
    pub gen: u32,
    pub kind: Kind,
    pub owner: Option<ObjRef>, // Untyped đã tạo object
    pub creator: Option<CapRef>, // cap chính xác đã dùng để retype
    pub alloc_offset: u32, // offset trong Untyped, dùng để reclaim
    pub alloc_len: u32,    // số byte đã charge
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
| `VSpace` | Reverse mapping table + root page table |
| `PageTable` | Metadata page table page |
| `CNode` | Bảng cap phẳng 64 slot |
| `SchedContext` | Budget/period + TCB bound |
| `Irq` | Logical IRQ + notification + pending |
| `Iommu` | Domain + 32 DMA mapping |
| `BlockDevice` | Cap quyền block I/O |

## 2. Capability

```rust
pub struct Cap {
    pub obj: ObjRef,
    pub ty: ObjType,
    pub rights: Rights,
    pub badge: u64,
    pub id: u64,
    pub root: bool,
    pub parent: Option<CapRef>,
}

pub struct CapRef { pub addr: SlotAddr, pub id: u64 }
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
    pub reserved: usize,
    pub free: [Option<FreeRegion>; 16], // (offset, len) đã trả
    pub leaked: u32,                    // byte trả khi free list đầy
}

pub struct FreeRegion { pub offset: u32, pub len: u32 }
```

`retype` hiện hỗ trợ:

| Loại | Charge | Ghi chú |
|---|---:|---|
| `Frame` | page (4096) | `pa` căn page; zero lại khi cấp |
| `PageTable` | page | `root = pa`; zero lại khi cấp |
| `VSpace` | page × `vspace_pages` | root + aux; zero/install kernel |
| `Endpoint` | 256 | |
| `Notification` | 128 | |
| `Tcb` | 4096 | quota, không phải `size_of::<Tcb>` |
| `CNode` | 4096 | |
| `SchedContext` | 64 | |
| `Iommu` | 1024 | |
| `BlockDevice` | 64 | |

Cấp phát (`untyped_take`):

1. Tìm vùng free **đúng kích thước** và căn đúng `align`;
2. nếu không có, lấy vùng free lớn hơn đầu tiên (first fit, tách phần dư);
3. nếu hết, cấp từ `bump` (căn lên theo `align`).

Thu hồi (`untyped_reclaim`) khi object bị destroy:

- nếu vùng nằm sát `bump` (đỉnh): hạ `bump`, rồi gộp tiếp các vùng free
  liền kề bên dưới;
- ngược lại: gộp với vùng free liền kề và đưa vào free list 16 entry;
- free list đầy: object vẫn bị destroy nhưng byte được cộng vào `leaked`
  (không tái sử dụng, được in ra để theo dõi);
- vùng chồng lấn (double free) bị coi là leak, không làm hỏng free list.

Mỗi object đều tiêu một slot `objects`; nếu không đủ byte hoặc không đủ
`objects + reserved + 1`, trả `OutOfMemory`. `reserved` giữ tổng `max_objects`
của các Untyped con trực tiếp, để sibling không tiêu hết quota của con.
Retype Untyped con lấy vùng căn trang từ cha (qua free list hoặc bump); con có
bump/objects/reserved/free riêng. Revoke Untyped cha thu hồi object bên trong
con trước, sau đó hủy con và trả `reserved`; revoke cap con chỉ reset nội dung
của con, không đụng tới cha.

## 4. Frame, VSpace, PageTable

### Frame

```rust
pub struct Frame {
    pub pa: usize,
    pub size: usize,
    pub mapped_w: u16,
    pub mapped_x: u16,
}
```

Chỉ là metadata vật lý. Kernel không tự đọc/ghi nội dung frame trừ khi
platform map nó hoặc copy user image. `mapped_w`/`mapped_x` đếm số leaf đang
WRITE/EXEC của chính Frame object này; W^X được kiểm tra trên toàn bộ Frame,
không chỉ một mapping, nên không thể map RW ở VA này rồi RX ở VA khác.

### VSpace

```rust
pub struct VSpace {
    pub root: u64,
    pub aux_pa: Option<u64>,
    pub page_size: u64,
    pub base: u64,
    pub next_va: u64,
    pub end: u64,
    pub mappings: [Option<Mapping>; MAX_MAPPINGS],
}
```

- `root` là handle mờ với kernel: Sv39 root, TTBR0 root hoặc CR3.
- `mappings` là reverse map để kiểm tra overlap/unmap và là nguồn tính
  `Frame.mapped_w/mapped_x`; page table thật là nguồn sự thật của phần cứng.
- `Cap` có `id: u64` tăng đơn điệu; `root: bool`; `parent` là `CapRef { addr, id }`.
  `root=true` chỉ do `install_cap` đặt cho Untyped khi parent=None lúc runtime boot;
  CapCopy, IPC và reparent giữ `root=false`.
- `Object.creator` lưu `CapRef`; revoke chỉ khớp cả slot và id, nên slot tái
  sử dụng không kế thừa creator cũ.
- `delete_cap` trả `KResult`: `InvalidCap` nếu không có cap, `IllegalOperation`
  nếu Untyped cap còn object/cap con.
- Khi TCB bị hủy, object creator/cap con được reparent lên cap cha còn sống;
  nếu không còn cha thì creator/parent = None; chỉ root cap thu hồi được nhóm đó.

- `PageTable { root, mapped }`; level nằm trong mapping do PageTableMap gán, và không thay entry 0 dùng chung.
- `next_va` dùng để cấp VA tuần tự trong range `[base, end)`.

### Mapping

```rust
pub struct Mapping {
    pub va: u64,
    pub pa: u64,
    pub len: u64,
    pub rights: Rights,
    pub frame: ObjRef,
}
```

`frame` trỏ đúng Frame object đã tạo leaf, nhờ đó bộ đếm W/X vẫn chính xác.
Invariant PA uniqueness bên dưới bảo đảm không có hai Frame sống cùng PA, nên
mỗi trang vật lý chỉ có một bộ đếm W/X duy nhất.

## 5. Object lifecycle

Có ba đường destroy:

1. `CapRevoke` trên Untyped: thu hồi cả cây object do các cap con tạo;
2. `TcbDestroy` (49): reparent cspace rồi destroy một TCB không phải current;
3. `ObjectDestroy` (50): destroy **một** object qua cap gốc.

`ObjectDestroy` chấp nhận `Frame`, `Endpoint`, `Notification`, `PageTable`,
`VSpace`, `CNode`, `SchedContext`; từ chối `Tcb`, `Untyped`, `Irq`, `Iommu`,
`BlockDevice` bằng `InvalidArg`.

Điều kiện (kiểm theo thứ tự):

1. cap hợp lệ, object còn sống, caller có `WRITE`;
2. cap là cap gốc do `Retype` tạo (`cap.parent == object.creator`), không phải
   bản copy/mint và không phải cap runtime (`root = true`);
3. không còn cap nào khác trỏ tới object → nếu còn trả `HasDependents`
   (phải `CapRevoke` trước);
4. object không đang dùng → nếu đang dùng trả `Busy`:

| Loại | Busy khi |
|---|---|
| Frame | còn mapping trong bất kỳ VSpace, IOMMU domain, hoặc là IPC buffer |
| Endpoint | send/recv queue không rỗng, hoặc là `fault_ep` của TCB |
| Notification | có waiter, bind vào TCB, hoặc gắn với IRQ |
| PageTable | đang gắn vào VSpace, hoặc còn PTE khác 0 |
| VSpace | có TCB dùng, còn mapping, hoặc có PageTable gắn vào |
| CNode | đang là cspace của TCB |
| SchedContext | không từ chối: destroy tự unbind + đánh thức TCB |

Thành công: gọi `destroy_object` (unmap/detach nếu cần), xoá cap, trả slot
object về `Free`, tăng `gen`, trả byte về Untyped qua `untyped_reclaim`.
`SchedContext` đang budget-blocked được đưa về Ready và wake.

Chưa có: destroy `Irq/Iommu/BlockDevice/Untyped`, huỷ Frame đang map (phải
`FrameUnmap` trước), pager eviction (sẽ dùng ObjectDestroy ở vòng 24).

## 6. Invariant checker

`Kernel::check_invariants()` kiểm tra:

- runqueue không vượt `MAX_TCBS`, không trùng TCB;
- TCB queued phải thực sự có trong runqueue;
- `current` phải `Running` và không queued;
- cspace trỏ object sống, đúng type;
- fd trỏ file còn tồn tại;
- VSpace mapping đúng alignment, overlap, có READ và không W+X trên một leaf;
- `Frame.mapped_w/mapped_x` khớp chính xác số leaf W/X trong toàn bộ reverse map;
- `Untyped.objects`/`reserved` khớp cây owner; `objects + reserved <= max_objects`;
- Frame/PageTable/VSpace root+aux không chồng dải PA; Untyped chỉ lồng theo
  owner chain và con nằm trọn trong cha;
- Frame/PageTable alignment;
- Untyped bump không vượt size; các free region dương, không chồng nhau và
  nằm dưới bump; `leaked <= size`; `alloc_offset/alloc_len` của object nằm
  trong Untyped chủ;
- notification waiter còn sống.

Hàm này chạy trong self-test. Trong tương lai có thể chạy định kỳ hoặc từ
watchdog.
