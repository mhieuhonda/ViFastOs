# ViFastOs — Báo cáo vòng 83 (audit #2: P0 filesystem + hoàn tất bản vá RX/TCP)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `2d0ddcf`
**Bối cảnh:** audit #2 (`docs/audits/2026-11-07-round82.md`) tìm thêm 2 P0
trong filesystem và chỉ ra 3 bản vá vòng 80 chỉ đúng một phần.

## 1. P0-1 — dir entry length từ đĩa

- Một byte `len` hỏng (ví dụ 200) làm `name[..len].copy_from_slice(...)` panic
  trong VFS server; panic handler loop `yield` nên **mọi IPC sau đó treo**.
- Sửa ở cả 3 chỗ (`lookup`, `remove_dir_entry`, `read_dir_entry`): bỏ qua entry
  nếu `len == 0 || len > MAX_NAME || off+3+len > BLOCK_SIZE`.
- Test `corrupt_dir_entry_length_does_not_panic` (sửa byte trên đĩa rồi mount
  lại, gọi lookup/readdir).

## 2. P0-2 — dir size vượt slot + bitmap không kiểm biên

- `Inode::blocks` là `[u32; 12]`; `blocks_used()` tính từ `size` trên đĩa nên
  dir size 7000 làm index quá biên ở mọi vòng lặp thư mục.
- `mark_free`/`mark_used` ghi bitmap theo block number đọc từ đĩa → ghi ngoài
  mảng.
- Sửa: `blocks_used()` kẹp 12 **cho INODE_DIR** (file lớn vẫn dùng indirect
  qua `file_block()`); `mark_*` kiểm `block < block_count()` và biên bitmap.
- Test `corrupt_dir_size_beyond_block_slots_does_not_panic`.
- `cargo test -p vifast-fs`: **24 passed**.

## 3. Hoàn tất các bản vá vòng 80

- **RX một-chain**: `net_init_both` tự treo chain 0 nhưng không đặt
  `RX_OUTSTANDING=1`, nên lần `net_rx_rearm()` đầu tiên treo trùng lần nữa.
  Nay set `RX_SEEN=0; RX_OUTSTANDING=1` ngay sau hand-post.
- **TCP RX**: thêm `net_rx_last_len()` đọc `used.ring[].len` và kẹp `ip_total`
  theo độ dài gói thật — không còn tin trường độ dài trong frame.
- **service_window**: audit chứng minh hook chạy với IRQ bị mask (sẽ treo nếu
  được gọi) và `UserAction::Exit` chỉ do panic handler gọi nên "cửa sổ 2.5s"
  không tồn tại → **gỡ bỏ** hook, counter và vòng WFI.
- **Client**: kẹp `len` của readdir theo `[u8; 28]` trước khi copy.
- **Fingerprint docs**: phủ thêm `harness_spec.py`, `test-qemu.py`, `gate.sh`,
  `soak.py` để đổi tiêu chí PASS cũng làm status cũ.

## 4. Kiểm chứng

- `cargo test -p vifast-fs`: 24 passed (2 test corruption mới).
- `cargo test -p vifast-kernel`: 135 passed.
- `gate.sh --with-tcp --with-input`: all steps passed (gồm fuzzer release).
- TCP echo và input check vẫn PASS sau khi gỡ service_window.
