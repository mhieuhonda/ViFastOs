# ViFastOs — Báo cáo vòng 47 (gửi ARP đầu tiên, pha 3 bước 7)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `f650849`
**Mốc:** Gói TX đầu tiên rời khỏi guest qua VirtIO-net.

## 1. Thay đổi

- `vifast-virtio`: thêm `net_send_arp()` — build Ethernet broadcast +
  ARP request (who-has 10.0.2.2 gateway QEMU user-net, tell 10.0.2.15),
  2 descriptors (hdr 12B zeroed + frame 42B), kick queue 1, poll used ring
  bounded 20M spins. Trả về device-reported len. Không chạm block queue.
- RISC-V/AArch64: gọi sau `net_init_tx` thành công, log
  `VIRTIO NET ARP sent len={}` hoặc `VIRTIO NET ARP FAIL`.
- Kết quả: cả hai `sent len=0` — used ring advance (QEMU consumed TX,
  len=0 là bình thường cho transmit vì device không ghi payload trả về).

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở giữ 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.

## 3. Việc tiếp theo (pha 3)

Gắn RX queue 0 với buffer nhận, poll ARP reply từ gateway để hoàn tất
vòng TX→RX đầu tiên; sau đó mới tới ICMP/UDP.
