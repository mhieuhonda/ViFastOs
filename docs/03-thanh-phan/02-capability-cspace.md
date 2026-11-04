# 03.2. Capability và CSpace

Mã nguồn: `vifast-kernel/src/cspace.rs`, `object.rs` (Cap, Slot); Python `cspace.py`.

## 1. Capability

```rust
pub struct Cap {
    pub obj_type: ObjType,
    pub obj: Option<ObjRef>,
    pub rights: Rights,              // R W X G
    pub badge: u64,                  // endpoint/notification
    pub irq: Option<u32>,            // IrqHandler
    pub mapped: Option<Mapping>,     // frame/page table đang map ở đâu
    pub io_mapped: Option<(ObjRef, u64)>,
    pub guard: u64,                  // CNode: guard
    pub guard_bits: u8,
}
```

Ý nghĩa quyền theo loại object:

| Quyền | Endpoint | Notification | Frame | CNode | TCB/VSpace/Untyped |
|---|---|---|---|---|---|
| R | Recv | Wait/Poll | đọc | | |
| W | Send/Call | Signal | ghi | là dest_cnode | thao tác cấu hình |
| X | | | thực thi | | |
| G | chuyển cap qua IPC | | | | |

## 2. Dẫn xuất (`derive`)

- Quyền con = quyền cha ∩ quyền yêu cầu. **Không bao giờ tăng quyền.**
- Badge chỉ gán cho endpoint/notification, và chỉ khi cap cha chưa có badge (gán một lần). Cố gán lần hai: `NO_RIGHTS`.
- Cap con không mang theo `mapped`/`io_mapped`: mỗi cap map được đúng một lần. Muốn chia sẻ một frame vào hai VSpace: copy cap, map bản copy.

## 3. Cây dẫn xuất (CDT)

Mỗi slot biết `parent` và `children`. Mọi Copy, Mint, Retype, chuyển cap qua IPC đều tạo cạnh cha con.

```
Untyped(mem) ── Frame A (retype)
              ├─ Endpoint E (retype) ── E' (mint badge 7) ── E'' (chuyển qua IPC)
              └─ CNode C (retype)
```

- **Delete(slot):** xoá một cap. Con của nó được nối lên cha (không mất quyền mà cap này đã cấp). Object hết cap thì huỷ.
- **Revoke(slot):** xoá toàn bộ con cháu, cap gốc giữ nguyên. Đây là cách **thu hồi quyền đã cấp**, kể cả cap đã chuyển sang tiến trình khác qua IPC (test `cap_transfer_and_revoke`). Revoke một cap Untyped huỷ mọi object sinh từ nó và trả lại toàn bộ vùng nhớ.

### Revoke có preemption point
Revoke có thể phải xoá hàng nghìn cap. Kernel xoá tối đa `REVOKE_BATCH = 8` cap (từ lá lên) rồi trả `Preempted`. PC không tăng, thread chạy lại đúng syscall ở lượt sau và tiếp tục. Như vậy thời gian mỗi lần vào kernel luôn có giới hạn, timer và ngắt vẫn được phục vụ giữa chừng. Test `revoke_is_preemptible`: 40 cap tạo đúng 4 lần preempt.

**Chú ý:** trong lúc revoke dở dang, các cap chưa bị xoá vẫn dùng được. Ngữ nghĩa này giống seL4. Nếu cần "thu hồi nguyên tử", dịch vụ user phải tự dừng các bên liên quan trước.

## 4. CNode và phân giải cptr nhiều cấp

CSpace của thread là một cây CNode. `cptr` là số 64 bit, được tiêu thụ từ bit cao xuống:

```
cptr (64 bit):  [ guard cấp 1 | index cấp 1 | guard cấp 2 | index cấp 2 | ... ]
                  guard_bits1   radix1        guard_bits2   radix2
```

Thuật toán `resolve(t, cptr)`:
1. Bắt đầu ở CNode gốc với guard và guard_bits lưu trong TCB.
2. Lấy `guard_bits` bit tiếp theo, phải bằng guard, sai thì `INVALID_CAP`.
3. Lấy `radix` bit tiếp theo làm chỉ số slot.
4. Nếu còn bit chưa dùng và slot chứa cap CNode: đi xuống CNode đó với guard của cap đó.
5. Ngược lại trả slot tìm được.

Ví dụ trong test `multilevel_cspace_with_guard`: gốc radix 4 + guard 52 bit; slot 2 chứa CNode radix 8; slot 5 của CNode đó là endpoint. `cptr = (2 << 8) | 5` phân giải được. `cptr = 5` (kiểu một cấp) đi vào slot 0 rỗng nên `INVALID_CAP`.

Boot cấu hình mỗi task với CSpace một cấp: guard 0, `guard_bits = 64 - radix`, nên cptr nhỏ chính là chỉ số slot.

**Vì sao cần guard?** Cho phép CSpace thưa và sâu mà không tốn CNode cho mỗi bit: một CNode 16 slot vẫn phủ được toàn bộ không gian 64 bit. Đây cũng là chỗ dễ sai nhất khi port: shift 64 bit trong Rust gây panic, xem hàm `bits_at`.

## 5. Zombie CNode

Khi một CNode hết cap trỏ tới, việc xoá mọi cap bên trong có thể kéo theo huỷ CNode khác, đệ quy không giới hạn. Ngoài ra CNode có thể chứa cap trỏ về chính nó (vòng).

Giải pháp: `destroy_object(CNode)` chỉ đánh dấu `dying` và đưa vào `reap_queue`. Mỗi tick, `reap(REAP_BUDGET = 16)` xoá dần các slot. Xoá xong thì giải phóng thật. CNode zombie không phân giải được (`live_cnode` từ chối), nên user không thể dùng nó nữa.

Vòng tự tham chiếu: CNode C chứa cap tới C. Xoá cap ngoài thì `refs` còn 1, C chưa chết. Revoke untyped cha: cap bên trong C là con cháu của cap untyped (qua CDT) nên bị xoá, `refs` về 0, C thành zombie và được dọn. Test `zombie_cnode_cycle_is_reclaimed`.

## 6. Bất biến cần giữ
1. Mỗi slot có cap thì `refs` của object tính đúng nó một lần.
2. Mọi cap (trừ cap gốc lúc boot) có đúng một cha trong CDT; cha luôn tồn tại.
3. Quyền của con ⊆ quyền của cha.
4. `mapped` khác `None` khi và chỉ khi có PTE trỏ đúng frame đó trong VSpace ghi trong `mapped`.
5. Không có cách nào tạo cap tới object mà không đi qua derive từ một cap đã có (trừ boot).

Bất biến 1, 2, 4 là ứng viên tốt để viết hàm kiểm tra `debug_check_invariants()` chạy sau mỗi syscall trong chế độ test.
