# Hợp đồng ABI argv khi exec (exec argv ABI)

Trạng thái: **implemented** — D15, vòng 246–248 (T12 của G1).
Nguồn sự thật duy nhất: `crates/vifast-abi/src/lib.rs`
(`user_layout::EXEC_ARGS_*`, `SyscallId::TcbSetArgs`).

## 1. Vấn đề trước r246

Exec thời kỳ đầu không có argv. Muốn "truyền tham số" cho chương trình, root
**vá byte vào trang code** của ảnh: ghi cặp `(ptr, len)` vào `entry+0x20` và
chuỗi vào `entry+0x40`, còn chương trình thì `auipc`/`adr`/`lea` tới đúng offset
cứng đó rồi gọi `DebugWrite`.

Ba hệ quả, đủ để coi đây là lỗi thiết kế chứ không phải "MVP tạm":

1. **Không dùng được cho chương trình thật.** Ảnh Rust (vifast-bin) không có
   "slot 0x140" để tham chiếu; muốn truyền tham số phải build **một ảnh riêng
   cho mỗi lệnh** (`VIFAST_BIN_MODE`) — đúng thứ D14 muốn bỏ.
2. **Không có multicall.** Chọn hành vi theo `argv[0]` là bất khả thi khi
   không có argv.
3. **Vá vào trang code là ghi vào vùng EXEC** — chạy được chỉ vì root ghi
   *trước khi map*; mọi thay đổi thứ tự đó sẽ hỏng ngầm.

## 2. Hợp đồng

Khi kernel bắt đầu một TCB được tạo bằng `TcbConfigure` + `TcbSetArgs`:

| Điều kiện | Giá trị |
|---|---|
| `a0` (`x0`/`rdi`) | `argc`, `0 ≤ argc ≤ 8` |
| `a1` (`x1`/`rsi`) | con trỏ tới mảng `argv` (canh 8 B), `0` khi `argc == 0` |
| `sp` | `EXEC_ARGS_BASE` = `EXEC_STACK_TOP − 256` |
| `argv[i]` | con trỏ tới chuỗi NUL-terminated trong vùng chuỗi |

Khối tham số nằm trên **trang stack của exec**, 256 B ngay dưới đỉnh trang:

```text
EXEC_STACK_TOP − 256   argv[0] ─┐   8 con trỏ × 8 B = 64 B
                       argv[1]  │
                       …        │
EXEC_STACK_TOP − 192   argv[7] ─┘
EXEC_STACK_TOP − 192   vùng chuỗi NUL-terminated, 192 B
EXEC_STACK_TOP         (đỉnh trang, không dùng)
```

`sp` khởi tạo ở **đáy khối**, không phải đỉnh trang: khối nằm *trên* `sp` nên
stack mọc xuống không bao giờ ghi đè tham số. Xem §5 để biết vì sao đây là
ràng buộc cứng, không phải chi tiết tuỳ ý.

`argv[0]` do root đặt bằng **đường dẫn chương trình** (giống Unix), các tham số
sau tách theo dấu cách. Nhờ vậy `/bin/vf` chọn được mode từ tên hard link.

## 3. Đường đi của một lần exec

1. **Root** (`spawn_elf_from_fs`/`spawn_elf_copy`) dựng `argv` (`build_argv`).
2. `write_exec_args` ghi mảng con trỏ + vùng chuỗi vào **frame stack, TRƯỚC khi
   map** — sau khi map thì `FrameWrite` lên frame RW vẫn được, nhưng ghi trước
   là điều kiện rẻ nhất và không phụ thuộc W^X.
3. `tcb_configure(tcb, entry, EXEC_ARGS_BASE, vspace, prio)`.
4. `tcb_set_args(tcb, argc, argv)` — syscall 56, kernel kiểm:
   `argc ≤ 8`; `argc != 0 ⇒ argv != 0`; `argv` canh 8 B; TCB phải
   `Inactive`/`Exited` (không ghi đè context của task đang sống).
5. Kernel dựng lại context với `a0 = argc`, `a1 = argv` và trả về action
   `SetTcbArgs` để platform ghi đúng thanh ghi (`a0/a1`, `x0/x1`, `rdi/rsi`).
6. `TcbRestart` cũng mang `argc/argv` đã lưu ⇒ restart giữ nguyên tham số;
   `TcbConfigure` thì **xoá** tham số (chương trình mới).

## 4. Bất biến phải giữ

- `EXEC_ARGS_BASE % 16 == 0` và `EXEC_ARGS_BASE > EXEC_STACK_BASE` (assert
  compile-time trong `vifast-abi`).
- Tổng chuỗi ≤ `EXEC_ARGS_STR_BYTES` (192 B). Vượt ⇒ `write_exec_args` trả
  `None` ⇒ spawn fail-êm, **không** cắt bớt chuỗi.
- Vùng stack exec nằm trong `COPY_WINDOWS` — đã có test
  `copy_windows_cover_user_regions` canh `EXEC_STACK_TOP − 8`.
- Chương trình **không** được giả định `argv` nằm ở địa chỉ cố định; chỉ dùng
  `a1`. (Ảnh demo một trang là ngoại lệ có chủ ý: nó đọc cặp `(ptr,len)` tĩnh
  trong chính trang code của nó và in `HI FS`.)

## 5. Bài học r247 — `sp` ở đỉnh trang là lỗi

Bản đầu tiên của r247 đặt `sp = EXEC_STACK_TOP` (khối argv nằm dưới `sp`, tức
**trong vùng stack sẽ mọc xuống**). Kết quả đo được:

- riscv64 và aarch64: `BIN OK args` + `argv: HIARG` — **đúng**.
- x86_64: `BIN OK args` + `argv:` **rỗng**, và cổng đỏ vì thiếu `HIARG`.

Nguyên nhân: prologue của `bin_main` trên x86 (SysV, `-C target-feature` mặc
định) cộng thêm khung của `write`/`debug_write` ăn ~256 B stack, tức `rsp` đi
từ `EXEC_STACK_TOP − 8` xuống dưới `EXEC_ARGS_BASE` và **xoá sạch khối argv**
trước khi `run_args` kịp đọc. Hai arch kia có khung nhỏ hơn nên "may mắn" đúng.

⇒ Quy tắc: **dữ liệu do kernel/người spawn đặt cho chương trình phải nằm NGOÀI
vùng stack tăng trưởng**, hoặc `sp` phải ở dưới dữ liệu đó. Ở đây chọn cách hai
(`sp = EXEC_ARGS_BASE`), vì nó rẻ và không cần thêm trang.

Bài học phụ (cùng vòng): self-test exec ở boot phải `tcb_wait` rồi `destroy`
**ngay**, nếu không frame/TCB/VSpace của lần exec cuối bị giữ lại. Đo được:
`DATA UNTYPED objects` cuối boot 74/86 trước khi sửa, 68/86 sau khi sửa; trước
khi sửa, x86_64 có 1/3 lần đỏ vì thiếu `MEM OOM OK`.

## 6. Phạm vi chưa làm

- Chưa có `envp`, chưa có `auxv`.
- Chưa có `exec` trả lỗi cho người gọi (vẫn fire-and-forget qua `LABEL_RUN`).
- Chưa hỗ trợ tham số dài hơn 192 B tổng; muốn vậy phải chuyển khối argv sang
  một trang riêng (tăng 1 frame/exec — cân nhắc với ngân sách Untyped).
