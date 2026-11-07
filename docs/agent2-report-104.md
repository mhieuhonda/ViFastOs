# ViFastOs — Báo cáo vòng 104 (đo BKL contention, định hướng khóa mịn)

**Ngày:** 2026-11-08. **HEAD:** `3a55605`.

## 1. Số liệu BKL từ serial QEMU mới nhất

| Arch | h0_acq / h0_spin | h1_acq / h1_spin | spin/acq hart1 |
|---|---|---|---|
| riscv64 | 137963 / 38723 | 58793 / 272497 | ~4,6 |
| aarch64 | 81643 / 57757 | 1859 / 459124 | ~247 |

Hart phụ aarch64 mỗi lần acquire spin trung bình ~247 vòng — contention rõ.
`pick_on`/runq/scheduler hiện chạy trong BKL platform (kernel portable không
có lock riêng), nên mọi quyết định schedule đều qua một ticket lock chung.

## 2. Quyết định mốc 5 (IPC/sched mở rộng)

- Không tách lock mù: mẫu đúng đã có (console Tx spinlock vòng 105 — lock
  hẹp, bound, tránh deadlock IRQ).
- Thứ tự tách: (1) per-hart runq lock (enqueue/pick theo hart, ít tranh nhất
  vì work-steal hiếm); (2) endpoint queue lock riêng; (3) đo lại spin/acq —
  chỉ khi vẫn cao mới tách tiếp.
- Điều kiện tiên quyết: SMP ≥4 hart + soak dài để đo (hiện mới 2 hart).
  Với 2 hart hiện tại, BKL vẫn đủ — ghi nhận số liệu baseline ở đây.

## 3. Mốc 4 (driver IRQ-driven)

Block (VirtIO-MMIO + ATA PIO) và net (TX/RX queue) hiện poll đồng bộ có
bound — ổn định qua gate/soak, chưa có repro fail. Không đụng driver khi
chưa có bằng chứng lỗi; IRQ-driven để sau mốc 5 (cần lock riêng cho hàng
đợi device trước).
