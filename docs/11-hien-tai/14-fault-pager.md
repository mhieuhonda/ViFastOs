# 11.14. Fault endpoint, demand paging và user-space pager

Nguồn: `crates/vifast-kernel/src/lib.rs` (`fault_to_endpoint`,
`fault_current_on`, `TcbConfigure`/`TcbRestart`/`TcbResume`),
`crates/vifast-runtime/src/lib.rs` (`fault::suspend`),
`crates/vifast-root/src/main.rs` (`spawn_pager*`, `pager_server`).

## 1. Luồng fault

1. User truy cập trang chưa map → trap → `fault::suspend` lưu context vào
   slot của TCB đang fault (để pager cứu xong task tiếp tục đúng điểm fault).
2. Nếu TCB có `fault_ep` còn sống, kernel đóng gói
   `Message(LABEL_FAULT, [addr, cause, fault_slot, tcb_index])` và
   `send` (non-call) tới endpoint. Message nằm trong `send` queue nếu
   handler chưa `recv` — đó là thiết kế (handler khởi động sau vẫn nhận
   được fault đã xếp hàng).
3. TCB fault chuyển `BlockedFault`, rời `current`, task khác được chọn.
4. Pager server `recv`, xử lý (retype Frame / `frame_map` / resume client),
   rồi `tcb_resume(client)`.

`recv` trên message fault là non-call nên không đổi state sender — TCB vẫn
`BlockedFault` cho tới khi handler resume显式.

## 2. Hợp đồng `TcbConfigure` với fault endpoint (vòng 101, K75)

- `VSpace` đòi quyền `WRITE` (đổi cả không gian địa chỉ — ngang với
  `FrameMap`/`PageTableMap`, `READ` là quá yếu).
- `fault_ep` trỏ vào **slot của nạn nhân** (delivery đọc qua `cap_at`
  victim lúc fault). Yêu cầu kép lúc cài:
  1. slot nạn nhân phải có vé Endpoint còn sống;
  2. caller cũng phải giữ một vé tới cùng endpoint (chứng minh caller được
     phép định tuyến fault của nạn nhân).
- Không có syscall unbind; `destroy_notification` tự gỡ `bound_ntfn` và
  `irq.notification` mọi TCB/IRQ (vòng 101, K67/K68).

## 3. Resume từ `BlockedFault` (bài học vòng 101)

`resume_tcb` chỉ cho `Inactive`/`Suspended`/`Ready`/`BlockedFault`.
`BlockedFault` là state duy nhất trong họ Blocked được resume, vì fault
handler là đường hợp lệ duy nhất đưa task ra khỏi đó (pager flow:
server `frame_map` xong `tcb_resume(client)`).

Siết ban đầu cấm cả `BlockedFault` đã làm gãy pager (PAGER mất tích trên
QEMU trong khi host test vẫn xanh, vì host test không chạy pager flow).
Mọi `Blocked*` khác (Send/Recv/Notification/Sleep/Budget/Reply) vẫn cấm:
task còn nằm trong hàng chờ, resume mù quáng tạo thành viên kép
(K32–K36, K41).

## 4. Pager demo hiện tại

- Ba pager client (`PAGER_VA/+0x1000/+0x2000`) + client demand-load
  (`PAGER4_VA`, nạp `/page.bin` qua VFS nếu `PAGER_VFS_ENABLED`).
- Server retype Frame từ Untyped riêng ngay lúc fault (vòng 69) nên cả
  x86_64 (Untyped chật) cũng chạy được.
- Marker harness: `PAGER OK`, `PAGER2 OK`, `PAGER3 OK`, `PAGER EVICT OK`,
  `DEMAND LOAD OK`, `DEMAND VFSLOAD OK` — đều `EXACT_ONE`.
