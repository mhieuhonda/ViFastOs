# ViFastOs — Báo cáo vòng 103 (đo static budget, chốt hướng mốc 2)

**Ngày:** 2026-11-08. **HEAD:** `7af7a01`.

## 1. Số liệu đo (`size_of`, host)

- `Kernel` = 562944 / budget 566272 → còn trống **3328 B** (gap AArch64: 8 B).
- `Tcb` = 3680 = max `Kind` → `Object` = 3736.
- `Tcb.cspace` = 48 × `Option<Cap>`(64 B) = **3072 B** (83% Tcb).
- `PendingSend` = 120, `Message` = 96; `VSpace` = 3136 (64 mappings);
  `Cap` = 64, `CapRef` = 24, `Frame` = 24, `Endpoint` = 200.

## 2. Quyết định mốc 2

- Phung phí lớn nhất là cspace nhúng 48 slot mọi TCB (3072 B × 18 TCB
  trong `Kind::Tcb`, quyết định size `Object` × 147). Nới object table/TCB
  quota/queue/CNode đa cấp đều đụng chỗ này.
- Chưa đụng code: budget còn 3,3 KiB đủ cho các mốc lõi trước mắt
  (TcbWait/syscall 51 vừa thêm không vượt). Khi cần field mới cho TCB
  (vd. owner cho destroy-cả-nhóm), phải refactor cspace trước:
  cspace nhúng nhỏ lại + CNode ngoài mặc định, hoặc sparse map.
- Destroy-cả-nhóm hoãn sang mốc 2 cùng đợt refactor owner/budget.
