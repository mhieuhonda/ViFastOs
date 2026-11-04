# 03.3. IPC: Endpoint, Call/Reply, Notification

Mã nguồn: `vifast-kernel/src/ipc.rs`; Python `ipc.py`.

## 1. Endpoint: điểm hẹn đồng bộ

Endpoint không có bộ đệm. Kernel chuyển message trực tiếp từ người gửi sang người nhận.

```
Send/Call tới E:
  có thread đang Recv trên E? ─có─► chuyển ngay, người nhận READY
                              └không─► người gửi BLOCKED_SEND, xếp vào send_queue

Recv trên E:
  (bound notification có tín hiệu? → trả ngay LABEL_NOTIFICATION)
  có thread trong send_queue? ─có─► lấy message của nó
                              └không─► BLOCKED_RECV, xếp vào recv_queue
```

Hàng đợi FIFO. Message: `label` + tối đa 8 word + tối đa 1 cap. Trên phần cứng, 8 word đi qua thanh ghi a0..a7 (nhanh, không copy bộ nhớ); dữ liệu lớn đi qua bộ nhớ chia sẻ.

## 2. Badge: biết ai đang gọi

Cap endpoint có thể mang badge (gán một lần khi Mint). Người nhận thấy `badge` trong kết quả Recv. Server cấp cho mỗi client một cap có badge khác nhau, nhờ vậy phân biệt được client mà không cần tin dữ liệu trong message. Supervisor dùng đúng cơ chế này: badge của fault endpoint = slot TCB của task bị lỗi.

## 3. Call/Reply và quyền trả lời một lần

```
client Call ─► server nhận; kernel ghi server.caller = (client, client.reply_epoch)
               client BLOCKED_REPLY
server Reply ─► kiểm tra: client còn BLOCKED_REPLY và reply_epoch còn khớp?
                 khớp: chuyển reply, client READY
                 không: NO_CALLER
```

`reply_epoch` tăng mỗi khi thread bị gỡ khỏi IPC (`cancel_ipc`): suspend, restart, huỷ. Nhờ vậy một server chậm không thể trả lời nhầm cho "phiên bản mới" của client sau khi client bị restart.

`ReplyRecv` = Reply + Recv trong một syscall (đường nhanh của server). Nếu phần Reply lỗi thì bỏ qua và vẫn Recv, giống seL4.

### Server chết khi đang giữ client
`release_caller`: khi server bị restart hoặc huỷ mà đang giữ quyền trả lời, client nhận `CANCELLED` thay vì treo mãi. Test `server_crash_cancels_waiting_client`.

## 4. Chuyển capability qua IPC

Điều kiện: cap endpoint của người gửi có quyền **G**, message có `cap`, người nhận chỉ định `recv_slot` khác 0 và slot đó trống. Cap được derive (quyền giữ nguyên) và là con của cap nguồn trong CDT, nên **người gửi revoke được**. Thiếu điều kiện nào thì cap bị bỏ, message vẫn đi (người nhận thấy `msg.cap = None`).

Reply luôn được phép chuyển cap (người gọi đã đồng ý nhận khi chỉ định `recv_slot` trong Call).

**Chú ý bảo mật:** `recv_slot` nằm trong CNode gốc của người nhận. Bản sau nên cho phép chỉ định CNode đích (giống seL4 `ReceivePath`) để server nhận cap vào một CNode riêng, tránh đè lên vùng slot quan trọng.

## 5. Notification

Một word 64 bit:
- `Signal`: OR badge (hoặc 1) vào word. Có thread đang Wait thì đánh thức ngay với giá trị word, xoá word.
- `Wait`: word khác 0 thì lấy và xoá; bằng 0 thì chặn.
- `Poll`: lấy và xoá, không chặn.

Các tín hiệu tới khi chưa ai lấy được **gộp lại** (OR), không đếm số lần. Đây là chủ ý: notification dùng cho "có việc cần làm", không phải hàng đợi sự kiện.

### Bound notification
Gắn một notification với một TCB. Khi TCB đang Recv trên endpoint mà notification nhận tín hiệu, Recv trả về ngay với `label = LABEL_NOTIFICATION`. Driver dùng cách này để chờ đồng thời cả request từ client (endpoint) và ngắt phần cứng (notification) trong một vòng lặp duy nhất, như `imu_driver`.

## 6. So sánh với các mô hình khác

| | ViFastOs/seL4 | Mach | Linux pipe/socket |
|---|---|---|---|
| Đệm trong kernel | Không | Có (port queue) | Có |
| Cấp phát khi gửi | Không | Có | Có |
| Thời gian xấu nhất | Xác định | Phụ thuộc hàng đợi | Phụ thuộc |
| Hệ quả | Kernel đơn giản, nhanh, dễ chứng minh | Linh hoạt nhưng chậm | Linh hoạt |

Không đệm nghĩa là **cả hai bên phải có mặt**. Muốn bất đồng bộ thì dùng bộ nhớ chia sẻ + notification (xem `producer`/`consumer`), hoặc dịch vụ hàng đợi ở user space.

## 7. Hướng mở rộng
- **IPC buffer**: trang nhớ riêng mỗi thread để gửi message dài hơn 8 word.
- **Fastpath**: đường xử lý Call/ReplyRecv tối ưu bằng tay khi điều kiện đơn giản (không chuyển cap, người nhận đang chờ, cùng hart), như seL4.
- **Reply object** (MCS): biến quyền trả lời thành object riêng để server có thể giữ nhiều client cùng lúc.
- **Async ring** kiểu io_uring giữa app và dịch vụ cho I/O số lượng lớn.
