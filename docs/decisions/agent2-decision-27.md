# ViFastOs: agent2 tự quyết định vòng 27

Dựa trên: agent2 self-review vòng 26, HEAD `91296ee`, hướng dẫn autonomous.
Mốc hiện tại: **M0 → M1**.

## 0. Tự review vòng 26 (checklist 6.2)

| Mục | Đạt? | Hành động |
|---|---|---|
| HISTORICAL không bị sửa nội dung | Chưa | C1 vòng 27 sửa lại |
| DONE docs có grep chứng minh | Chưa | C1/C2 dán grep |
| Danh sách tay trùng generated | Chưa | C2 |
| Status từ repeat 3 | Đạt | — |
| pass render đủ điều kiện | Đạt | — |
| 1 task = 1 commit | Đạt (C4) | — |
| git log đủ commit | Chưa | báo cáo 27 dùng script |
| BLOCKED có thử | Đạt | — |
| Không so số khác kiến trúc | Đạt | — |
| Không né SMP không lý do | Đạt | — |
| Design doc không lộ PA/index | Đạt (C4) | — |
| Bảng footprint 3 kiến trúc | Chưa | M1.1 |

## 1. Việc của vòng 27

| ID | Việc | Cổng | Commit dự kiến |
|---|---|---|---|
| M0.2-C1 | Sửa HISTORICAL/tổng 4, nguồn gốc 0x1001000000/0x1001001000, boot Untyped | docs | `docs: correct fault provenance and stale boot untyped text` |
| M0.2-C2 | Khối limits về mục 7, ghi crate, MAX_FDS/MAX_FD | docs | `docs: qualify generated limits by crate` |
| M0.2-C3 | Status từ `target/qemu-status-standard.json` | docs + check-all | `test: pin docs status to standard run artifact` |
| M1.1 | Đo variant + Object/Kernel; HWM nếu AArch cho phép | code gate | `test: measure object variant sizes and pool high-water marks` |
| M1.2 | Host test chuỗi boot + bước a/b context_slot | code gate | `test: reproduce boot chain for pool id` |

## 2. Quyết định tự đưa ra

- C1 đã xong trước khi viết file này (commit `91296ee`), giữ 1 task 1 commit.
- M1.1 tách HWM ra khỏi `alloc_owned`; nếu AArch vẫn đỏ thì ghi BLOCKED + inbox,
  chỉ commit phần host test đo variant.

## 3. Điều kiện dừng

- AArch đỏ 2 phiên liên tiếp với HWM → inbox, chuyển M1.2 host test và M0.2-C3.
