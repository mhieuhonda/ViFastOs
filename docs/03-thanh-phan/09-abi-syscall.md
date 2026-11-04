# 03.9. ABI và bảng syscall

Mã nguồn: `vifast-abi/src/lib.rs`; Python `abi.py`, `ktypes.py`.

## 1. Quy ước gọi

- Lệnh `ecall` từ U-mode. `a7` = số hiệu syscall, tham số trong `a0..a6` (bản hiện tại truyền enum `Syscall` trực tiếp; mã hoá thanh ghi chi tiết sẽ làm khi viết `vifast-core`/`vifast-user`).
- Trả về: `a0` = `KError`, `a1` = value/badge, message trong `a2..` (khi mã hoá thanh ghi).
- `cptr = 0` luôn là null. `dest_cnode = 0` nghĩa là CNode gốc của người gọi.

## 2. Bảng syscall

| # | Syscall | Cap cần | Quyền | Chặn? | Ghi chú |
|---|---|---|---|---|---|
| 0 | Send(ep, msg) | Endpoint | W (+G để chuyển cap) | Có | |
| 1 | Call(ep, msg, recv_slot) | Endpoint | W | Có | Chờ Reply |
| 2 | Recv(ep, recv_slot) | Endpoint | R | Có | Kiểm tra bound notification trước |
| 3 | Reply(msg) | (quyền reply ngầm) | | Không | |
| 4 | ReplyRecv(ep, msg, recv_slot) | Endpoint | R | Có | Reply lỗi thì bỏ qua |
| 5 | Signal(ntfn) | Notification | W | Không | OR badge hoặc 1 |
| 6 | Wait(ntfn) | Notification | R | Có | |
| 7 | Poll(ntfn) | Notification | R | Không | |
| 8 | Yield | | | Không | |
| 9 | Retype(untyped, type, bits, dest, count, dest_cnode) | Untyped | W | Không | count 1..64 |
| 10 | CapCopy(src, dest, rights, dest_cnode) | bất kỳ | | Không | Quyền chỉ giảm |
| 11 | CapMint(src, dest, rights, badge, dest_cnode) | Endpoint/Notification | | Không | Badge một lần |
| 12 | CNodeMint(src, dest, guard, guard_bits, dest_cnode) | CNode | | Không | Tạo cap CNode có guard |
| 13 | CapDelete(slot) | bất kỳ | | Không | |
| 14 | CapRevoke(slot) | bất kỳ | | Không | Có thể Preempted |
| 15 | TcbConfigure(tcb, cspace, vspace, prio, mcp, image, args, fault_ep) | TCB(W), CNode, VSpace, Endpoint(W) | | Không | prio, mcp ≤ MCP người gọi; TCB phải INACTIVE/FAULTED |
| 16 | TcbResume(tcb) | TCB | W | Không | |
| 17 | TcbSuspend(tcb) | TCB | W | Không | Đang chặn thì kết quả là CANCELLED |
| 18 | TcbRestart(tcb) | TCB | W | Không | |
| 19 | TcbSetPriority(tcb, prio) | TCB | W | Không | ≤ MCP người gọi |
| 20 | TcbSetAffinity(tcb, hart) | TCB | W | Không | |
| 21 | TcbBindNotification(tcb, ntfn) | TCB(W), Notification(R) | | Không | |
| 22 | SchedContextConfigure(sc_ctrl, sc, budget, period) | SchedControl, SchedContext(W) | | Không | 0 < budget ≤ period |
| 23 | SchedContextBind(sc, tcb) | SchedContext(W), TCB(W) | | Không | |
| 24 | PageTableMap(pt, vspace, vaddr) | PageTable, VSpace(W) | | Không | Trả cấp đã gắn |
| 25 | FrameMap(frame, vspace, vaddr, rights) | Frame, VSpace(W) | | Không | W^X; có thể MISSING_PAGE_TABLE |
| 26 | FrameUnmap(frame) | Frame | | Không | Shootdown |
| 27 | IoMap(frame, iospace, iova, rights) | Frame, IoSpace(W) | | Không | Không nhận frame thiết bị |
| 28 | IoUnmap(frame) | Frame | | Không | |
| 29 | IrqGet(control, irq, dest, dest_cnode) | IrqControl | | Không | IRQ_IN_USE nếu đã có |
| 30 | IrqSetNotification(handler, ntfn) | IrqHandler, Notification(W) | | Không | |
| 31 | IrqAck(handler) | IrqHandler | | Không | |
| 32 | DebugPrint(text) | | | Không | Chỉ dùng khi phát triển |
| 33 | GetTime | | | Không | Trả tick |

Load/Store/Fetch **không phải syscall**: là lệnh máy đi qua MMU.

## 3. Mã lỗi

| Mã | Tên | Khi nào |
|---|---|---|
| 0 | OK | |
| 1 | INVALID_CAP | cptr không phân giải được, slot rỗng, guard sai, CNode zombie |
| 2 | WRONG_TYPE | Cap đúng chỗ nhưng sai loại |
| 3 | NO_RIGHTS | Thiếu quyền; gán badge lần hai |
| 4 | SLOT_OCCUPIED | Slot đích đã có cap |
| 5 | OUT_OF_MEMORY | Untyped không đủ; hết ASID |
| 6 | INVALID_ARG | Tham số ngoài phạm vi |
| 7 | ALIGNMENT | vaddr không căn trang |
| 8 | ALREADY_MAPPED | Cap đã map, hoặc vị trí đã có ánh xạ |
| 9 | NOT_MAPPED | Unmap cap chưa map |
| 10 | NO_CALLER | Reply mà không có ai chờ, hoặc người chờ đã bị restart |
| 11 | OBJECT_DESTROYED | Object bị huỷ khi đang chờ trên nó |
| 12 | IRQ_IN_USE | IRQ đã có handler |
| 13 | PRIO_TOO_HIGH | Vượt MCP |
| 14 | CANCELLED | Đối tác bị restart/suspend/huỷ |
| 15 | MISSING_PAGE_TABLE | FrameMap thiếu bảng trung gian |
| 16 | ILLEGAL_OPERATION | Vi phạm W^X |

## 4. Ổn định ABI
- `vifast-abi` là hợp đồng giữa kernel và mọi chương trình user. Đổi số hiệu syscall, thứ tự mã lỗi hay bố cục Message là thay đổi phá vỡ.
- Khi bắt đầu có chương trình user thật: đóng băng phiên bản ABI, thêm syscall mới ở cuối, không tái sử dụng số đã bỏ.
- Kiểu Rust (`Syscall` enum) không phải ABI nhị phân. Mã hoá thanh ghi phải được đặc tả và test riêng (encode ở `vifast-user`, decode ở `vifast-core`, test round-trip).
