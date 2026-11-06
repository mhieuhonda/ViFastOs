# ViFastOs — Báo cáo vòng 67 (M2 demand-load hoàn thành)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `0859af2`
**Mốc:** **Demand-load hoạt động** — trang chỉ được cấp + nạp nội dung khi
fault, đóng mục cuối của M2.

## 1. Thiết kế

- `PAGER4_VA = PAGER_VA + 0x3000`: trang **chưa map**, frame **chưa có nội dung**.
- `pager3_client` đọc `PAGER4_VA` sau khi xong PAGER3 → fault.
- `pager_server` nhận fault, tìm đúng VA, ghi `PAGER4_IMAGE` (chỉ tồn tại
  trong backing image của server) vào frame **tại thời điểm fault**, map
  READ vào VSpace của client, rồi resume TCB.
- Client đọc lại, gửi giá trị về; server so với image → `DEMAND LOAD OK`.

Điểm cốt lõi: nội dung trang không hề tồn tại trong bộ nhớ client trước
fault; nó được "nạp" từ backing store của server ngay khi có yêu cầu.

## 2. Vấn đề gặp và cách xử lý

1. **Cap frame thiếu quyền WRITE** → `frame_write` fail im lặng, client đọc 0.
   Sửa: cấp `READ | WRITE` (map vẫn dùng READ).
2. **x86_64 hết Untyped** (page table 4 cấp): không đủ frame mới →
   `OutOfMemory` làm root panic. Sửa: quyết định theo `cfg!(target_arch)`,
   x86 dùng client rút gọn (`pager3_only_client`), server bỏ qua copy cap.
3. **Harness quá chặt**: số fault dao động vì evict có thể map lại trước khi
   client đọc lần 2. Sửa: kiểm tra **sự hiện diện** của từng địa chỉ
   (`0x1001003000` bắt buộc ở riscv/aarch64, cấm ở x86), thay vì tổng số.
4. Thêm `VIFAST_KEEP_SERIAL=1` để giữ serial từng lần chạy khi điều tra flake.

## 3. Kết quả

- riscv64 + aarch64: `DEMAND LOAD OK` (1 lần, exact-one theo arch).
- Fault mới `0x1001003000` xuất hiện đúng 1 lần trên 2 arch virtio.
- Harness âm 489 → **494** cases.

## 4. Cổng

- QEMU 3/3×3 (chạy 2 lần liên tiếp đều xanh), persist 7×3, harness 494,
  host 207, build 0 warning, `fmt` PASS, docs PASS.
