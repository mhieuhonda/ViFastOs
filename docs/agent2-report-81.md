# ViFastOs — Báo cáo vòng 81 (siết guard static AArch64)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `71d34a7`
**Bối cảnh:** mục P1-3 của audit (`docs/audits/2026-11-07-round79.md`): ngân
sách static được runtime kiểm tra lỏng hơn nhiều so với giới hạn thật.

## 1. Vấn đề

- `KERNEL_STATIC_BUDGET` = 716_800 trong khi `Kernel` thực tế là 561_768 →
  runtime còn cho phép `Kernel` phình ~155 KB.
- Trên AArch64, `KERNEL_STORAGE end` chỉ còn cách `BKL` **8 byte**; thêm một
  field nhỏ vào `Kernel` là chạm `BKL` (build.sh có kiểm tra nhưng chỉ in một
  dòng, không ai để ý).

## 2. Thay đổi

- `KERNEL_STATIC_BUDGET` = **566_272** (= 561_768 + 4_504), kèm chú thích nêu
  rõ vì sao, để runtime (`KERNEL STATIC OVERFLOW`) chặn sớm.
- `scripts/build.sh`: in thêm `gap=` giữa `KERNEL_STORAGE end` và `BKL`, cảnh
  báo `KERNEL STATIC GAP WARNING` khi dưới 4 KiB (hiện tại: 8 byte). Vẫn giữ
  kiểm tra cứng `end >= BKL` → build fail.

## 3. Kiểm chứng

- `./scripts/build.sh`: `AArch64 static check ... gap=8` + warning như thiết kế.
- QEMU 3/3 × 3, harness 504 case, docs/fmt xanh.
