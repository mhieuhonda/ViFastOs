# Thiết kế pager protocol v2 (dự thảo cho vòng 24)

Trạng thái: **implemented** (cập nhật r234). `PAGER4`/`DEMAND LOAD OK`
trên cả 3 arch, `DEMAND VFSLOAD OK` nạp trang từ FS, evict/refault có
marker riêng, harness đã bỏ loại trừ x86. Mục tiêu: thay demo pager hai client
bằng một pager service tổng quát, nạp trang từ file qua VFS, evict được, và
dùng `ObjectDestroy` (syscall 50) làm đường thu hồi Frame.

## 1. Hiện trạng (sau vòng 23)

- Fault endpoint gửi message `LABEL_FAULT` với `data[0]=fault VA`,
  `data[1]=cause`, `data[2]=fault endpoint slot`, `data[3]=TCB index`.
- Pager server là TCB chạy trong **root VSpace**, giữ cap cho 2 cặp TCB/Frame,
  `FrameMap` Frame vào root VSpace rồi `TcbResume` pager.
- Page table phải được root tạo trước bằng `map_with_tables` (pre-map/unmap)
  vì server không có đường tạo PageTable lúc fault.
- Eviction chưa có: Frame đã map không thể `ObjectDestroy` (trả `Busy`).

## 2. Mục tiêu vòng 24

1. Pager phục vụ **mọi VSpace** được cấp cap, không chỉ root VSpace.
2. Nạp trang từ file qua VFS: fault → đọc file → Frame → map → resume.
3. Evict trang: unmap + destroy Frame + trả bộ nhớ về Untyped.
4. Giới hạn: 4 client, 16 trang/file, không cần mmap POSIX đầy đủ.

## 3. Cap và quyền

| Cap | Ai giữ | Quyền | Mục đích |
|---|---|---|---|
| Endpoint fault | pager server | READ/WRITE/GRANT | nhận fault |
| VSpace của client | pager server | READ/WRITE | map/unmap trang |
| PageTable (nếu cần) | pager server | READ/WRITE | tạo bảng lá |
| TCB của client | pager server | WRITE | `TcbResume` |
| Fd/file VFS | pager server | READ | đọc nội dung trang |
| Untyped (data) | pager server | WRITE | retype Frame |

Root cấp các cap này lúc spawn pager; pager không tự `CapCopy` từ client.

## 4. Message

Giữ label 64-bit, 4 data word, tối đa 1 cap chuyển kèm (`msg.cap`).

```text
PAGER_FAULT   (kernel → pager)  data = [va, cause, fault_ep_slot, badge]
PAGER_MAP     (root  → pager)  data = [vspace_slot, file_ino, offset, len]
PAGER_UNMAP   (root  → pager)  data = [vspace_slot, va]
PAGER_EVICT   (root  → pager)  data = [vspace_slot, va]
PAGER_REPLY   (pager → root/client) data = [va, error, 0]
```

- **Client được xác định bằng badge 1..4** do root mint trên fault endpoint;
  `data[3]` là badge, không phải TCB/object index của kernel.
- `PAGER_FAULT` do kernel sinh; pager giữ bảng `badge -> (VSpace, TCB,
  backing)`.
- Trang file map **READ-only**; trang ẩn danh map READ|WRITE, không EXEC.
- `error` dùng `KError` 32-bit; 0 = OK.

## 5. Luồng fault

```text
client đọc VA chưa map
  → MMU fault → kernel: BlockedFault, gửi PAGER_FAULT tới fault endpoint
  → pager:
      1. xác định (file, offset) từ VA (bảng ánh xạ do root cấp)
      2. retype Frame từ Untyped
      3. VFS READ file vào Frame (FrameWrite hoặc đọc qua IPC buffer)
      4. FrameMap(VSpace, Frame, VA, READ|WRITE)
         - nếu MissingPageTable: PageTableMap(PageTable, VSpace, VA) rồi thử lại
      5. TcbResume(client_tcb)
  → client chạy tiếp
```

Yêu cầu kernel:

- `FrameMap`/`PageTableMap` cho phép caller không phải root, miễn có cap
  VSpace/PageTable với `WRITE`.
- Fault message phải mang đủ để pager chọn client (hiện có `tcb` + slot) và
  VA; thêm `vspace` index nếu pager phục vụ nhiều VSpace.
- `TcbResume` chỉ chấp nhận TCB đang `BlockedFault` (đã có).

## 6. Luồng evict

```text
PAGER_EVICT(vspace, va)
  → FrameUnmap(vspace, va, frame)
  → ObjectDestroy(frame_cap)        # trả Frame về Untyped
  → (tuỳ chọn) ghi lại trang nếu dirty: VFS WRITE trước khi destroy
  → PAGER_REPLY(va, 0)
```

Điều kiện với `ObjectDestroy`:

- Frame phải không còn mapping nào (kể cả IOMMU, IPC buffer) → đã unmap.
- Không còn cap copy: pager phải tiêu thụ/xoá cap phụ trước khi destroy, hoặc
  giữ đúng một cap gốc và dùng nó.
- Chọn **ObjectDestroy rồi Retype lại** (không giữ pool Frame) để đi qua
  reclaim; báo cáo `reused` lấy từ free list.
- Page table leaf đã bị `FrameUnmap` xoá; không cần destroy PageTable (bảng
  dùng lại cho lần fault sau).

Đây chính là lý do vòng 23 phải làm object lifecycle trước pager.

## 7. Bảng ánh xạ VA → file

Pager giữ một bảng nhỏ (16 entry):

```text
struct Backing { vspace: ObjRef, va_start: u64, va_len: u64, ino: u16, file_off: u64 }
```

`PAGER_MAP` thêm entry; `PAGER_UNMAP` xoá. Fault ngoài bảng → `PAGER_REPLY`
với `KError::NotFound` và không resume (client bị kill hoặc panic tùy test).

## 8. Đồng bộ và lỗi

- Pager chạy dưới BKL như mọi syscall; không giữ lock xuyên IPC.
- Fault đến khi pager đang xử lý fault khác: endpoint queue 8 entry; pager
  xử lý tuần tự, client khác block.
- `OutOfMemory` khi retype Frame: trả lỗi, giữ client `BlockedFault`; test phải
  chứng minh không leak Frame.
- `FrameMap` fail sau khi retype: `ObjectDestroy` Frame để trả bộ nhớ.
- Nếu client bị `TcbDestroy` khi đang fault: pager phải bỏ entry và destroy
  Frame liên quan (test reap).

## 9. Test plan

Host test:

- protocol encode/decode 4 word;
- backing lookup (trong/ngoài range, nhiều VSpace);
- evict → destroy → retype lại cùng PA (dùng free list);
- fault không có backing → NotFound.

QEMU:

- 4 client, mỗi client đọc 4 trang từ file; `PAGER3 OK`.
- evict toàn bộ rồi fault lại; dữ liệu đọc đúng; `PAGER REUSE OK`.
- xoá file trong lúc fault → lỗi sạch, không panic; `PAGER ERR OK`.
- harness: thêm `PAGER3 OK` (exact one), `PAGER3 BAD` (forbidden).

## 10. Câu hỏi mở

1. ~~Cấp PageTable cap hay kernel tự tạo bảng?~~ **Đã chốt:** pager nhận
   Untyped con riêng và tự `Retype(PageTable)`; gặp `MissingPageTable` thì
   retype rồi thử lại.
2. Có cần dirty bit để quyết định ghi lại trang? (đề xuất: mọi trang map RW
   đều coi là dirty ở MVP.)
3. Giới hạn số client đồng thời có nên là 4 hay theo MAX_TCBS?
