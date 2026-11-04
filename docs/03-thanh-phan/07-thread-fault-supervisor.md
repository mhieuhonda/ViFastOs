# 03.7. Thread, fault, supervisor, pager

Mã nguồn: `vifast-kernel/src/tcb.rs`, `kernel.rs` (raise_fault); `vifast-sim/src/images.rs` (supervisor, pager); Python `tcb.py`.

## 1. Trạng thái thread

```
            TcbResume / make_ready
INACTIVE ─────────────────────────► READY ◄──────────────┐
   ▲                                 │ schedule            │ đánh thức
   │ TcbSuspend / reply fault        ▼                     │
   │ không phải VM                RUNNING ──Send/Call/Recv/Wait──► BLOCKED_*
   │                                 │ fault, không có handler
   └──────────────── TcbRestart ── FAULTED
```

| Trạng thái | Nghĩa |
|---|---|
| INACTIVE | Chưa chạy hoặc bị suspend |
| READY | Trong hàng đợi của hart |
| RUNNING | Đang là `current` của một hart |
| BLOCKED_SEND/RECV | Chờ trên endpoint |
| BLOCKED_REPLY | Đã Call (hoặc đã fault), chờ trả lời |
| BLOCKED_NOTIFICATION | Chờ Wait |
| FAULTED | Lỗi mà không có ai xử lý; chỉ TcbRestart cứu được |

## 2. TrapFrame và ABI thanh ghi

| Thanh ghi | Vào | Ra |
|---|---|---|
| a7 | Số hiệu syscall | |
| a0 | | Mã lỗi (`KError`) |
| a1 | | value, hoặc badge nếu value = 0 |
| pc | | +4 sau syscall hoàn tất hoặc bị chặn; giữ nguyên nếu Preempted/Contended/page fault |

"PC giữ nguyên" là cách kernel làm cho một lệnh **chạy lại**: revoke dở dang, tranh khoá, page fault đã được pager xử lý. Test `trap_frame_abi`.

## 3. Fault

Các loại: `VM_FAULT` (truy cập bộ nhớ không hợp lệ), `USER_EXCEPTION` (panic, lệnh không hợp lệ), `EXITED` (thoát), `BAD_SYSCALL`.

`raise_fault`:
1. Ghi log, đặt `fault_kind`.
2. Có fault endpoint còn sống: gửi **Call** với `label = LABEL_FAULT`, `data = [kind, addr]`, `badge = fault_badge`. Thread chuyển BLOCKED_REPLY (hoặc BLOCKED_SEND nếu handler chưa Recv).
3. Không có: thread FAULTED.

Handler trả lời fault:
- `VM_FAULT`: thread READY, chạy lại đúng lệnh lỗi (pager đã map trang).
- Loại khác: thread INACTIVE; muốn chạy lại phải TcbRestart.

Fault của một thread **không bao giờ** ảnh hưởng thread khác (test `fault_without_handler_stops_only_that_thread`).

## 4. Supervisor (user space)

Chính sách mẫu trong `images.rs::supervisor`:
- Nhận fault trên fault endpoint. Badge = slot TCB cap của task lỗi.
- `EXITED`: ghi log, không làm gì.
- **Dev mode**: lỗi bất kỳ thì suspend mọi task và dừng (fail-fast để thấy lỗi ngay khi phát triển).
- **Production**: restart, tối đa `max_restarts` lần trong cửa sổ `window` tick. Quá thì `DEGRADED`, không restart nữa (tránh vòng restart vô hạn che giấu lỗi).

Test `space_profile` (3 lần restart rồi DEGRADED) và `dev_mode_halts_everything`.

Đây chính là cách giải bài toán "không sập thì biết đâu mà fix": dev mode sập ngay, production thì cô lập + giới hạn + ghi log.

## 5. Restart (`TcbRestart`)

1. `release_caller`: client đang chờ thread này trả lời nhận `CANCELLED`.
2. `cancel_ipc`: gỡ khỏi mọi hàng đợi, tăng `reply_epoch`.
3. `reset_context`: tăng `context_epoch`, xoá thanh ghi, kết quả chờ, fault_kind.
4. `make_ready`: chạy lại từ điểm vào.

`context_epoch` là tín hiệu cho bên thực thi (CPU thật hoặc simulator) biết phải bỏ ngữ cảnh cũ. Trên phần cứng: TrapFrame được đặt lại `pc = entry`, `sp = stack top`.

**Chú ý:** restart không khôi phục **trạng thái** của tiến trình (bộ nhớ, cap đã nhận). Dịch vụ phải được thiết kế để khởi động lại an toàn: hoặc không giữ trạng thái, hoặc lưu trạng thái ở nơi khác (một dịch vụ lưu trữ, vùng nhớ do supervisor giữ).

## 6. Pager (user space)

`images.rs::pager`: nhận VM fault của task `lazy`, Retype Frame, FrameMap vào VSpace của lazy (thêm PageTable nếu thiếu), Reply. Lazy chạy tiếp như chưa có gì. Đây là nền cho cấp phát lười, swap, copy-on-write, file mmap, tất cả đều nằm ở user space.

## 7. Tạo tiến trình lúc chạy

`images.rs::init` (profile desktop): Retype TCB, CNode, VSpace, Frame, 2 PageTable; CNodeMint với guard để tạo CSpace; Mint cap console có badge vào CNode mới; map trang RAM; `TcbConfigure` (image, args a0..a3, ưu tiên ≤ MCP của init); `TcbResume`. Đây là "fork/exec" phiên bản capability: tiến trình mới chỉ có đúng những cap được trao.
