# Hợp đồng cửa sổ copy user (copy-window contract)

Trạng thái: **implemented** — hình thành ở vòng 145–147, chốt thành hợp đồng r234.
Nguồn sự thật duy nhất: `crates/vifast-abi/src/lib.rs` (`COPY_WINDOWS`,
`copy_window_ok`, `USER_COPY_MAX`).

## 1. Vấn đề

Mọi syscall copy dữ liệu giữa kernel và user (`Read`, `Write`, `DebugWrite`,
`FrameWrite`, IPC buffer, `exec` đọc ELF/argv) đều phải kiểm "con trỏ người
dùng có hợp lệ không" **trước khi** dereference. Có bốn cách kiểm:

1. **Walk bảng trang** (`validate_user_range`) — chính xác nhưng chậm và phức
   tạp cho mọi syscall.
2. **So cận** theo `user_layout` (`CODE_VA..`, `STACK_BASE..`) — nhanh nhưng dễ
   bỏ sót một vùng mới.
3. **Copy qua cửa sổ cố định** (`read_user`/`write_user` trong platform) — nhanh
   nhất, nhưng chỉ đúng nếu cửa sổ được khai báo đủ.
4. **Không kiểm** — đúng nguyên nhân lỗi vòng 80 (kernel deref VA rác rồi chết).

Dự án dùng (3) cho đường copy nhanh, và (1) cho đường cần chính xác (map/frame).

## 2. Hợp đồng

```rust
pub const USER_COPY_MAX: usize = 256;      // trần một lần copy
pub const COPY_WINDOWS: &[(usize, usize)] = &[
    (CODE_VA, 0x2_0000),                   // text + rodata user
    (STACK_BASE, STACK_LEN),               // stack thường
    (EXEC_STACK_BASE, EXEC_STACK_TOP - EXEC_STACK_BASE),  // stack task exec
    (DATA_VA, 4096),                       // data
    (ROOT_MODULE_SERVER_VA, ROOT_MODULE_END - ROOT_MODULE_SERVER_VA),
    (IPC_BUF_SERVER_VA, 4096),
    (IPC_BUF_CLIENT_VA, 4096),
    (ROOT_ELF_BASE, ROOT_ELF_STACK_TOP - ROOT_ELF_BASE),
];
```

Bất biến:

- **C1.** `copy_window_ok(ptr, len)` đúng khi và chỉ khi `[ptr, ptr+len)` nằm
  **trọn** trong **một** cửa sổ; `len == 0`, tràn số, hoặc vắt qua hai cửa sổ
  đều sai.
- **C2.** Ba port (`riscv64`, `aarch64`, `x86_64`) kiểm **cùng** danh sách này.
  Không port nào có danh sách riêng.
- **C3.** Kernel không phát `len` gốc của user: mọi `UserAction` copy phát
  `len = min(a3, USER_COPY_MAX)` **đã validate** (bài học K72).
- **C4.** Thêm/đổi một vùng VA của user **bắt buộc** cập nhật `COPY_WINDOWS`;
  test `copy_windows_cover_user_regions` sẽ fail nếu thiếu.

## 3. Vì sao cần hợp đồng (bài học vòng 112–145)

Lỗi "exec không đọc được trang stack" mất **40 vòng** điều tra: PTE hợp lệ,
frame/PTE khớp, task chạy đúng VSpace, `read_user` vẫn `InvalidArg`. Nguyên nhân
cuối cùng (vòng 145): `EXEC_STACK_*` **không có** trong danh sách cửa sổ của
platform, nên mọi lần đọc stack của task exec đều bị từ chối — dù phần cứng
hoàn toàn cho phép.

Bài học: **kiểm tra "cửa sổ" là một bất biến của layout, không phải chi tiết
từng port.** Khi nó bị phân tán ở 3 nơi, một vùng mới sẽ lọt.

## 4. Test bảo vệ

| Test | Nội dung |
|---|---|
| `copy_windows_cover_user_regions` | mỗi vùng trong `user_layout` phải nằm trong ít nhất một cửa sổ |
| `copy_window_rejects_partial_and_zero` | `len == 0`, vắt biên, tràn số đều bị từ chối |
| `copy_window_matches_all_ports` | 3 platform dùng cùng danh sách (kiểm bằng hằng số, không bằng chạy máy) |
| `frame_write_clamps_len` (K72) | `a3 > USER_COPY_MAX` phát `len` đã kẹp, và `validate_user_range` chạy trên `len` đó |

## 5. Điều còn mở

- `COPY_WINDOWS` là **danh sách tĩnh**; nếu sau này user có vùng do
  `mmap`-tương đương tạo ra (G2), sẽ phải chuyển sang **đăng ký động** theo
  VSpace. Khi đó `copy_window_ok` phải tra bảng của VSpace hiện tại, không dùng
  hằng số toàn cục.
- Chưa có kiểm chứng "cửa sổ nằm trong vùng đã map" ở mức PTE; hiện dựa vào
  việc layout được dựng đúng.
