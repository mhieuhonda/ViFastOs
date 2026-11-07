# ViFastOs — Báo cáo vòng 80 (sửa P0 từ audit + TCP thường trú)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `ef75a4f`
**Bối cảnh:** vòng này mở đầu bằng audit toàn cảnh của subagent DeepSeek V4.1
Flash (`docs/audits/2026-11-07-round79.md`). Ba phát hiện P0/P1 được sửa ngay,
cộng với việc đưa TCP thành server thường trú.

## 1. P0-1 — `FrameWrite` không kiểm tra con trỏ

- Trước: arm `SyscallId::FrameWrite` trả `UserAction::FrameWrite` thẳng, không
  gọi `validate_user_range`. `frame_write(slot, 0, <VA trong cửa sổ nhưng chưa
  map>, 8)` làm runtime copy từ địa chỉ chưa map → kernel fault ở EL1/C-ring0.
  Biến thể `ptr = 0xFFFF_FFFF_FFFF_FFF0` còn làm phép cộng tràn và lọt kiểm
  tra cửa sổ.
- Sau: arm kiểm tra `validate_user_range(tcb, ptr, min(len, USER_COPY_MAX))`;
  `USER_COPY_MAX` chuyển vào `vifast-abi` để kernel và runtime dùng chung.
- Ba helper `page_in_range`/`user_range` của riscv64/aarch64/x86_64 đổi sang
  `checked_add`, trả false khi tràn.
- Test host mới trong `user_pointer_validation_rejects_kernel_and_unmapped_ranges`:
  con trỏ rác trả `InvalidArg`, con trỏ hợp lệ vẫn tạo `UserAction::FrameWrite`.

## 2. P0-2 — underflow `ip_total - ihl` trong TCP RX

- Trước: frame IPv4 với IHL=60, total length=20 làm `ip_total - ihl` underflow;
  `data_len` khổng lồ khiến driver copy byte cũ trong buffer RX và echo ra
  mạng (rò rỉ giữa các gói), thậm chí panic với buffer lớn.
- Sau: yêu cầu `ip_total >= ihl + 20` và kẹp `copy` theo `buf.len() - start`.

## 3. P1-2 — RX descriptor treo trùng

- Trước: `net_rx_post` luôn đẩy descriptor chain 0 (một buffer duy nhất) và
  `net_rx_refill` đẩy lặp 8 lần → 8 chain cùng trỏ một buffer; thiết bị ghi
  đè và driver đọc sai.
- Sau: `RX_OUTSTANDING` bảo đảm chỉ một chain đang treo; `net_rx_post` là
  idempotent, `net_rx_refill` chỉ đăng lại khi chưa có chain nào.

## 4. TCP thường trú (service window)

- `tcp_echo_demo` trong `init_block` được thay bằng `tcp_init` (chỉ lấy ISN từ
  rng, nạp buffer, in `VIRTIO TCP LISTEN OK`) và `tcp_pump` chạy trong timer
  IRQ hart 0 với bound nhỏ (2k poll, 250k TX) — mỗi tick tối đa 2 segment.
- Bug phụ tìm thấy: trên aarch64 `tcp_pump()` **chưa từng được gọi** (lần chèn
  trước sai thụt lề) — đã nối đúng.
- Thêm hook `Platform::service_window()` gọi trước `exit(0)`: giữ guest sống
  thêm ~2.5s (250 tick × 10ms) với WFI, timer IRQ vẫn bơm input/TCP. Nhờ đó
  test TCP/bàn phím có thời gian chạy mà không cần cửa sổ chặn boot.

## 5. Kết quả đo

- TCP echo: riscv64 **3/3**, aarch64 **3/3** (`--tcp-check`).
- `--input-check`: PASS cả hai arch.
- `gate.sh` mặc định: all steps passed, **3m22s** (nhanh hơn trước vì bỏ cửa sổ
  TCP trong boot).
- `gate.sh --with-tcp --with-input`: all steps passed.
- `cargo test -p vifast-kernel`: 134 passed, 0 failed.

## 6. Việc chưa làm (từ audit, chuyển vòng sau)

- P1-1: reclaim Untyped khi `alloc_owned` thất bại sau `untyped_take`.
- P1-3: thay `KERNEL_STATIC_BUDGET` bằng kiểm tra `nm` khoảng cách `.bss`→`BKL`.
- P1-4: `check-docs` so hash nội dung thay vì mtime.
- P1-5: marker FORBIDDEN `STALL` trùng `INSTALL FAILED`.
- P2: chạy fuzzer `#[ignore]` trong gate, bỏ `unwrap_or(0)` ở 53 chỗ.
