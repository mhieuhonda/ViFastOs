# ViFastOs — Báo cáo vòng 82 (P1-1 reclaim Untyped + P1-4 hash nguồn)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `c2fb57e`
**Bối cảnh:** tiếp tục các mục P1 từ audit (`docs/audits/2026-11-07-round79.md`).

## 1. P1-1 — rò rỉ byte Untyped khi retype thất bại

- Trước: `retype_untyped_as` lấy byte bằng `untyped_take` rồi mới `alloc_owned`;
  nếu bảng object đầy (MAX_OBJECTS=147) thì trả `OutOfMemory` **mà không trả
  byte** — mỗi lần retype hỏng đốt vĩnh viễn 4 KiB. Cùng lỗi ở
  `retype_untyped_child_as`.
- Sau: mọi nhánh lỗi sau `untyped_take` (checked_add, vượt end, alloc_owned)
  gọi `untyped_reclaim(untyped, offset, charge)`.
- Test host mới `failed_retype_returns_the_carved_bytes`: tạo Untyped giới hạn
  1 object, retype thành công 1 frame, retype lần hai trả `OutOfMemory` và
  `untyped_usage()` không đổi (bump và số object giữ nguyên).
- `cargo test -p vifast-kernel`: **135 passed**, 0 failed.

## 2. P1-4 — check-docs không còn phụ thuộc mtime

- Trước: `gen-docs.py` so `mtime` của `qemu-status-standard.json` với
  `vifast-build.log`; build lại (dù không đổi mã) làm cổng docs đỏ.
- Sau: `test-qemu.py` ghi `source_sha256` = hash nội dung toàn bộ mã ảnh hưởng
  kernel (mọi `.rs/.S/.toml/.ld` trong `crates/` + `scripts/build.sh`, sắp xếp
  theo tên tương đối). `gen-docs.py` tính lại và so sánh:
  - mã đổi → `gen-docs: kernel sources changed since the standard QEMU run`.
  - rebuild không đổi mã → **PASS** (đã kiểm chứng).
  - JSON cũ không có hash → quay về so mtime như trước.

## 3. Kiểm chứng

- Thêm `crates/.probe.rs` → check-docs báo "sources changed"; xoá → PASS.
- `./scripts/build.sh` lại lần nữa (không đổi mã) → check-docs PASS.
- QEMU 3/3 × 3, harness 504, persist 7×3, fmt, docs — **all steps passed**.

## 4. Ghi chú

- Trong một lần chạy trước khi chốt vòng, riscv64 có 2/3 (flake cũ, lần chạy
  sau 3/3). Theo dõi tiếp ở các soak sau; nếu tái hiện sẽ đào serial.
