# ViFastOs — Báo cáo vòng 59 (khóa net vào harness gate)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `3a06f89`
**Mốc:** Biến thành quả pha 3 thành yêu cầu gate tự động.

## 1. Thay đổi

- `scripts/harness_spec.py`: thêm `VIRTIO NET READY` vào `ARCH_EXACT_ONE`
  của riscv64 và aarch64 (x86_64 dùng IDE, không có VirtIO net nên không thêm).
- `scripts/test-harness-spec.py`: thêm dòng `VIRTIO NET READY base=..
  accepted=0x10020` vào fixture cả 2 arch (đúng base thật:
  riscv `0x10007000`, arm `0xa003c00`).
- Harness âm giữ 483 cases (chỉ thêm marker bắt buộc, không thêm case mới).

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS), `fmt` PASS, docs PASS.
- QEMU `repeat 3`: 3/3×3 PASS (có 2 lần flaky đơn lẻ x86/arm khi máy tải,
  chạy riêng/lại đều 3/3 — không liên quan thay đổi harness).
- Persistence 7×3 PASS, harness âm 483 PASS.

## 3. Ý nghĩa

Từ vòng này, mọi commit làm mất `VIRTIO NET READY` trên riscv64/aarch64 đều
đỏ gate ngay — pha 3 được khóa bằng test thay vì log mắt thường.
