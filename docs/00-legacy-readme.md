# ViFastOs

Microkernel kiểu seL4 viết bằng Rust, hướng tới hai profile dùng chung một kernel:
**Desktop** (động: tạo tiến trình, cắm thiết bị lúc chạy) và **Space** (tĩnh: khoá cấu hình sau boot, phục vụ máy tính bay).

Repo gồm ba phần:

| Thư mục | Nội dung | Trạng thái |
|---|---|---|
| `python-model/` | Đặc tả chạy được bằng Python (27 test) | Đã chạy, 27/27 test qua |
| `rust/` | Bản port Rust: `vifast-abi`, `vifast-kernel`, `vifast-sim` (27 test) | **Chưa biên dịch thử** (môi trường viết không có Rust toolchain) |
| `docs/` | Tài liệu thiết kế, phân tích thành phần, điểm chú ý, lộ trình | Đầy đủ |

## Bắt đầu nhanh

```bash
# Mô hình Python
cd python-model
python run_demo.py space
python tests/test_kernel.py -v

# Bản Rust (cần Rust 1.85+)
cd rust
cargo build
cargo test -p vifast-sim
cargo run -p vifast-sim -- space
cargo run -p vifast-sim -- desktop
```

Nếu `cargo build` báo lỗi, xem `docs/05-port-rust-va-kiem-thu.md`, mục "Khi biên dịch lần đầu".

## Đọc tài liệu theo thứ tự

1. `docs/01-tong-quan.md`: mục tiêu, phạm vi, các quyết định lớn
2. `docs/02-kien-truc.md`: kiến trúc tổng thể, crate, nguyên tắc thiết kế
3. `docs/03-thanh-phan/`: phân tích kỹ thuật từng thành phần (9 tài liệu)
4. `docs/04-diem-chu-y.md`: toàn bộ điểm cần chú ý (bảo mật, đúng đắn, port, giới hạn)
5. `docs/05-port-rust-va-kiem-thu.md`: cách bản Rust được tổ chức, cách build, test, gỡ lỗi
6. `docs/06-lo-trinh.md`: lộ trình từ simulator tới QEMU, phần cứng thật, desktop, space
7. `docs/07-thuat-ngu.md`: thuật ngữ
