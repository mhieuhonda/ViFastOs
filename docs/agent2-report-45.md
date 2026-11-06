# ViFastOs — Báo cáo vòng 45 (NetQueue static riêng, pha 3 bước 5)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `16b3844`
**Mốc:** Chuẩn bị hàng đợi net mà không chạm ngân sách static.

## 1. Đo ngân sách trước khi code

- `Kernel` = 561.768 / budget 716.800 (~155 KiB dư trong `KERNEL_STATIC_BUDGET`).
- Nhưng khe hở AArch64 thực tế chỉ 8B (`end=1075053160`, `bkl=1075053168`).
- `nm` xác nhận block `QUEUE` (4 KiB) nằm ở `0x401ee000` vùng `.bss` sau
  BKL — còn nhiều chỗ. Kết luận: NetQueue mới phải nằm ngoài `Kernel`.

## 2. Thay đổi

- `vifast-virtio`: thêm `NET_BUF_SIZE=2048`, `NET_HDR_SIZE=12`,
  struct `NetQueue` (desc/avail/used + hdr 12B + buf 2048B, align 4096,
  ~4 KiB) + static `NET_QUEUE` + `net_queue_info() -> (addr, size)`.
- RISC-V/AArch64: log `VIRTIO NETQ addr=.. size=4096` đầu `init_block`
  (read-only, không chạm device).
- Verified: khe hở static vẫn đúng 8B sau build (không tăng `Kernel`);
  RISC-V log `NETQ addr=0x80366000 size=4096`.

## 3. Số liệu cổng

- Host 203, build 0 warning (+check static PASS), `fmt` PASS, docs PASS.
- QEMU 3/3×3, persist 7×3, harness 483.

## 4. Việc tiếp theo (pha 3)

Dùng `NET_QUEUE` để init 1 TX queue cho net (ACK features, set queue addr),
gửi ARP request đầu tiên; RX sau.
