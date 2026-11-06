# Ngân sách bộ nhớ để mở pager v2 (đề xuất vòng 26)

Trạng thái: **đề xuất**, chưa code. Mục tiêu: tìm đường hợp lệ cho data Untyped
368 KiB + `MAX_OBJECTS` đủ object cho pager mà vẫn giữ headroom RISC-V
≥ 196.608 byte.

## 1. Vì sao đề xuất cũ bất khả thi

- Data 304→368 KiB: −65.536 headroom → còn 187.728 < guard.
- `MAX_OBJECTS` 147→163: Object 4.752 byte × 16 = −76.032 nữa → 144.464.
- Giữ 147 slot thì data max_objects chỉ 76; pager cần ~16 object, chỉ trống
  6–10 sau khi reap fault TCB.

## 2. Hai nguồn bộ nhớ có thể giải phóng

1. **CONTEXTS theo object index → dense TCB index.** Hiện mỗi platform giữ
   `[TrapFrame; 147]`; chỉ cần `MAX_TCBS` (22) phần tử nếu `Tcb` lưu
   `context_slot: u8` cấp phát/giải phóng theo pool. Tiết kiệm khoảng
   `(147−22) × size_of::<TrapFrame>()` (đo thực tế khi làm; ước 60 KiB).
2. **Giảm `MAX_CAPS_TASK` 64→48.** `Tcb.cspace` chiếm 4.096 byte; giảm 16 slot
   tiết kiệm 1.024 byte/Tcb, kéo `Object` từ 4.752 xuống ~3.728. Với 163
   object, tiết kiệm ~90 KiB so với 147×4.752. Cần kiểm tra user hiện dùng
   slot tối đa (client ≤ 7) và cập nhật docs/ABI.

## 3. Phương án đề xuất

| Bước | Thay đổi | Headroom ước tính |
|---|---|---:|
| 0 | hiện tại | 253.264 |
| 1 | dense CONTEXTS | ~313.000 |
| 2 | data +64 KiB, BOOT_POOL handoff −32 KiB | ~281.000 |
| 3 | `MAX_CAPS_TASK` 48 (Object ~3.728) | ~371.000 |
| 4 | `MAX_OBJECTS` 163 (quota 5+34+92+32) | ~295.000 |

Kết quả: đủ data 92 trang/92 object cho pager, vẫn ≥ 212.992 mục tiêu.
Bước 1–2 độc lập, có thể làm trước; bước 3–4 cần một vòng riêng vì đổi
layout object/cap.

## 4. Rủi ro

- Dense context index phải cấp/giải phóng cùng TCB; TCB destroy hiện reparent
  cap, cần trả context slot.
- Giảm cap task ảnh hưởng test cspace; phải grep toàn bộ slot > 47.
- `Object` size đổi làm mọi mốc LAYOUT/headroom trong docs phải sinh lại.
