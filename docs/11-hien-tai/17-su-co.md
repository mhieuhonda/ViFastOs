# 17. Runbook sự cố — triệu chứng, nguyên nhân đã gặp, cách xử lý

Tài liệu này gom **những sự cố thật đã gặp** khi vận hành ViFastOs, kèm lệnh cụ
thể. Nguyên tắc: **xác minh trước, sửa sau** — gần như mọi lần tôi "đoán
nguyên nhân" trong phiên này đều đoán sai ít nhất một lần.

## 0. Trước mọi lần chạy cổng

```bash
./scripts/diag.sh          # HEAD, cây bẩn?, số trang module, headroom, arena, qemu-system
pkill -f qemu-system       # nếu dòng "qemu-system:" khác 0
```

`diag.sh` in cả **arena** (`DATA UNTYPED pages=x/N`) của ba arch lấy từ serial
gần nhất — đây là con số quyết định có nên thêm tính năng vào client/root hay
không (xem mục 3).

## 1. Cổng QEMU đỏ vì `PREEMPT OK`

| Triệu chứng | Nguyên nhân thường gặp | Cách xác minh | Xử lý |
|---|---|---|---|
| thiếu **đúng** `PREEMPT OK` | QEMU mồ côi chiếm CPU | `./scripts/diag.sh` → `qemu-system: N` | `pkill -f qemu-system`, chạy lại |
| thiếu `PREEMPT OK` + `SMP IDLE OK` + `SMP POST IDLE OK` | kernel của cổng **thiếu feature `test-exit`** | `python3 -c "d=open('target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64','rb').read(); print(d.count(b'PREEMPT OK'))"` phải ra **2** | chạy `./scripts/build.sh`; **không** build tay `--no-default-features` vào cùng target |
| guest in rất ít dòng | flake UEFI/QEMU (thường x86_64) | đếm số dòng serial | giờ đã **tự thử lại 1 lần** (vòng 162) |

**Bài học**: lỗi "thiếu PREEMPT OK" đã làm tôi đi sai hướng ba vòng
(149/151/153) — nguyên nhân thật là chính tôi build tay đè kernel cổng. Từ vòng
153 `test-qemu.py` có `check_gate_kernel()` chặn trước khi boot.

Khi cần kernel **không** có `test-exit` để thử tương tác, build sang target dir
khác:

```bash
cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf \
  --release --no-default-features --target-dir /tmp/tgt-noexit
```

## 2. Cổng đỏ vì marker khác

| Marker thiếu | Nghĩa | Xử lý |
|---|---|---|
| `SYS REJECT OK` / `SEC PROBE OK` | phép thử từ chối dùng **slot giả định trống**; slot đó có thể đã hợp lệ khi arena lớn lên | dùng slot **ngoài phạm vi** (`200`) — đã sửa ở vòng 161 |
| `MEM FREE OK` | arena cạn (`used >= size`) | nới arena (mục 3) hoặc thu hồi thêm |
| `FS MOUNT CLEAN` | cấu trúc đĩa bất nhất ngay khi mount | xem serial có `FS MOUNT DIRTY`; kiểm tra thay đổi định dạng đĩa gần nhất |
| `KILL OK` | `tcb_destroy` không thành công hoặc chấp nhận slot sai | kiểm tra thay đổi gần nhất ở đường destroy/retype |

`test-qemu.py` in rõ **tên marker** thiếu; nếu chỉ có `EXACT_ONE` lặp hoặc
`FORBIDDEN` xuất hiện, nó in cả hai danh sách (vòng 153).

## 3. Build đỏ vì ngân sách (đây là loại đỏ **đúng**)

```
client(aarch64) 29024 B = 8 trang (ngân sách 7)
CLIENT ELF aarch64 VƯỢT NGÂN SÁCH TRANG (tối đa 7)
RISC-V KERNEL HEADROOM BELOW 192 KiB: ...
```

- Client: riscv **7** / aarch64 **8** / x86_64 **9** trang (ngân sách trong
  `scripts/build.sh`, hàm `check_client_budget`).
- Root: `.smp` root tăng **1 trang** ⇒ kernel nhúng ELF phình **+4 KiB**.
- Kernel riscv: cửa sổ `0x8020_0000..0x8060_0000`, ngưỡng dư **192 KiB**.

**Cách xử lý theo thứ tự ưu tiên**:
1. Đo trước: `./scripts/diag.sh` (số trang + headroom + arena).
2. Giảm chỗ khác thay vì nới (bỏ bảng tĩnh không cần, gộp đường mã).
3. Thu hồi bộ nhớ thật: trang staging module đã được trả về arena
   (`ModuleRelease`, vòng 160) — riscv thu 21, aarch64 22, x86_64 23 trang.
4. Nới **có kỷ luật**: sửa **cả ba chỗ** (`struct DataPool`, static initialiser,
   slice trong `data_page()`) rồi đo lại; ghi số vào `16-van-hanh.md` mục 5z.

## 4. Sự cố khi thêm self-test vào guest

| Triệu chứng | Nguyên nhân | Xử lý |
|---|---|---|
| panic `root slot range exhausted` | gọi `slots.alloc()` khi cấp phát slot | dùng slot **đã destroy** (`probe_endpoint_slot`) — vòng 139 |
| marker mới in `BAD` sau khi arena lớn | phép thử dựa vào "slot 63 trống" | dùng slot ngoài phạm vi (`200`) — vòng 161 |
| một marker cuối biến mất | đoạn chèn nằm sai nhánh/closure | đọc lại ngữ cảnh bằng `sed -n`, không tin vị trí dòng |

## 5. Sự cố khi chèn test Rust

- **Trùng `#[test]`** hoặc làm mất doc comment của test cũ ⇒ `warning: duplicated
  attribute`, `function ... is never used`, và build vẫn "xanh" nếu không đọc
  warning. Luôn kiểm `grep -c "warning:" <log>` phải bằng **0**.
- Chèn theo **mốc hàm** (`fn ten_ham(`) rồi xem lại bằng `sed -n`, vì `cargo
  fmt` reflow làm lệch mọi giả định về số dòng.
- Tên API/hằng số phải đọc từ **mã nguồn**, không theo trí nhớ (đã đoán sai
  `fs.rmdir` vs `fs.unlink`, magic `ViFs` vs `VF04`, lỗi `Invalid` vs `NoSpace`).

## 6. Sự cố tài liệu

| Triệu chứng | Xử lý |
|---|---|
| `gen-docs: kernel sources changed since the standard QEMU run` | chạy `python3 scripts/test-qemu.py --repeat 3` (đủ 3 arch), không dùng `--arch` |
| `check-docs: 09-testing.md thiếu marker ...` | sinh lại danh sách từ `scripts/harness_spec.py` (xem lệnh trong log vòng 162) |

## 7. Git

- Commit từ tiến trình khác (web/branding) có thể **quét** thay đổi của phiên
  này vào commit của họ. Trước khi báo xong: `git status` và `grep` xác nhận
  thay đổi đã nằm trong `HEAD`.
- `content/` và `LICENSE.bak` là untracked có chủ ý — **không** commit.

## 8. Bảng nhanh

```bash
./scripts/diag.sh                      # tình trạng tổng thể
pkill -f qemu-system                   # dọn QEMU mồ côi
./scripts/build.sh                     # build + unit test + ngân sách
python3 scripts/test-qemu.py --repeat 3 # cổng QEMU 3 arch × 3 lần
python3 scripts/test-persist.py        # cổng bền vững đĩa
python3 scripts/test-harness-spec.py   # đột biến marker
python3 scripts/gen-docs.py && python3 scripts/check-docs.py
cargo fmt --all -- --check
```
