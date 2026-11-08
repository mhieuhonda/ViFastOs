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
