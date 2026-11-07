# ViFastOs — Báo cáo vòng 101 (soak 30/30 + harness, suite ổn định)

**Ngày:** 2026-11-07
**Mốc:** soak/stress test (xác nhận cuối cùng).

## 1. Soak 30 iter có settle + harness mỗi vòng: 30/30 cả 3 arch

`soak.py --iterations 30 --settle 3 --with-harness` (~543s):
riscv64 30/30, aarch64 30/30, x86_64 30/30, harness checked mỗi vòng —
all passed. So với soak-30 không settle trước fix (fail rải rác hạ tầng)
và soak-20 verify (20/20 sau console lock), đây là lần soak dài nhất sạch
hoàn toàn.

Kết luận: `--settle` + spinlock console Tx đã triệt tiêu mọi flake đã biết
(mất byte đầu serial, GPU timing aarch64, dính ký tự SMP x86, boot hang
khi quá tải).

## 2. Gate giữ xanh

Kernel 156, QEMU 3/3×3, persist 7×3, harness 508, fmt + docs PASS.
