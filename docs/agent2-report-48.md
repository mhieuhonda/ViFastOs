# ViFastOs — Báo cáo vòng 48 (RX API giữ lại, runtime hoãn sau DRIVER_OK)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `e8c903e`
**Mốc:** Thử khép vòng TX→RX; phát hiện giới hạn QEMU, giữ gate xanh.

## 1. Đã thử và kết quả

- Thêm `NET_RX_QUEUE` static (~4 KiB, `.bss` ngoài Kernel), `net_init_rx()`
  (gắn queue 0 + post 1 buffer WRITE) và `net_recv_arp()` (poll used, parse
  Ethernet+ARP reply → sender MAC).
- Đảo thứ tự post RX trước TX: vẫn `NO REPLY` (QEMU user-net không giao reply
  trong poll bounded, hoặc ARP request chưa tới backend).
- Gắn RX sau `DRIVER_OK` của TX: QEMU kill guest `rc=-9` timeout 20s sau
  SELFTEST (cả khi revert padding). Kết luận: không cấu hình queue sau khi
  device đã DRIVER_OK — phải cấu hình cả 2 queue trước DRIVER_OK.
- Revert runtime về TX-only (vòng 47 đã ổn định): `NO REPLY` biến mất khỏi
  binary, gate 3/3×3 xanh lại. Giữ `net_init_rx`/`net_recv_arp`/`NET_RX_QUEUE`
  làm API cho vòng sau (cấu hình RX+TX cùng lúc trước DRIVER_OK).

## 2. Bài học

VirtIO: mọi queue phải gắn xong trước khi set DRIVER_OK. Vòng sau refactor
`net_init_tx` thành `net_init_both` (gắn queue 0 + 1 rồi mới DRIVER_OK),
sau đó TX ARP → poll RX.

## 3. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở giữ 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.
