# Dùng nhanh ViFastOs trong 10 phút

Mục tiêu: từ máy sạch tới **shell chạy được**, và biết **cách tự kiểm** hệ thống
thay vì tin vào lời mô tả. Tài liệu này dành cho người **dùng thử**; muốn hiểu
kiến trúc thì đọc `16-van-hanh.md` và `03-ipc-scheduler.md`.

## 1. Cần gì

- Rust (stable) + `cargo`, `qemu-system-riscv64`, `qemu-system-aarch64`,
  `qemu-system-x86_64` (x86 cần thêm OVMF).
- Không cần phần cứng thật: mọi thứ chạy trong QEMU.

## 2. Dựng và chạy

```bash
git clone <repo> ViFastOs && cd ViFastOs
./scripts/build.sh          # build hết, kiểm ngân sách trang + bảng tài liệu
```

Chạy gate đầy đủ (khuyến nghị trước khi tin bất cứ thay đổi nào):

```bash
./scripts/check-all.sh      # format + host test + QEMU 3 lần/arch + persist + docs
```

Chạy riêng một arch:

```bash
python3 scripts/test-qemu.py --arch riscv64 --repeat 1
```

Kết quả tốt phải thấy (`--repeat 1`, mỗi arch):

```
=== summary ===
riscv64  1/1 PASS
```

## 3. Tour shell (ví dụ thật, đo trên máy ảo)

```
shell> free
28/344 KiB free
shell> uptime
2s
shell> df
12/8162 blocks [#...................]
shell> ls
motd
page.bin
scratch
prog.elf
shell> ps
0 15 4 aff=1 root
58 5 4 aff=1 rety
...
shell> loop 6
loop sent
LOOP OK
shell> free
28/344 KiB free
```

(Ba con số `28/344`, `12/8162`, `2s` là **đo thật** trên QEMU riscv64; máy chậm
hơn sẽ ra `uptime` lớn hơn, nhưng `free` phải **giữ nguyên** trước/sau `loop`.)

Điểm cần hiểu:

- **`free` trước và sau `loop 6` phải bằng nhau** (đo được: `28/344` → `28/344`). Đây là phép kiểm rò rỉ bộ nhớ
  mà bạn tự làm được, không cần harness. Nếu `free` giảm sau mỗi lần `loop` thì
  đó là **lỗi thật**, hãy báo kèm hai con số.
- **`df`** in tỷ lệ dùng FS: `used/total blocks` cộng thanh 20 ô; một ô `#` xuất
  hiện ngay cả khi tỷ lệ rất nhỏ để mức dùng không bị "vô hình".
- **`uptime`** dạng `1m05s` (dưới 1 phút in `42s`).
- **`ps`** cho biết task nào đang sống; **`jobs`/`kill <slot>`** để quản lý.

Danh sách đầy đủ 26 lệnh: gõ `help`. Bảng mô tả từng lệnh được **sinh tự động**
từ mã nguồn (xem `16-van-hanh.md` mục 2c) nên không thể lệch.

## 4. Tự kiểm bằng số, không bằng cảm giác

| Câu hỏi | Lệnh | Điều kiện ĐÚNG |
|---|---|---|
| Còn bao nhiêu bộ nhớ? | `free` | `free/total KiB`; đọc **số đầu** (ví dụ `28/344`) |
| Chạy lặp có rò rỉ? | `loop 6` rồi `free` | `LOOP OK` và `free` **không đổi** |
| FS còn chỗ? | `df` | `used/total` + thanh tỷ lệ |
| Hệ thống sống bao lâu? | `uptime` | `0s`, `1m05s`… |
| Task nào đang chạy? | `ps` | mỗi dòng `slot prio state aff=… tên` |

Nếu một lệnh trả `unknown cmd`, lệnh đó **không tồn tại** trong bản này (đừng
đoán theo tài liệu cũ).

## 5. Khi cổng đỏ thì đọc gì

- `FAIL: thiếu marker (...)` → guest không in marker đó; xem tên marker trong
  bảng sinh tự động ở `16-van-hanh.md` mục 5.
- `persist` đỏ với `blank missing FS FORMAT OK` → đã có nhánh thử lại hẹp; nếu
  **vẫn** đỏ sau khi chạy lại thì là lỗi FS thật.
- `guest dừng trước SHELL READY` → nghi flake boot; script tự thử lại **một**
  lần. Nếu lần hai vẫn hỏng thì đọc serial, đừng bỏ qua.

Runbook chẩn đoán đầy đủ: `17-su-co.md`.

## 6. Giới hạn đã biết (đừng kỳ vọng)

- **Không có FP/vector cho người dùng**: chương trình dùng FP sẽ trap (có marker
  `EL0 FP TRAP`). Đây là lựa chọn có chủ ý, xem `07-arch-ports.md`.
- **Client riscv chỉ có 8 trang** ELF: thêm tính năng vào shell phải đo ngân sách
  (`./scripts/build.sh` sẽ chặn nếu vượt).
- **Arena riscv còn ~7 trang trống** sau khi client nạp: đủ cho một lần exec,
  không đủ cho nhiều task exec đồng thời.
