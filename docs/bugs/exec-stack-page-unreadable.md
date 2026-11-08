# Lỗi: chương trình exec không đọc được trang RW (stack)

Trạng thái: **đang mở**, đã khoanh vùng mạnh (vòng 112–119).

## Triệu chứng

Chương trình exec (nạp qua `spawn_elf_copy` hoặc `spawn_elf_from_fs`) chạy
được, đọc được **trang code** (`R|X`), nhưng mọi truy cập vào **trang stack**
(`R|W`, map tại `EXEC_STACK_BASE`) đều không có kết quả: `DebugWrite` từ địa
chỉ đó không in gì, task chết im lặng (exec TCB không có fault endpoint).

## Bằng chứng đã thu (đều trên riscv64, kernel xanh)

| # | Thí nghiệm | Kết quả |
|---|---|---|
| 1 | Đọc `entry+0x40` (trang code), `auipc` ở lệnh đầu | in đúng `HI FS` ✔ |
| 2 | `frame_write(stack_frame, off, msg)` tại `0xFF8`/`0x100` | trả `Ok`, không có `STK WRITE BAD` |
| 3 | Root in hằng số layout | `0x10_0002_3000`/`0x10_0002_4000` — đúng |
| 4 | Đọc `EXEC_STACK_TOP-8` | **không in** |
| 5 | Đọc `EXEC_STACK_BASE+0x100` | **không in** |
| 6 | Đổi VA stack sang `0x10_0000_2000` (kề trang code) | **không in** |
| 7 | Dùng `map_with_tables` (đường client) thay `map_frame_fallible` | **không in** |
| 8 | Map stack TRƯỚC code (đảo thứ tự) | **không in** |
| 9 | Map LẠI đúng VA/frame sau khi map (`frame_map` lần 2) | trả **`AlreadyMapped (8)`** ⇒ kernel **có** ghi nhận mapping |

## Đã loại trừ

- **Sai địa chỉ/offset**: đã tính tay + in hằng số từ root + `auipc` đặt ở lệnh
  đầu (bài học vòng 113: `auipc` lấy PC của chính nó).
- **`frame_write` hỏng**: trả `Ok` cho cả hai offset.
- **Lỗi kernel core (walk bảng, ghi leaf)**: test host
  `maps_two_pages_in_one_fresh_vspace` map 2 trang trong một vspace mới, kiểm
  **PTE thật** → PASS (kể cả khi assert trực tiếp `FAKE_MEM`).
- **Lỗi do `map_frame_fallible`**: thay bằng `map_with_tables` vẫn hỏng.
- **Lỗi do thứ tự map**: đảo thứ tự vẫn hỏng.
- **Thiếu bit A/D**: đã set `PTE_A|PTE_D` (vòng 115) — không đổi gì.
- **Kernel không ghi nhận mapping**: `frame_map` lần hai trả `AlreadyMapped`
  ⇒ sổ mapping có entry; `map_leaf` đã chạy không lỗi (mọi lỗi đều propagate).

## Giả thuyết còn lại (ưu tiên giảm dần)

1. **Bất thường ở platform riscv `encode_leaf` cho quyền RW**: PTE có
   `V|U|A|D|R|W` nhưng CPU vẫn fault → cần đọc PTE thật trong QEMU
   (`info mem`/`xp`) hoặc thêm `klog` phía platform.
2. **VSpace window**: `VSpace::with_page_size(pa, USER_VA_START, size, page)` —
   nếu `end` tính sai thì `map_leaf` bỏ qua (nhưng `frame_map` vẫn Ok vì
   mapping được ghi vào sổ).
3. **`tcb_configure` + `set_tcb_vspace`**: task chạy với vspace khác root?
   (đã chứng minh không phải: đọc code page trả đúng dữ liệu ta ghi).

## Cách kiểm tra nhanh ở vòng sau

```sh
# 1) Xem PTE thật trong QEMU monitor cho VA 0x10_0002_3100
python3 scripts/test-qemu.py --interactive --arch riscv64   # rồi dùng monitor
# 2) Hoặc thêm klog trong platform riscv map_leaf: in va, pte, table_pa
```

## Bổ sung vòng 120

- ASID: `take_asid` cấp ASID **duy nhất** cho mỗi vspace, không bao giờ 0, và
  vòng đời ASID gắn với VSpace còn sống ⇒ khó xảy ra TLB stale do trùng ASID.
- `frame_map` lần hai trả `AlreadyMapped` ⇒ cả sổ mapping **và** đường
  `map_leaf` đều chạy đúng; nghi vấn dồn về **giá trị PTE mà CPU đọc** (encode
  RW của platform) hoặc **cửa sổ VA của VSpace** (`space.end`).

### Đề xuất kiểm tra kế tiếp (rẻ nhất trước)

1. Trong platform riscv, tạm thời `klog` (hoặc ghi ra serial trực tiếp bằng
   `putc`) ngay sau `write_pte` trong `map_leaf`: in `table_pa`, `idx`, `pte`.
2. So `pte` nhận được với PTE mong đợi `0x10_0002_3xxx`.
3. Nếu PTE đúng → vấn đề ở `satp`/switch vspace; nếu sai → lỗi encode.

## Bổ sung vòng 121 — PTE THẬT (trace trong platform)

Bật in PTE ngay trong `PageOps::write_pte` khi `idx == 1` (trang code) hoặc
`idx == 0x23` (trang stack). Kết quả một lần boot:

```
T:0000000080382000 I:0000000000000001 P:00000000200dc4db   ← code (R|X|U|A|D)
T:0000000080382000 I:0000000000000023 P:00000000200e0cd7   ← stack (R|W|U|A|D)
```

- Cùng một bảng (0x80382000), hai entry 1 và 0x23, **hai PTE đều hợp lệ**:
  PPN trỏ PA 0x80371000 / 0x80383000, cờ `V|R|~~W~~|X|U|A|D` đúng như thiết kế.
- ⇒ **Không phải PTE hỏng**. Giả thuyết "trang không được map" coi như bị bác.
- Đáng chú ý: trace còn bắt được một lần ghi **PTE = 0** vào `idx 1`
  (`P:0000000000000000`) ở bảng khác (0x8036c000) — tức có thao tác **unmap**
  trang code của một vspace nào đó trong lúc boot. Cần đối chiếu xem unmap này
  có rơi vào vspace của task exec đang chạy hay không.
- Thêm dữ kiện: serial có `USER FAULT ... cause=0x2 addr=0x0` (illegal
  instruction) ngay sau khi chương trình in byte đầu — gợi ý luồng thực thi bị
  lạc sau một syscall, không phải lỗi đọc bộ nhớ.

### Hướng đi tiếp (hẹp)

1. Đối chiếu thời điểm `destroy_exec`/`object_destroy` với thời điểm task exec
   chạy (thêm trace unmap kèm VA/table).
2. Kiểm tra `cause=0x2`: task exec có bị `tcb_configure` lại entry sai không
   (gen=3 cho thấy TCB được tái dùng).

## Bổ sung vòng 122 — chương trình dài fault `cause=2` (illegal instruction)

- Chương trình test 18 lệnh (in byte từ trang code ở `entry+0x80`, in chuỗi từ
  trang stack, in byte kiểm tra ở `entry+0x81`) **không in gì** và kết thúc
  bằng `USER FAULT ... cause=0x2 addr=0x0` (illegal instruction).
- Chương trình ngắn hiện tại (8 lệnh, đọc cặp `(ptr,len)` trong trang code)
  **vẫn chạy đúng** trong cùng bản build.
- ⇒ Nghi vấn mới: chương trình dài hơn **52 byte** (0x120..0x168) chạm vùng
  nào đó khiến PC nhảy vào bộ nhớ 0; cần kiểm tra (a) độ dài segment
  `filesz=0x60` có phủ đủ code + vùng phụ không, (b) `e_entry`/`p_memsz` có bị
  lệch khi code vượt quá vùng 0x140 mà `write_program_args` ghi đè.
- Đây là **lỗi của test harness tự chế**, chưa kết luận gì về lỗi stack; ghi
  lại để lần sau viết test ngắn (≤ 8 lệnh) và kiểm tra từng bước.

## Bổ sung vòng 123 — frame & PTE khớp nhau, dữ liệu nằm đúng PA

Trace mới (bật lại PTE + probe map frame stack ở VA thứ hai):

```
T:80380000 I:23 P:00000000200e04d7     ← PTE trang stack  → PA 0x80381000
T:80380000 I:24 P:00000000200e04d7     ← PTE probe (0x24000) → cùng PA
PMAP 0 68719624192                     ← frame_map trả VA 0x10_0002_4000
```

- Hai mapping (0x23000 và 0x24000) trỏ **cùng một frame PA 0x80381000**.
- Root đã ghi `"HI FS"` vào frame đó ở offset 0x100 ⇒ dữ liệu nằm đúng PA và
  đúng VA (0x23100).
- **Kết luận**: không phải lỗi map, không phải lỗi PTE, không phải lỗi ghi.
  Nghĩa là khi chương trình đọc `0x23100` nó **không nhận được** `"HI FS"`,
  tức hoặc task chạy với **vspace khác**, hoặc chương trình **không chạy**.
- Loại trừ thêm: map với `R|X` (thay `R|W`) vẫn không đọc được ⇒ không phải
  bit W.

### Việc cần làm ngay (một thí nghiệm duy nhất)

Ghi một **giá trị đặc trưng** vào frame stack (ví dụ `0x41424344`) rồi cho
chương trình đọc 4 byte ở `0x23100` và in ra; so sánh với việc đọc cùng VA
bằng một *task khác chắc chắn chạy* (worker). Nếu worker thấy giá trị mà task
exec không thấy ⇒ **task exec chạy sai vspace**.

## Bổ sung vòng 124 — task CHẠY trong vspace của chính nó (bằng chứng dương)

- Ghi `"TEST"` vào **trang code** của exec tại offset `0x1a0` (ngoài vùng code +
  ngoài vùng tham số), chương trình đọc 4 byte ở `entry+0x80` và in.
- Kết quả serial: `TESTPAGER OK` — chuỗi `TEST` in ra **dính liền** `PAGER OK`
  ⇒ task exec **thực sự chạy trong vspace của nó** và đọc được trang code của
  chính nó.
- Vậy hai khả năng "task chạy sai vspace" và "chương trình không chạy" đều bị
  loại. Khác biệt duy nhất còn lại giữa hai trang: **trang RW**.
- Bước tiếp theo (rất hẹp): kiểm tra `validate_user_range()` — nếu nó từ chối
  VA trong trang stack, `DebugWrite` trả `InvalidArg` và chương trình **im
  lặng** (đúng triệu chứng). Cần một chương trình ngắn in mã lỗi trả về
  (`O`/`E`) — lưu ý đã có 2 lần thất bại vì chương trình dài chạm vùng tham số.

## Bài học quy trình (vòng 125) — sửa code trong `spawn_elf_copy` rất dễ lạc chỗ

Nhiều lần kết quả "không in gì" là do **bản sửa không rơi vào hàm đích**:

- `let mut rights = Rights::READ;` xuất hiện ở cả `load_module` và `spawn_elf_copy`
  → chèn theo mẫu này có thể ghi vào hàm sai, khiến frame không được ghi và
  chương trình in ra NUL.
- `let code: [u32; 8]` cũng xuất hiện ở 3 nhánh arch → thay theo mẫu dễ rơi vào
  nhánh khác.

**Quy tắc**: sau mỗi lần sửa, `grep -n` kèm **ngữ cảnh hàm** để xác nhận dòng đã
đổi nằm giữa `fn spawn_elf_copy(` và hàm kế tiếp; và in dấu hiệu (như `TEST`)
trước khi kết luận bất cứ điều gì về bộ nhớ.

## Vòng 126 — chương trình 8 lệnh in errno cũng KHÔNG in

- Sửa đúng vào nhánh riscv của `exec_demo_elf` (xác minh bằng `sed -n` sau khi
  ghi): 8 lệnh `auipc a0,0x22 ; addi a0,a0,-0x20 ; a1=4 ; a7=32 ; ecall ;
  a0+=0x30 ; a1=1 ; ecall` (in `'0'+errno` của `DebugWrite(0x23100,4)`).
- Kết quả: sau `EXEC OK` **không có ký tự nào** (serial nhảy thẳng sang demo
  Fault4) ⇒ ecall đầu tiên hoặc không chạy, hoặc không trả về như mong đợi.
- Đối chiếu: chương trình 8 lệnh đọc **trang code** (`entry+0x80`) thì in được
  (`TESTPAGER OK`, vòng 124). Hai chương trình chỉ khác **giá trị a0**
  (`0x23100` vs `entry+0x80`).
- ⇒ Nghi vấn mạnh: `DebugWrite` với VA nằm trong trang stack khiến **task chết
  ngay trong syscall** (không kịp trả lỗi về user), hoặc kernel `return`s mà
  không cập nhật PC.

### Trạng thái

Điều tra **tạm dừng** ở đây theo hướng chuyển sang hạng mục khác của mục tiêu;
báo cáo này đã đủ dữ kiện cho một người có quyền sửa kernel kết luận nhanh.

## Vòng 142 — kiểm chứng trong môi trường SẠCH: lỗi vẫn tái hiện

Bối cảnh: vòng 141 phát hiện "QEMU mồ côi" có thể gây timeout `PREEMPT OK`, nên
nghi các kết luận trước đây bị nhiễu. Đã kiểm tra lại **có kiểm soát**:

- Xác nhận `ps` không còn tiến trình `qemu-system` nào trước khi chạy.
- Dựng lại đúng thí nghiệm: root ghi `"HI FS\n"` vào frame stack offset `0x100`
  **trước** khi map; chương trình 8 lệnh đọc `EXEC_STACK_BASE + 0x100`.
- Kết quả: **`HI FS` count = 0**; serial sau `EXEC OK` đi thẳng sang demo khác,
  **không có byte NUL nào** ⇒ lời gọi `DebugWrite` không phát ra gì, chứ không
  phải "in ra chuỗi rỗng".
- Kết luận: đây là **lỗi thật, tái hiện được**, không phải flake môi trường.

### Trạng thái các giả thuyết sau vòng 142

| Giả thuyết | Trạng thái |
|---|---|
| PTE sai/thiếu | **Bác** (trace PTE hợp lệ, cùng bảng với trang code) |
| Ghi sai frame/offset | **Bác** (PA khớp, `frame_write` = Ok) |
| Task chạy sai vspace | **Bác** (`TESTPAGER OK` đọc được dữ liệu do root ghi) |
| `validate_user_range` từ chối | Chưa quan sát trực tiếp được (chương trình in errno không chạy — do test tự chế lỗi) |
| Huỷ ánh xạ sau đó (unmap) | **Còn mở** (trace bắt được một lần ghi PTE=0 ở bảng khác) |
| Flake môi trường/QEMU | **Bác** (đã chạy trong môi trường sạch) |
