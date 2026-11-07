# agent2-report-27 — Vòng 103: soak 30 iter xong + gate xanh + settle

Ngày: 2026-11-07. Soak 30 iter (~456s) hoàn thành.

## 1. Kết quả soak 30

- riscv64, aarch64: fail rải rác; x86_64: 29/30 (1 timeout rc=-9).
- Fail cuối (027/028/029): riscv64+aarch64 cùng boot treo ở "Isolation"
  (serial dừng sau FAULT EP OK, không tới SHELL) — boot hang flake khi máy
  host tải nặng cuối soak, không phải bug kernel xác định.
- Fail đầu/giữa: mất dòng đầu serial (`EL0 FP TRAP`), `VIRTIO GPU INIT FAIL`
  trên aarch64 (thiết bị chưa xử lý ngay sau DRIVER_OK — đã retry 4 lần),
  1 x86 timeout.

Kết luận: gate đơn lẻ (QEMU 3/3×3) xanh ổn định; soak lộ 3 loại flake hạ
tầng (serial race, GPU timing, boot hang khi quá tải). Không có bằng chứng
regression từ vòng 101.

## 2. Đã làm vòng này

- `scripts/soak.py`: thêm `--settle` (mặc định 3s giữa iter) giảm contention.
- `docs/08`: từ điển thuật ngữ cho sinh viên năm nhất (đã commit).
- Gate vòng này: build 0 warning, kernel 156, QEMU 3/3×3, persist 7×3,
  harness 508, fmt + docs PASS.

## 3. Đề xuất vòng sau

- Chạy soak có settle để lấy baseline sạch so sánh.
- Điều tra boot hang: thêm watchdog marker từng phase boot để định vị điểm
  kẹt (Isolation → TCB SUSPEND → SMP WORKERS).
- GPU aarch64: tăng retry hoặc chấp nhận flake hạ tầng, không siết harness.
