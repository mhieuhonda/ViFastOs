# ViFastOs — Báo cáo vòng 100 (soak-verify 20/20 + spinlock console Tx)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `7a45f30`
**Mốc:** soak/stress test (xác nhận fix vòng 105).

## 1. Soak-verify 20 iter có settle: 20/20 cả 3 arch

Sau fix spinlock console Tx (vòng 105), soak lại 20 iter (`--settle 3`):
riscv64 20/20, aarch64 20/20, x86_64 20/20 (~359s, all passed).
So với soak-20 trước fix (x86 18/20, 2 fail dính ký tự serial SMP) —
xác nhận fix triệt tiêu flake.

## 2. Fix vòng 105 (đã commit `7a45f30`)

- `console::log`: AtomicBool spinlock quanh cả dòng + bound 1M spins
  (tránh deadlock IRQ-handler-klog). Portable trong `vifast-kernel`
  no_std, cover cả 3 arch.
- Gate sau fix: kernel 156, QEMU 3/3×3, persist 7×3, harness 508, fmt +
  docs PASS.

## 3. Còn lại

- Boot hang cuối soak-30 cũ (treo ở Isolation khi quá tải): chưa tái hiện
  có settle; giữ watchdog-marker đề xuất nếu tái hiện.
- GPU aarch64 `INIT FAIL` khi tải nặng: flake hạ tầng, không siết harness.
