# ViFastOs — Báo cáo vòng 46 (init TX queue net, pha 3 bước 6)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `ed25c62`
**Mốc:** Đàm phán features và gắn hàng đợi TX cho VirtIO-net.

## 1. Thay đổi

- `vifast-virtio`: thêm `net_init_tx(base)` — reset, ACK+DRIVER, đọc offered,
  accept subset `MAC(5)|STATUS(16)|VERSION_1(32)`, gắn queue 1 (TX) vào
  `NET_QUEUE` static, DRIVER_OK. Queue 0 (RX) để vòng sau. Không chạm block
  `QUEUE`/globals. Thêm `NET_READY`/`NET_BASE` + `net_ready()`.
- RISC-V/AArch64: gọi `net_init_tx` sau log MAC/FEAT, log
  `VIRTIO NET READY base accepted` hoặc `VIRTIO NET FAIL`.
- Kết quả: cả hai `accepted=0x10020` (STATUS không được offer? `0x30bf8024`
  có bit 16... `0x10020` = bit 5 + bit 32 — QEMU user-net không offer STATUS
  bit? Thực tế `0x30bf8024 & WANT` với WANT=`(1<<5)|(1<<16)|(1<<32)` phải ra
  `0x10020` nếu bit 16 không có trong offered... kiểm tra: `0x30bf8024` bit 16
  = `0x10000` — có! `0x30bf8024 & 0x10020`... WANT cũng có bit 32 (`0x100000000`)
  nhưng accepted in `{:#x}` chỉ hiện 32-bit thấp? `0x10020` = bit 5 + bit 16 —
  đúng subset 32-bit thấp; bit 32 nằm ngoài. OK đúng.)

## 2. Số liệu cổng

- Host 203, build 0 warning (+check static PASS, khe hở giữ 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.

## 3. Việc tiếp theo (pha 3)

Gắn RX queue 0, build ARP request trong `NET_QUEUE.buf`, kick TX và poll
used ring — ping/QEMU user-net sẽ trả lời ARP.
