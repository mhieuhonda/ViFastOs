# ViFastOs — Báo cáo vòng 33 (check tràn static AArch64 trong build)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `3547159`
**Mốc:** Chốt pha 1 mục tiêu dài hạn — guard tràn static ở cả runtime và build.

## 1. Thay đổi

`scripts/build.sh`: sau khi build AArch64, đo `KERNEL_STORAGE` (địa chỉ +
size từ `llvm-nm --print-size`) và `BKL`; fail nếu storage đè lên BKL.

Log build in: `AArch64 static check storage=... end=... bkl=...`.
Hiện tại: end=1075051112, bkl=1075051120 — vừa khít 8 byte, PASS.

Cùng với `KERNEL_STATIC_BUDGET` trong runtime (vòng 30), giờ có 2 lớp guard:
runtime exit rõ trên QEMU, build fail sớm trên host.

## 2. Số liệu cổng

- Host: kernel **134** → **203** tổng; build 0 warning.
- QEMU chuẩn **3/3 ×3**; persistence 7×3 PASS; harness **462**.
- `fmt --check` PASS; `check-docs.py` PASS.

## 3. Pha tiếp theo

Pha 2 mục tiêu dài hạn: pager v2 tổng quát + demand-load
(`docs/design/pager-v2.md`). Cần cấp PageTable/VSpace cho pager và nạp trang
từ file qua VFS — hiện pager server chỉ phục vụ 2 client demo với page table
pre-map.
