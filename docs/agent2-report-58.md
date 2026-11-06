# ViFastOs — Báo cáo vòng 58 (xác minh TX bằng packet dump)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `3e5d666`
**Mốc:** Chứng minh gói ARP của guest đã thực sự lên dây QEMU.

## 1. Thay đổi

- `scripts/test-qemu.py`: thêm opt-in `VIFAST_NET_DUMP=1` gắn
  `-object filter-dump` cho cả riscv64/aarch64 (mặc định tắt, gate không
  đổi; QEMU mới đã bỏ `dumpfile=` trong `-netdev`).
- Chạy dump 1 lần riscv64: pcap 102B = 1 gói 62B, linktype Ethernet.

## 2. Kết quả parse (bỏ 2B pad đầu của filter-dump)

`ff:ff:ff:ff:ff:ff | 52:54:00:12:34:56 | 0806 | 0001 | 0800 | 06 04 |
0001 | sha 52:54:00:12:34:56 | spa 10.0.2.15 | tha 00×6 | tpa 10.0.2.2`
+ 18B pad = 60B — ARP request who-has gateway đúng chuẩn, đã ra khỏi guest.

Kết luận pha 3: TX hoạt động hoàn toàn (guest → QEMU). Thiếu reply là việc
của backend/timing, không phải TX. Chốt vòng ở bằng chứng dây.

## 3. Số liệu cổng

- Host 203, build 0 warning (+check static PASS), `fmt` PASS, docs PASS.
- QEMU 3/3×3 (dump off mặc định, không sinh pcap), persist 7×3, harness 483.
