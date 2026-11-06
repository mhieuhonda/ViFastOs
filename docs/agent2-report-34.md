# ViFastOs — Báo cáo vòng 34 (pager v2 bước 1: evict/unmap + fault lại)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `0df066a`
**Mốc:** Pha 2 mục tiêu dài hạn — bước nhỏ đầu tiên của pager v2.

## 1. Thay đổi

Pager server sau khi phục vụ `PAGER2` lần đầu (client đọc xong) thì `FrameUnmap`
trang `PAGER2_VA`; client đọc lại → fault lần 2 → server `FrameMap` lại cùng
frame → resume. Marker mới:
- `PAGER EVICT OK` (exact-one) khi lần 2 đọc đúng giá trị.
- `PAGER EVICT BAD` (forbidden) khi sai.
- Fault count 6 → 7 (`PAGER2_VA=0x1001001000` xuất hiện 2 lần); harness kiểm
  `faults.count(0x1001001000) == 2`.
- Fixture `test-harness-spec.py` thêm fault lặp; 462 → **471 cases**.

PAGER3 client đã thử nhưng stash riêng (hết slot root trên AArch64/x86:
`root slot range exhausted` — mỗi pager tốn 3 slot + page table; xem stash
`wip pager3 client`). Evict/unmap là bước pager v2 không tốn thêm slot.

## 2. Số liệu cổng

- Host: kernel **134** → **203** tổng; build 0 warning.
- QEMU chuẩn **3/3 ×3**; persistence 7×3 PASS; harness **471**.
- `fmt --check` PASS; `check-docs.py` PASS.

## 3. Việc tiếp theo (pha 2)

Cấp Untyped con + VSpace/PageTable cap cho pager server để `Retype` frame mới
sau evict (hiện map lại frame cũ), rồi nạp trang từ file qua VFS.
