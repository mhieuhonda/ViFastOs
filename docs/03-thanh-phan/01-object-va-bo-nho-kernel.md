# 03.1. Object, bảng object và Untyped

Mã nguồn: `vifast-kernel/src/object.rs`, `cspace.rs` (retype), Python `objects.py`.

## 1. Danh sách object

| Loại | Kích thước | Tạo bằng | Vai trò |
|---|---|---|---|
| Untyped | 2^bits (bits 4..40) | Retype hoặc boot | Vùng nhớ thô, nguồn của mọi object khác |
| Tcb | 1024 B | Retype | Thread: trạng thái, thanh ghi, ưu tiên, CSpace, VSpace |
| Endpoint | 16 B | Retype | Điểm hẹn IPC đồng bộ |
| Notification | 32 B | Retype | Word bit báo hiệu bất đồng bộ |
| CNode | 2^bits × 32 B (bits 1..12) | Retype | Mảng slot chứa capability |
| Frame | 4 KB | Retype; frame thiết bị lúc boot | Trang nhớ có thể map |
| VSpace | 4 KB | Retype | Page table gốc Sv39 + ASID |
| PageTable | 4 KB | Retype | Page table trung gian |
| SchedContext | 256 B | Retype | Ngân sách CPU budget/period |
| IoSpace | (không chiếm RAM) | Boot, mỗi thiết bị một cái | Bảng dịch IOMMU |
| IrqControl | (không chiếm RAM) | Boot, duy nhất | Quyền tạo IrqHandler |
| IrqHandler | (không chiếm RAM) | IrqGet | Quyền với một đường ngắt |
| SchedControl | (không chiếm RAM) | Boot, duy nhất | Quyền cấu hình SchedContext |

## 2. Header chung

```rust
pub struct Header {
    pub refs: u32,                      // số cap trỏ tới
    pub parent_untyped: Option<ObjRef>, // untyped sinh ra nó
    pub dying: bool,                    // đang huỷ hoặc là zombie
    pub paddr: u64,                     // địa chỉ vật lý
}
```

- `refs` về 0 thì object bị huỷ (trừ khi `dying` đã bật, để tránh huỷ hai lần).
- `paddr` là địa chỉ thật trong RAM. Page table, frame dùng nó trực tiếp; các object khác dùng nó để đảm bảo không chồng lấn và để xoá trắng.

## 3. Bảng object có generation

```rust
struct Entry { gen: u32, obj: Option<Object> }
pub struct ObjectTable { entries: Vec<Entry>, free: Vec<u32> }
pub struct ObjRef { index: u32, gen: u32 }
```

- `insert`: lấy chỉ số từ free list hoặc thêm mới, trả `ObjRef { index, gen }`.
- `remove`: tăng `gen`, xoá object, trả chỉ số về free list.
- `try_get(r)`: chỉ trả object nếu `gen` khớp. ObjRef cũ không bao giờ trỏ nhầm sang object mới dùng lại chỉ số.

**Vì sao không dùng con trỏ?** Object trong kernel tham chiếu chéo rất nhiều (TCB ↔ endpoint, notification ↔ TCB, cap ↔ object, VSpace ↔ slot). Trong Rust, con trỏ chéo buộc phải dùng `unsafe` hoặc `Rc<RefCell>`. Arena có generation cho an toàn bộ nhớ, phát hiện use-after-free bằng một phép so sánh, và dễ chuyển sang mảng cố định cấp lúc boot.

**Chú ý:** `gen` là `u32` và tăng bằng `wrapping_add`. Một chỉ số phải bị dùng lại 2^32 lần thì mới có thể trùng. Trên hệ chạy rất lâu, nên giữ ObjRef ngắn hạn hoặc dùng `u64`.

## 4. Untyped và Retype

```
Untyped (size, watermark, children)
  │  Retype(Frame, n=2)
  ▼
  [ ... đã dùng ... | Frame A | Frame B | ... trống ... ]
                    ^start              ^end = watermark mới
```

Thuật toán `retype_objects`:
1. `size = object_size(type, bits)`, căn lề = `size` nếu là luỹ thừa 2, ngược lại 16.
2. `start = ceil(watermark / align) * align`, `end = start + size × n`. Vượt `untyped.size` thì `OUT_OF_MEMORY`.
3. VSpace: kiểm tra còn đủ ASID.
4. Với mỗi object: `paddr = untyped.paddr + start + i × size`, **xoá trắng** vùng nhớ, tạo object, thêm vào `children` của untyped, tạo cap con của cap untyped.

Thu hồi: khi object con cuối cùng bị huỷ (`children` rỗng), `watermark` về 0. Đây là mô hình "watermark + reset khi rỗng" của seL4: đơn giản, không phân mảnh bên trong một untyped, nhưng không tái sử dụng được từng lỗ. Muốn tái sử dụng chi tiết thì user space chia untyped nhỏ (Retype Untyped từ Untyped) và quản lý như allocator.

**Bất biến:**
- Hai object sống bất kỳ không chồng vùng nhớ.
- Bộ nhớ trả về cho user qua Retype luôn đã được xoá trắng (test `retype_zeroes_memory`).
- Vùng `KERNEL_RESERVED` (64 KB đầu RAM) không thuộc untyped nào, nên không ai có cap tới.

## 5. Huỷ object (`destroy_object`)

| Loại | Việc phải làm khi huỷ |
|---|---|
| Tcb | Nhả client đang chờ (CANCELLED), gỡ khỏi hàng đợi, xoá ngữ cảnh, gỡ notification và SchedContext |
| Endpoint | Đánh thức mọi thread đang chờ với `OBJECT_DESTROYED` |
| Notification | Đánh thức thread chờ, gỡ liên kết với TCB và IRQ |
| CNode | Thành zombie, đưa vào hàng đợi dọn dần |
| VSpace | Gỡ cờ mapped ở mọi cap frame/page table, shootdown, trả ASID |
| IoSpace | Gỡ cờ io_mapped ở mọi cap frame |
| IrqHandler | Xoá trạng thái IRQ |
| SchedContext | Gỡ khỏi TCB, đưa TCB về hàng đợi nếu đang READY |
| Untyped | Cắt liên kết parent_untyped của các con |

Cuối cùng `finalize_object`: gỡ khỏi untyped cha (reset watermark nếu hết con), xoá khỏi bảng.

## 6. Hướng cải tiến
- Siêu dữ liệu nằm trong chính vùng nhớ của object (hàng đợi intrusive, CNode là mảng slot trong RAM) thay vì `Vec`/`BTreeMap`. Khi đó `object_size` phản ánh đúng chi phí thật.
- Bảng object cố định kích thước lúc boot, dùng `[Option<Entry>; N]` hoặc vùng nhớ cấp từ untyped của kernel.
