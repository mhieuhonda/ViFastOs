# agent2-report-30 — Vòng 104: soak có settle 10/10 xanh cả 3 arch

Ngày: 2026-11-07. Soak `--iterations 10 --settle 5`: 10/10 PASS cả 3 arch
(~197s). So với soak 30 không settle (fail rải rác: mất byte đầu serial,
GPU timing aarch64, boot hang cuối đợt) — xác nhận flake do contention
QEMU/host khi chạy back-to-back, không phải regression kernel.

## 1. Bằng chứng

- `scripts/soak.py --settle` (vòng 102): nghỉ 5s giữa iter.
- Kết quả: riscv64 10/10, aarch64 10/10, x86_64 10/10.
- Gate chuẩn vẫn xanh: kernel 156, QEMU 3/3×3, persist 7×3, harness 508.
