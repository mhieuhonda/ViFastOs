# ViFastOs — Báo cáo vòng 63 (net TX+RX end-to-end: ping gateway)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `2b48f3e`
**Mốc:** Lần đầu guest gửi ICMP echo và **nhận được reply** từ 10.0.2.2 —
cả riscv64 và aarch64. Đóng mục "net + stack tối thiểu".

## 1. Ba lỗi gốc tìm được (đều là bug thật, đã sửa)

1. **Header virtio-net phải 10 byte, không phải 12.** Ta dùng 12 (kèm
   `num_buffers`) nhưng KHÔNG thoả thuận `VIRTIO_NET_F_MRG_RXBUF`; QEMU đọc
   10 byte rồi coi 2 byte còn lại là đầu frame → frame lệch 2 byte →
   ethertype sai → slirp bỏ qua mọi gói. Đây là lý do "NO REPLY" suốt các
   vòng 47–62. Sửa `NET_HDR_SIZE = 10`.
2. **QueueReady (0x44) phải ghi 1, không phải index queue.** Ta ghi `index`
   nên RX (index 0) không bao giờ ready → device không ghi gói vào RX.
3. **RX cần kick + re-post.** VirtIO yêu cầu notify queue 0 sau khi thêm
   buffer; và sau mỗi gói tiêu thụ phải cấp lại buffer. Thêm `net_rx_post()`,
   `net_rx_poll()` (đếm `RX_SEEN`), `net_rx_rearm()`, và vòng lặp bỏ qua gói
   không khớp (slirp có thể gửi probe ARP trước).

## 2. Thêm mới

- `net_send_arp_to(tpa)` (ARP cho gateway + DNS), `net_ping(ip)` (ICMP echo
  với checksum RFC 1071), `net_recv_icmp()` (lọc reply type 0), sửa đọc
  used-ring đúng `ring[(idx-1) % size]`.
- Log: `VIRTIO NET ARP GW/DNS sent`, `PING sent`, **`PING REPLY from
  10.0.2.2`**.
- Harness: `VIRTIO NET PING REPLY` thành exact-one cho riscv64 + aarch64
  (x86_64 dùng IDE nên loại trừ); fixture cập nhật.

## 3. Bằng chứng

- riscv64: `VIRTIO NET PING REPLY from 10.0.2.2`.
- aarch64: `VIRTIO NET PING REPLY from 10.0.2.2`.
- pcap (VIFAST_NET_DUMP=1): 6 gói — ARP request/reply (.2, .3), ICMP
  request/reply — khớp hoàn toàn với log guest.

## 4. Số liệu cổng

- Host 203, build 0 warning (+check static PASS), `fmt` PASS, docs PASS.
- QEMU 3/3×3, persist 7×3, harness 489.
