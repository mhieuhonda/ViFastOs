# ViFastOs — Báo cáo vòng 50 (ARP 60B + chẩn đoán RX, pha 3 bước 10)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `4e6beac`
**Mốc:** Loại trừ frame runt và lỗi parse; xác định không có reply.

## 1. Đã thử và kết quả

- Padding ARP 42B → 60B (Ethernet minimum) + rút TX poll về 2M cho nhất
  quán. Vẫn `NO REPLY`, không PEEK (used.idx RX vẫn 0 — không có gói nào đến).
- Rà soát parse: `hdr` và `buf` là 2 descriptors riêng (device ghi header
  vào `hdr`, frame vào `buf[0..]`) — code parse từ `buf[0]` đã đúng; chỉ sửa
  comment giải thích (trước đây ghi sai là liền nhau).
- Thêm `net_recv_peek()` snapshot (used_len, ethertype, 6B đầu) khi có gói
  bất kể loại + log `VIRTIO NET PEEK` cả 2 arch — xác minh không có gì đến,
  không phải parse bỏ lỡ.
- Kết luận: TX đi (used advance, len=0) nhưng QEMU user-net không giao reply
  trong boot window. Vòng sau xác minh backend bằng monitor `info network`
  hoặc thử ping từ host vào guest.

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở giữ 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.

## 3. Việc tiếp theo (pha 3)

Xác minh backend user-net ngoài guest (monitor/host ping) trước khi đụng
thêm vào TX; sau đó mới tới ICMP/UDP.
