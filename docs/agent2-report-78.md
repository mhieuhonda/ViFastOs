# ViFastOs — Báo cáo vòng 78 (sửa rò rỉ RX, TCP tin cậy, chốt chặn x86)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `00867fd`

## 1. Bug thật: rò rỉ descriptor RX

`net_rx_poll_bounded` gán `RX_SEEN = used.idx` (nhảy tới gói mới nhất) thay vì
tiến từng gói. Mỗi gói bị nhảy qua **không được đăng lại buffer**, nên sau vài
gói ring RX cạn dần và thiết bị "im" — biểu hiện trên aarch64: guest in
`VIRTIO TCP LISTEN OK` nhưng `used.idx` đứng yên (5 → 5) suốt cửa sổ, không
bao giờ `ACCEPT`.

Sửa: `RX_SEEN = RX_SEEN.wrapping_add(1)`; thêm `net_rx_refill(8)` nạp đầy
buffer trước phiên TCP; log `VIRTIO TCP RX`/`RXEND` (avail/used/seen) để chẩn
đoán về sau.

## 2. TCP tin cậy hơn

- ISN lấy từ **virtio-rng** (không còn hằng số dự phòng trừ khi rng lỗi).
- Xử lý `RST` (`VIRTIO TCP RESET`, đóng kết nối).
- Cửa sổ nghe: 400 poll × 25k spins, **thoát sớm sau FIN** — giữ tổng thời
  gian boot trong hạn QEMU 20s (250×80k từng làm riscv vượt hạn).
- `--tcp-check` chuyển sang **chardev socket**: Python giữ đầu server, QEMU
  kết nối (`-serial unix:path`), nên không mất byte nào và biết chính xác lúc
  `LISTEN OK` xuất hiện rồi mới nối TCP. Trước đó đọc file serial bị buffer
  khiến SYN tới trước cửa sổ và bị drop.

## 3. Kết quả đo

- aarch64 `--tcp-check`: **5/5 PASS** (trước: 0/5 với file serial, 2-4/5 với
  cách nối mù).
- riscv64 `--tcp-check`: **3/3 PASS**.
- `scripts/gate.sh --with-tcp`: **all steps passed**.

## 4. Chốt chặn x86_64 (ghi rõ để khỏi thử lại lần 4)

x86_64 map root ELF/stack bằng trang 2 MiB, nên mọi VA mới trong cùng block
(ví dụ IPC buffer 0x1F000, 0x2E0000, 0x2FF000) đều trả `MissingPageTable` —
cần tách PMD thành PageTable. Nguồn cấp thì đã cạn:

- `data` Untyped: 76/76 trang đã dùng; `meta`: 76/76;
- boot Untyped còn 3 trang nhưng `retype(PageTable)` vẫn thất bại;
- vì vậy VFS demand-load giữ nguyên cho riscv64/aarch64, x86 dùng ảnh tĩnh.

Hướng mở thật sự: giải phóng staging frame của module loader (trả về Untyped)
hoặc thêm hỗ trợ tách huge page trong kernel.

## 5. Cổng

- `gate.sh` (mặc định): build, QEMU 3/3×3, persist 7×3, harness 504, fmt, docs
  — all steps passed.
- `gate.sh --with-tcp`: thêm TCP echo qua hostfwd — all steps passed.
