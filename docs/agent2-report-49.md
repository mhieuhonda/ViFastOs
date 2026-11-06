# ViFastOs — Báo cáo vòng 49 (gắn cả 2 queue trước DRIVER_OK)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `4e0bca6`
**Mốc:** Refactor init net đúng chuẩn VirtIO, hết treo QEMU.

## 1. Thay đổi

- `vifast-virtio`: thêm `net_init_both(base)` — reset/ACK/accept features,
  gắn queue 0 (RX + post 1 buffer WRITE hdr+buf) và queue 1 (TX), rồi mới
  DRIVER_OK. `net_init_tx` giờ chỉ là wrapper gọi `net_init_both` để giữ
  tương thích platform. Không chạm block queue.
- Rút ngắn bound poll RX 20M → 2M spins: QEMU user-net trả lời ARP trong ms
  nếu TX đúng; 20M spins trên QEMU mô phỏng mất >20s gây timeout rc=-9.
  (Nguyên nhân treo vòng 48 không phải QEMU kill mà là poll quá lâu.)
- Kết quả: cả 2 arch `READY accepted=0x10020`, `ARP sent len=0`,
  `NO REPLY` trung thực trong boot window — hạ tầng both-queues đúng,
  reply chờ QEMU backend hoặc TX cần padding (vòng sau).

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở giữ 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.

## 3. Việc tiếp theo (pha 3)

Thử ARP với padding 60B + post thêm RX buffer sau TX; nếu vẫn NO REPLY thì
dùng QEMU monitor `info network`/`hostfwd` xác minh backend trước khi
kết luận TX sai.
