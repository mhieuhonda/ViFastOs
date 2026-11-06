# ViFastOs — Báo cáo vòng 42 (probe net trước init, cả 2 arch thấy device=1)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `32f1106`
**Mốc:** Pha 3 bước 2 — xác minh net hiện diện qua probe an toàn.

## 1. Phát hiện then chốt

Serial test đúng nằm ở `$TMPDIR/vifast-*.serial`, không phải `/tmp/`. Đọc
nhầm file cũ là nguyên nhân vòng 39–41 tưởng probe không chạy — thực tế
probe trước init chạy tốt và đã log từ trước.

## 2. Thay đổi

- RISC-V: probe 3 slot (`0x10001000/7000/8000`) trước `init_range`; log
  `VIRTIO PRE base device`. Net ở slot 2: `device=1`.
- AArch64: quét 32 slot đầu, chỉ log slot có device; net ở `0xa003c00`
  (`device=1`), blk ở `0xa003e00` (`device=2`).
- QEMU: gắn `virtio-net-device,netdev=vn0` + `user,id=vn0` cho cả riscv64 và
  aarch64 (blk giữ nguyên slot, gate không ảnh hưởng).

## 3. Số liệu cổng

- Host 203, build 0 warning (+check static PASS), `fmt` PASS, docs PASS.
- QEMU 3/3×3, persist 7×3, harness 483. Lần 2/3 riscv flaky 1 lần, chạy
  riêng lại 3/3 PASS.

## 4. Việc tiếp theo (pha 3)

Đọc MAC qua config space (`net_mac` đã có), init TX/RX queue cho net,
gửi ARP request đầu tiên.
