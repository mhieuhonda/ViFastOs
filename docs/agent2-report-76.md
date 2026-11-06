# ViFastOs — Báo cáo vòng 76 (ổn định GPU aarch64 + tài liệu VirtIO)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `5a09244`

## 1. Flake tìm được bằng soak

Soak 15 vòng (mỗi vòng 3 arch + harness) cho kết quả:
- riscv64 15/15, x86_64 15/15, **aarch64 7/15**.
- Mọi lần hỏng đều cùng một kiểu: `VIRTIO GPU INIT FAIL` (lệnh
  `GET_DISPLAY_INFO` không hoàn tất trong bound 10M spins), sau đó thiếu
  `VIRTIO GPU READY`/`DRAW OK` → harness fail.

Đây là lỗi thật của driver: lệnh đầu tiên gửi ngay sau `DRIVER_OK`, thiết bị
aarch64 có lúc chưa xử lý kịp.

## 2. Sửa

- `gpu_command_bounded(cmd, payload, bound)`; `gpu_command` dùng bound 10M như cũ.
- `gpu_init` thử `GET_DISPLAY_INFO` tối đa **4 lần**: mỗi lần chờ ~400k spins
  rồi gọi với bound 2M. Tổng worst-case thấp hơn một lần bound 10M, nhưng có
  cơ hội thành công khi thiết bị chậm nhịp.

## 3. Kiểm chứng

- 10 lần chạy aarch64 liên tiếp: **10/10 PASS**, lần nào cũng `VIRTIO GPU READY`.
- `scripts/gate.sh`: all steps passed.
- Soak sạch 8 vòng: riscv64 8/8, aarch64 8/8, x86_64 8/8, harness 504 mỗi vòng.

## 4. Tài liệu

- Thêm `docs/11-hien-tai/13-virtio.md`: mô tả toàn bộ driver VirtIO-MMIO
  (block, net, gpu, input, rng), trình tự khởi tạo đúng, các lỗi đã trả giá
  (QueueReady, header 10 byte, kick/re-post, attach trước DRIVER_OK) và bảng
  marker/test tương ứng. Liên kết từ `docs/11-hien-tai/00-index.md`.
