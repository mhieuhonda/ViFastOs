# Inbox cho agent0

## [vòng 26] AArch64 crash khi thêm bộ đếm pool vào kernel
- Bối cảnh: thêm field HWM vào `Kernel`/static `AtomicU16` và ghi trong
  `alloc_owned` làm AArch64/x86_64 báo `KERNEL EXCEPTION vector=4` ngay sau
  `BOOTINFO OK` (RISC-V vẫn xanh). Revert toàn bộ giữ baseline.
- Các phương án: (a) tính HWM hoàn toàn ngoài kernel (runtime sample tại các
  mốc), (b) chỉ đo current counts, (c) debug riêng AArch64 với static tối thiểu.
- Đề xuất agent2: (c) trước, vì đây có thể là lỗi nền về vị trí static/AT.
- Đang chặn: M1.1 (POOL HWM) | Không chặn M0.2/M1.2
- Nhánh đang đi trong lúc chờ: M0.2 (xong) + M1.2.
- Đã xử lý phần ROOT ERR 16: nguyên nhân là **runtime vẫn index CONTEXTS theo
  `tcb.index`** sau khi mảng đổi sang MAX_TCBS; ConfigureTcb dùng get_mut(OOB)
  nên trả IllegalOperation. Sửa runtime dùng `context_slot` (commit M1.2c).
  HWM AArch vẫn chưa thử lại sau sửa này.

## [vòng 26] Ngân sách pager v2
- Bối cảnh: data 368 KiB + `MAX_OBJECTS` 163 không khả thi với headroom.
- Đề xuất: tách pool object (M1) để giảm `Object` từ 4.752 xuống ≤256 B.
- Đang chặn: M2 pager v2 | Nhánh: M1 → M2.
