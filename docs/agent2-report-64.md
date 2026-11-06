# ViFastOs — Báo cáo vòng 64 (UDP + DNS hai chiều)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `36dbb99`
**Mốc:** Stack tối thiểu có thêm UDP: guest gửi DNS query tới slirp
(10.0.2.3:53) và **nhận reply** — cả riscv64 và aarch64.

## 1. Thêm mới

- `net_tx_frame(&[u8])` — tách phần kick/poll TX dùng chung (frame dài hơn
  60B cho UDP).
- `net_send_udp(dst_ip, dst_port, payload)` — Ethernet + IPv4 + UDP với
  checksum IP; checksum UDP = 0 (hợp lệ trong IPv4).
- `net_recv_udp(src_ip, src_port)` — lọc đúng nguồn/cổng, bỏ qua gói khác.
- `net_rx_poll_bounded(bound)` — UDP dùng bound 8M spins vì DNS phải qua
  slirp forward ra host (chậm hơn ICMP nội bộ).
- Platform riscv64 + aarch64: DNS query A cho `example.com`, log
  `VIRTIO NET DNS sent` và `VIRTIO NET DNS REPLY len=`.

## 2. Bug tìm được

- **Không re-post buffer RX sau khi nhận thành công**: `net_recv_icmp` trả
  `Some` mà không cấp lại buffer, nên gói kế tiếp (DNS reply) không có chỗ
  và bị device bỏ. Sửa: re-post trước mọi lần trả về trong cả ba hàm nhận.
- Đây là lỗi tiềm ẩn từ vòng 63, chỉ lộ ra khi có hai gói liên tiếp.

## 3. Bằng chứng

- Guest: `VIRTIO NET DNS sent len=0` → `VIRTIO NET DNS REPLY len=69` trên
  cả hai arch (có trong serial của lần chạy gate).
- pcap: 8 gói — ARP request/reply ×2, ICMP request/reply, UDP query
  49152→53 len=37, **UDP reply 53→49152 len=69**.
- Harness khóa `VIRTIO NET DNS sent` (TX tất định); reply giữ best-effort
  để gate không phụ thuộc DNS của host.

## 4. Số liệu cổng

- Host 203, build 0 warning, `fmt` PASS, docs PASS.
- QEMU 3/3×3, persist 7×3, harness 489.
