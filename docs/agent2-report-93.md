# ViFastOs — Báo cáo vòng 93 (NX cho static kernel + input end-to-end)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `2628cf9`
**Bối cảnh:** audit #6 (câu 1) và mốc "virtio-input tương tác" của mục tiêu.

## 1. NX: bổ sung static kernel còn thiếu

`kernel_data_ranges()` chỉ liệt kê 14 vùng; nhiều static quan trọng không có
mặt nên vẫn **executable** trong kernel space dưới NX:

| Static | Vai trò |
|---|---|
| `AP_GDT` | GDT của AP (dữ liệu) |
| `GDT_PTR`, `IDT_PTR`, `AP_GDT_PTR` | con trỏ bảng mô tả (dữ liệu) |
| `CPU_STATES` | trạng thái per-hart (dữ liệu) |

→ thêm 5 entry (mảng `[(u64, usize); 14]` → `19`). `KERNEL DATA NX OK` vẫn in
đúng một lần; x86_64 2/2 PASS.

## 2. Input end-to-end: phím phải tới **user shell**

`--input-check` trước đây chỉ đòi `VIRTIO KEY 18` — tức bằng chứng **driver**
nhận phím, chưa chứng minh phím tới được user space.

- Thêm yêu cầu thứ hai: shell echo ký tự (`shell> e`), tức chuỗi
  device → virtio-input driver → kernel IRQ → `irq_raise` → notification →
  `console_read()` của client → shell echo ra serial.
- Thông điệp lỗi giờ tách `driver=` và `echo=` để biết đứt ở đâu.
- riscv64 1/1 PASS, aarch64 2/2 PASS với kiểm tra mới.

## 3. Vì sao chưa làm "display tương tác"

Cập nhật framebuffer ngay trong IRQ handler của virtio-input sẽ gọi
`gpu_command` (queue poll đồng bộ) trong ngữ cảnh ngắt đang giữ BKL — rủi ro
deadlock/timeout. Hướng đúng là một **display service** user-space hoặc hàng đợi
việc làm deferred; ghi nhận cho vòng sau thay vì làm ẩu trong 1 vòng.

## 4. Cổng

- `gate.sh --with-tcp --with-input`: all steps passed.
- Kernel 141 test, FS 25 test, harness 508 case.
