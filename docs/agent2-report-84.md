# ViFastOs — Báo cáo vòng 84 (P2: dead code, assert biên dịch, slot validation)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `cd0ec59`
**Bối cảnh:** các mục P2 còn lại từ audit #1/#2, làm trong lúc audit #3 chạy.

## 1. Xoá `net_recv_udp` (dead code, không validate)

- Hàm không có caller nào (DNS dùng `net_recv_dns_a`), lại đọc thẳng
  `buf[34..40]` mà không kiểm IHL/độ dài gói — đúng loại parser nguy hiểm nếu
  sau này có người dùng.
- Đã xoá hẳn thay vì để lại bẫy.

## 2. `KERNEL_STATIC_BUDGET` thành ràng buộc biên dịch

- Trước: chỉ kiểm ở runtime (`KERNEL STATIC OVERFLOW`), tức là phải boot mới
  biết; trên AArch64 khe tĩnh chỉ còn 8 byte.
- Thêm `const _: () = assert!(size_of::<Kernel>() <= KERNEL_STATIC_BUDGET, ...)`
  ngay sau định nghĩa `Kernel` — mọi thay đổi làm `Kernel` phình quá ngân sách
  giờ fail ngay khi `cargo build`, kèm thông điệp nhắc kiểm tra `gap=` của
  `build.sh`.

## 3. `slot_arg(..).unwrap_or(0)` → `slot_or_invalid!` (53 chỗ)

- Trước: tham số slot > 255 bị biến thành slot 0, nên một lời gọi sai lặng lẽ
  thao tác trên self-TCB của caller thay vì trả lỗi.
- Thêm macro `slot_or_invalid!` đặt ở đầu crate (macro_rules cần khai báo
  trước khi dùng) trả `UserAction::Return { error: InvalidArg }`.
- Thay toàn bộ 53 chỗ; thêm assert trong test con trỏ: `CapDelete` với slot 300
  trả `InvalidArg`.
- `CAP_NONE` (= u64::MAX) không đi qua đường này (nó là tham số cap của
  `Send/Call`, xử lý riêng trong `message_from_cap`), nên không hồi quy.

## 4. Kiểm chứng

- `cargo test -p vifast-kernel`: **136 passed**.
- QEMU: riscv64 3/3, aarch64 3/3, x86_64 6/6 (hai lượt) — lượt 2/3 duy nhất
  xảy ra khi audit #3 đang chạy lệnh nặng song song.
- `gate.sh --with-tcp --with-input`: xanh (trừ lượt chịu tải nói trên).

## 5. Ghi chú

- Rà `scripts/soak.py` và `scripts/gate.sh` cho câu hỏi "false green": soak
  coi `code != 0` **hoặc** `passed != total` là hỏng; nếu thiếu summary thì
  mặc định `(0,1)` → hỏng. `gate.sh` gom lỗi từng bước và trả mã 1. Không
  thấy đường nào báo PASS khi bước bị bỏ qua.
