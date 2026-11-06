# ViFastOs — Báo cáo vòng 36 (nâng trần slot root 96→128)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `de3bef5`
**Mốc:** Mở đường pager v2 (PAGER3/VFS) — hết slot root là blocker vòng 34.

## 1. Thay đổi

`MAX_CAPS_ROOT` 96 → 128 (`crates/vifast-kernel/src/object.rs`).

Lý do: root spawn pager/VFS cần ~50 slot (TCB/stack/frame/table/endpoint/
untyped...); trần 96 cạn trên AArch64/x86 khi thêm PAGER3 (`root slot range
exhausted`). Tăng root rẻ hơn tăng task: chỉ 1 root cspace (+32 slot × 64 B
≈ 2 KiB vào `Kernel`), trong khi tăng task tốn ×147 object.

Kết quả đo: `Kernel` 559.720 → 561.768 (+2.048 B), vẫn dưới
`KERNEL_STATIC_BUDGET` 716.800 và build check AArch64 PASS.

## 2. Số liệu cổng

- Host: kernel **134** → **203** tổng; build 0 warning (+check static PASS).
- QEMU chuẩn **3/3 ×3**; persistence 7×3 PASS; harness **471**.
- `fmt --check` PASS; `check-docs.py` PASS.

## 3. Việc tiếp theo (pha 2)

Thử lại PAGER3 (stash `wip pager3 client`) và cấp VFS cap cho pager để nạp
file — giờ đã có slot.
