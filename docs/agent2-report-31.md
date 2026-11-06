# ViFastOs — Báo cáo vòng 31 (giảm MAX_CAPS_TASK 64→48)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `6dd4e34`
**Mốc:** Cải tiến bộ nhớ an toàn đầu tiên sau khi giải blocker layout.

## 1. Thay đổi

Giảm `MAX_CAPS_TASK` 64 → 48 (`crates/vifast-kernel/src/object.rs`).

Cơ sở audit:
- User dùng tối đa slot 11 (client IRQ/UART, pager server 1–6, worker 1–3).
- CNode demo `mint` dest slot 1.
- Host test dùng tới slot 36 (`untyped_free_list_full_counts_leak`).
- 48 giữ headroom ~12 slot cho test/user mới.

## 2. Kết quả đo

| Hạng mục | Trước | Sau | Tiết kiệm |
|---|---:|---:|---:|
| `Object` | 4.752 | 3.728 | 1.024 |
| `Tcb` | 4.696 | 3.672 | 1.024 |
| `CNode` | 4.096 | 3.072 | 1.024 |
| `Kernel` | 710.248 | 559.720 | 150.528 |
| headroom RISC-V | ~290.128 | ~440.656 (ước tính) | +150.528 |

`Kernel` 559.720 < `KERNEL_STATIC_BUDGET` 716.800 — an toàn tuyệt đối.

## 3. Số liệu cổng

- Host: kernel **134**, abi 4, arch 7, pte 11, elf 17, acpi 8, fs 22 → **203**.
- QEMU chuẩn **3/3 ×3**; persistence 7×3 PASS; harness **462**; warning 0.
- `cargo fmt --check` PASS; `check-docs.py` PASS.

## 4. Docs cập nhật

- `docs/00-hien-trang.md`, `docs/11-hien-tai/02-objects-capabilities.md`,
  `README.md` (2 chỗ): 64 → 48 cap task / CNode slot.
- Summary section 18 cần thêm mục Vòng 31 (làm lúc bundle).
