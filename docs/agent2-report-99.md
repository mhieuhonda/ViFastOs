# ViFastOs — Báo cáo vòng 99 (gate full tcp+input xanh + soak settle)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `b751fe4`
**Mốc:** soak/stress test + tài liệu + virtio-input/display tương tác.

## 1. Gate full `--with-tcp --with-input`: PASS toàn bộ

- build 0 warning, kernel 156 test, QEMU 3/3×3, persist 7×3, harness 508,
  fmt + docs PASS (`gate summary: all steps passed`).
- Input tương tác: `test_interactive` gửi phím qua HMP, yêu cầu đồng thời
  `VIRTIO KEY 18` (driver) + `shell> e` (shell echo) + `INPUT ECHO 18`
  (vẽ framebuffer) — chứng minh đường device → driver → kernel →
  user `console_read` → display end-to-end (vòng 93/95).
- TCP: `VIRTIO TCP LISTEN OK` + echo qua hostfwd.

## 2. Soak có settle: 10/10 xanh cả 3 arch

Soak 30 không settle lộ 3 flake hạ tầng (mất byte đầu serial, GPU timing
aarch64, boot hang khi quá tải — xem report-28). Soak 10 với `--settle 5`
(pass mới ở vòng 102): 10/10 cả 3 arch, xác nhận flake do contention.

## 3. Docs vòng này

- `14-fault-pager.md`, `15-iommu-audit101.md` (vòng 101).
- `00-hien-trang.md`: bổ sung IOMMU/W^X-DMA, GRANT-TOCTOU, siết
  resume/restart/destroy.
- `08-giai-thich-cho-sinh-vien-nam-nhat.md`: từ điển thuật ngữ OS.
- `agent2-report-26/28/30`: soak flake + phân tích.

## 4. Còn lại trong goal cuối tuần

- Boot hang cuối soak-30 (treo ở Isolation khi quá tải): cần watchdog marker
  từng phase nếu tái hiện có settle.
- GPU aarch64 `INIT FAIL` khi soak: retry 4 lần chưa đủ trong đợt tải nặng.
