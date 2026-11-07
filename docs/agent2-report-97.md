# ViFastOs — Báo cáo vòng 97 (NX toàn ảnh + 4 phát hiện soi chay)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `e4241fc`

## 1. NX: từ 20 vùng tĩnh sang **toàn ảnh trừ text**

- `kernel_text_ranges()` đọc PE section table của ảnh đã nạp, lọc
  `IMAGE_SCN_MEM_EXECUTE` (không lọc theo tên — tên bị cắt 8 ký tự và
  `efi_main` nằm ở section riêng `.text.entry`).
- `image_pages_nx()` đánh NX mọi trang của ảnh trừ các dải text, rồi
  **`reload_cr3()`** — audit #9 chỉ ra thiếu bước này thì NX chỉ nằm trên giấy
  cho tới lần switch VSpace sau.
- `kernel_wx_ok()` kiểm **hai chiều** từ chính page table: non-text còn exec =
  fail, text bị NX = fail, leaf thiếu/không supervisor = fail.
- `split_identity_2m` có bounds check `region < 512` (base ưu tiên
  0x1400_0000 ⇒ region 10240 sẽ ghi ngoài `PD_IDENTITY`).
- Marker mới: `IMAGE TEXT NX OK ranges=3 image=0xdc4b000..0xde1a000` (đo thật).

## 2. Bốn phát hiện "soi chay"

- **K1**: `wake_error` không còn enqueue task đang `Running` (chỉ `Blocked*`),
  tránh một task chạy trên hai hart. 2 test mới.
- **K2**: test chuỗi A→B→C chứng minh destroy C đánh thức B, destroy B đánh
  thức A (đều `ObjectDestroyed`).
- **K3**: `enqueue` chọn hart **ít việc nhất** thay vì luôn hart 0. Test mới.
- **N1**: `net_recv_dns_a` và `net_recv_icmp` kẹp theo `net_rx_last_len()`,
  `ip_total` và `udp_len` khai báo — trước đây gói khai length lớn có thể khiến
  parser đọc rác cũ trong buffer.

## 3. Kiểm chứng

- Kernel **145 test**, FS 25, harness 508.
- QEMU 3/3 cả ba kiến trúc (gate có 1 lần qemu flake, chạy lại 3/3).
- `gate.sh --with-tcp --with-input`: all steps passed.

## 4. Còn mở

- Chuyển `image_pages_nx()` xuống **sau** `setup_gdt_tss_idt()` để fault (nếu
  có) in được thay vì triple fault.
- Nếu ảnh được nạp dưới 0x9F000 thì phải skip trang trampoline AP khỏi NX.
