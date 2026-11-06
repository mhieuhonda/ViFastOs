# ViFastOs — Báo cáo vòng 29 (baseline + đo layout, khoanh vùng kẹt AArch64)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `d93e0e6`
**Mốc:** M1.2c giữ xanh, M1.1/M1.3 vẫn BLOCKED, thêm bước đo layout an toàn.

## 1. Bảng task

| ID | Trạng thái | Commit | Bằng chứng |
|---|---|---|---|
| Baseline | DONE | — | build 0 warning, host 203, QEMU 3/3×3, persist 7×3, harness 462 |
| Layout probe | DONE | (vòng này) | `LAYOUT objs=0 root_cspace=698544 current=705552 size=710248` |
| CNode pool WIP | STASH | `stash@{0}` | RISC-V 3/3 nhưng AArch64 0/3 → tách riêng, không commit |
| TCB pool (M1.3 lần 2) | SKIPPED | — | chờ khoanh vùng AArch64 xong |
| Docs/bundle | DONE | (vòng này) | report + overnight-log + bundle |

## 2. Phát hiện chính

- Cây dở dang CNode pool (`Kind::CNode(CNodeId)` + `Kernel.cnodes[8]`):
  RISC-V 3/3 PASS nhưng **AArch64 0/3** `KERNEL EXCEPTION vector=4
  (cur-spx-sync) ... ELR=0x4009201c FAR=0xb4410` ngay sau `BOOTINFO OK`.
  Baseline sạch AArch64 1/1 PASS → lỗi do thay đổi layout, không phải logic.
- `Object=4.752, Tcb=4.696, CNode=4.096, VSpace=3.136, Kernel=710.248`.
  Tách CNode riêng lẻ **không giảm Object** (CNode vẫn 4.096 trong pool) mà
  còn phình Kernel 710.248 → 743.016 (+32.768 = 8×4.096). Hướng đúng là giảm
  `MAX_CAPS_TASK` (mỗi slot 64 B × 147 object) hoặc tách Tcb/VSpace, không
  phải tách CNode đơn lẻ.
- `Kernel` layout hiện tại: `objs=0, root_cspace=698544, current=705552,
  size=710248`. Mọi thay đổi thêm field vào `Kernel` đều dịch `.bss` trên
  AArch64 — đây là manh mối cho cả 2 blocker M1.1 (HWM) và M1.3 (TCB pool).

## 3. Số liệu cổng (với bước layout probe)

- Host: kernel **134**, abi 4, arch 7, pte 11, elf 17, acpi 8, fs 22 → **203**.
- QEMU chuẩn **3/3 ×3** cả 3 arch; persistence 7×3 PASS; harness **462**.
- Warning **0**; `cargo fmt --check` PASS.

## 4. Việc tiếp theo

1. Thử giảm `MAX_CAPS_TASK` 64→48 có kiểm chứng QEMU (tiết kiệm ~1 KiB/Tcb).
2. Hoặc tách `Tcb.cspace`/`fds` khỏi `Tcb` trước khi tách toàn bộ TCB pool.
3. Debug AArch64 layout: in `__kernel_end`/`.bss` range khi thêm field, so với
   `kernel_accessible_range` 0x4000_0000..0x4080_0000.
