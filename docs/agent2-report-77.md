# ViFastOs — Báo cáo vòng 77 (TCP echo server tối giản)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `547be3c`
**Mốc:** Stack mạng chạm TCP: guest bắt tay và **echo dữ liệu thật** với một
client trên host qua QEMU user-net `hostfwd`.

## 1. Thay đổi

- `vifast-virtio`: cờ TCP (`FIN/SYN/RST/PSH/ACK`), `TcpSegment`,
  `net_send_tcp(dst_ip, dst_port, src_port, seq, ack, flags, payload)` (IPv4 +
  TCP với checksum pseudo-header), `net_recv_tcp[_bounded]` (parse ihl/doff,
  copy payload ra ngoài, bỏ qua segment không đúng cổng).
- Platform riscv64/aarch64: `tcp_echo_demo()` chạy trong `init_block` sau demo
  net — nghe cổng 8080, xử lý SYN → SYN-ACK, ACK+payload → ACK+PSH echo,
  FIN → ACK+FIN; cửa sổ chờ ~100 poll × 100k spins để không làm chậm boot
  đáng kể khi không có client. Log `VIRTIO TCP LISTEN OK port=8080`,
  `VIRTIO TCP ACCEPT`, `VIRTIO TCP CLOSE`.
- `test-qemu.py`: thêm `--tcp-check`, boot với
  `-netdev user,hostfwd=tcp::18080-:8080` (aarch64 dùng 18081), client host
  gửi `ping-from-host` và yêu cầu echo đúng.
- Harness: `VIRTIO TCP LISTEN OK` exact-one trên riscv64/aarch64
  (504 cases).

## 2. Bằng chứng

- riscv64: `--- riscv64 tcp PASS (echo b'ping-from-host') ---`.
- aarch64: `--- aarch64 tcp PASS (echo b'ping-from-host') ---`.
- Serial guest: `VIRTIO TCP LISTEN OK port=8080` + `VIRTIO TCP ACCEPT`.

## 3. Giới hạn đã biết (ghi rõ để vòng sau)

- Một kết nối tại một thời điểm, không retransmit, không window scaling,
  không xử lý out-of-order; ISN cố định (sẽ lấy từ virtio-rng sau).
- Server chỉ tồn tại trong cửa sổ chờ của `init_block`, chưa phải service
  thường trú (muốn vậy cần task riêng + syscall net hoặc driver trong kernel
  loop).

## 4. Cổng

- `scripts/gate.sh`: build, QEMU 3/3×3, persist 7×3, harness 504, fmt, docs —
  **all steps passed** (khoảng 5 phút do cửa sổ chờ TCP).
