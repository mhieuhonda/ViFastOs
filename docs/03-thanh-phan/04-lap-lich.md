# 03.4. Lập lịch: ưu tiên, timer, SMP, khoá kernel, MCS

Mã nguồn: `vifast-kernel/src/sched.rs`, `kernel.rs`; Python `sched.py`, `kernel.py`.

## 1. Hàng đợi theo hart

Mỗi hart có một `RunQueue`: 256 hàng đợi FIFO (một cho mỗi mức ưu tiên) + bitmap 256 bit (`[u64; 4]`). Chọn thread: tìm bit cao nhất bằng `leading_zeros`, O(1).

Mỗi thread có `affinity` (hart nó chạy). Không có migrate tự động: đây là chủ ý cho tính xác định. Đổi hart bằng `TcbSetAffinity`.

## 2. Quyết định lập lịch (`schedule_inner`)

```
1. Thread hiện tại không còn RUNNING hoặc đã đổi hart → bỏ
2. Tới mtimecmp (hết timeslice) → ngắt timer, resched
3. Thread hiện tại tiếp tục nếu: không resched VÀ không có ai ưu tiên cao hơn đang chờ
4. Ngược lại: đưa thread hiện tại về hàng đợi
     còn timeslice (bị chen ngang) → đầu hàng
     hết timeslice → cuối hàng
5. Chọn thread ưu tiên cao nhất, đặt mtimecmp = now + timeslice
```

- **Ưu tiên cố định:** thread ưu tiên cao luôn chạy trước (test `priority`).
- **Xoay vòng trong cùng mức:** timeslice 5 tick (test `timer_round_robin`: chuỗi bắt đầu `AAAAABBBBB`).
- `Yield` đặt timeslice = 0 và resched.

## 3. MCP: chặn tự nâng ưu tiên

Mỗi TCB có `mcp` (maximum controlled priority). Một thread chỉ được đặt ưu tiên cho thread khác (hoặc chính nó) tối đa bằng `mcp` của mình. Nếu không có MCP, bất kỳ ai có cap TCB đều có thể tạo thread ưu tiên 255 và chiếm CPU. Test `mcp_limits_priority`.

## 4. SMP và IPI

- Mỗi hart chạy độc lập, có hàng đợi riêng.
- Khi một thread được đánh thức trên hart khác và có ưu tiên cao hơn thread đang chạy ở đó, kernel gửi **IPI** để hart kia lập lịch lại (ở mô phỏng: đếm `stats.ipi`).
- Thứ tự chạy các hart trong một tick xoay vòng theo tick để công bằng.

## 5. Khoá kernel lớn (BKL)

Trong một tick, chỉ một hart được vào kernel xử lý syscall. Hart khác nhận `Outcome::Contended`, PC không tăng, thử lại lượt sau (spin). Lệnh máy bình thường (Load/Store) không cần khoá.

| Ưu điểm | Nhược điểm |
|---|---|
| Đúng ngay, không có data race trong kernel | Syscall bị tuần tự hoá giữa các hart |
| Đơn giản, seL4 SMP cũng dùng | Ứng dụng nhiều syscall không tăng tốc theo số lõi |

Test `big_kernel_lock_serializes_syscalls` cho thấy tranh chấp; test `two_harts_run_in_parallel` cho thấy công việc không cần kernel thì song song thật.

**Quan sát từ demo space:** nếu IMU phát ngắt mỗi 4 tick, driver IMU gần như chiếm trọn hart 0 vì vừa xử lý ngắt vừa tranh khoá với hart 1. Giảm tần suất ngắt (8 tick) thì ổn. Bài học: giữ đường syscall cực ngắn, và về lâu dài thay BKL bằng khoá mịn (xem `06-lo-trinh.md`).

## 6. MCS: ngân sách CPU (temporal isolation)

Ưu tiên cố định có một lỗ hổng: thread ưu tiên cao lặp vô hạn làm mọi thread thấp hơn chết đói. MCS (Mixed Criticality Scheduling, như seL4 MCS) thêm **SchedContext**:

```
SchedContext { budget, period, remaining }
- mỗi tick chạy: remaining -= 1
- remaining = 0 → thread bị throttle (không được chọn), kể cả khi ưu tiên cao nhất
- mỗi period: nạp lại remaining = budget (sự kiện trong hàng đợi release)
```

Test `budget_gives_temporal_isolation`: hog ưu tiên 200 lặp vô hạn. Không budget: victim (ưu tiên 10) không chạy được tick nào. Budget 2/10: victim nhận khoảng 80% CPU.

`epoch` của SchedContext tăng mỗi lần cấu hình lại; sự kiện nạp lại cũ bị bỏ qua, tránh nạp hai lần.

Quyền cấu hình SchedContext cần cap **SchedControl**. Profile space xoá cap này sau boot: ngân sách cố định.

## 7. Điểm chú ý
- Thời gian ở mô phỏng là tick rời rạc (1 tick = 1 lệnh). Trên phần cứng, timeslice và budget tính bằng chu kỳ timer thật; kernel phải trừ thời gian đã chạy khi vào kernel do ngắt timer, syscall hoặc fault.
- Thread bị throttle mà đang giữ tài nguyên (đang phục vụ Call) sẽ làm client chờ lâu. seL4 MCS giải bằng cách cho server "mượn" SchedContext của client (passive server). Chưa mô hình hoá.
- Chưa có kế thừa ưu tiên (priority inheritance). Với microkernel kiểu seL4, cách khuyến nghị là thiết kế server ưu tiên cao hơn client (ceiling) hoặc passive server.
