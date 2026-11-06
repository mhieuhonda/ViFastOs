# ViFastOs — Báo cáo vòng 65 (fw_cfg: kết quả âm có căn cứ + test checksum)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `442d4d9`
**Mốc:** Đóng điều tra fw_cfg/ramfb bằng bằng chứng; thêm test cho checksum.

## 1. fw_cfg — ba biến thể, cùng một kết cục

| Biến thể | Kết quả |
|---|---|
| DMA (control/len/address, ghi địa chỉ BE vào +0x08) | treo, không có log sau `FWCFG base` |
| Legacy selector `u16` tại +0x04 (đúng offset) | treo |
| Legacy selector ghi từng byte (tránh endianness BE) | treo |

Phát hiện quan trọng: **vòng 53 ghi selector vào +0x08 — đó là thanh ghi DMA
address, không phải selector**. Selector đúng là +0x04. Dù vậy, mọi thao tác
ghi vào vùng fw_cfg `0x10100000` đều làm guest treo, nên rất có thể vùng này
chỉ cho đọc (write gây fault → trap loop) trong cấu hình QEMU hiện tại.
Đọc +0x10 (version) trả về đúng chữ ký "QEMU" (0x554d4551) — dấu hiệu vùng
MMIO chỉ phơi bày data port.

**Kết luận:** không thể cấu hình ramfb qua fw_cfg ở môi trường này. Hướng
display tiếp theo là driver virtio-gpu (MMIO, không cần fw_cfg) — để vòng sau.
Đã revert toàn bộ thực nghiệm, gate giữ nguyên.

## 2. Test mới cho checksum (RFC 1071)

`crates/vifast-virtio`: 3 unit test
- `checksum_matches_rfc1071_ipv4_header` — header mẫu RFC 1071 → 0xb861.
- `checksum_of_valid_header_is_zero` — header đã điền checksum → 0.
- `checksum_handles_odd_length` — độ dài lẻ đệm 0.

Đây là phần lõi của `net_ping` và `net_send_udp` nên test bảo vệ trực tiếp
đường TX.

## 3. Số liệu cổng

- Host test 204 → **207** (thêm 3, và `cargo test -p vifast-virtio` được thêm
  vào `scripts/build.sh` để gate thực sự chạy chúng), build 0 warning.
- QEMU 3/3×3, persist 7×3, harness 489, `fmt` PASS, docs PASS.
