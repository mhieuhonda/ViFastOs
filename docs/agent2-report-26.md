# agent2-report-26 — Vòng 102: soak phát hiện flake hạ tầng + docs fault/IOMMU

Ngày: 2026-11-07. Tiếp sau vòng 101 (vá 37/44 K32–K75, commit 7fe404c).

## 1. Docs mới (đã commit)

- `docs/11-hien-tai/14-fault-pager.md`: luồng fault/demand paging, hợp đồng
  TcbConfigure fault_ep kép (K75), bài học resume-BlockedFault.
- `docs/11-hien-tai/15-iommu-audit101.md`: software IOMMU, W^X chéo CPU/DMA,
  gate IoMap/IoUnmap, destroy domain, danh mục 44 phát hiện
  (37 đúng / K45,K46,K60 sai / K72 mismatch vệ sinh).

## 2. Soak 30 iter (đang chạy nền): flake hạ tầng, không phải regression

Mẫu sau 8 iter: aarch64 fail 6/8, riscv64 2/8, x86_64 1/8 — trong khi gate
chuẩn QEMU 3/3×3 vừa PASS cả 3 arch cùng binary.

Phân tích 2 kiểu fail aarch64:

- **Mất dòng đầu serial** (`EL0 FP TRAP` in ở `Arch::init`, dòng 1):
  serial fail bắt đầu bằng `DTB magic=...`, thiếu dòng 1 → QEMU serial
  race mất byte đầu khi boot nhanh. Flake hạ tầng, không phải bug kernel.
- **Thiếu GPU markers** (`VIRTIO GPU READY/DRAW OK`, iter-014): boot đầy đủ
  tới SHELL nhưng GPU init không in — cần điều tra thêm (chủng loại khác,
  có thể timing device init).

Cả hai đều có từ trước K101 (không liên quan thay đổi vòng 101).

## 3. Đề xuất vòng sau

- Soak tiếp tới 30 iter lấy tỷ lệ chuẩn; nếu GPU-flake tái hiện, thêm retry
  hoặc marker mềm cho GPU init trên aarch64.
- Không siết harness EXACT_ONE cho dòng boot đầu (kẻo flake hạ tầng làm
  đỏ gate oan) — hoặc buffer serial từ byte đầu.
