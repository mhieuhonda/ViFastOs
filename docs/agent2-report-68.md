# ViFastOs — Báo cáo vòng 68 (DNS A record: guest resolve tên miền thật)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `726371d`
**Mốc:** Stack mạng không chỉ gửi/nhận UDP mà còn **phân tích DNS answer** và
lấy được địa chỉ IPv4 thật.

## 1. Thay đổi

- `vifast-virtio`: thêm `parse_dns_a(payload)` (bỏ header + question, duyệt
  answer section, hỗ trợ tên nén 0xC0, lấy bản ghi A đầu tiên) và
  `net_recv_dns_a(src_ip)` (poll RX + parse, không tiêu thụ gói trước khi đọc).
- Platform riscv64/aarch64: thay `net_recv_udp` bằng `net_recv_dns_a`, log
  `VIRTIO NET DNS A {}.{}.{}.{}`.
- 2 unit test DNS trên host (response hợp lệ có A, response không có A).

## 2. Kết quả

- riscv64: `VIRTIO NET DNS A 104.20.23.154`
- aarch64: `VIRTIO NET DNS A 172.66.147.243`
- (IP khác nhau giữa hai lần chạy do DNS round-robin của example.com — cả hai
  đều là địa chỉ thật của Cloudflare cho example.com.)

Marker `VIRTIO NET DNS A` giữ **best-effort** (phụ thuộc DNS của host), không
đưa vào harness bắt buộc; `VIRTIO NET DNS sent` vẫn là marker cứng.

## 3. Cổng

- Host test 207 → 209 (thêm 2 test DNS), build 0 warning.
- QEMU 3/3×3, persist 7×3, harness 494, `fmt` PASS.
