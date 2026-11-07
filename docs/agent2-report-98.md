# ViFastOs — Báo cáo vòng 98 (x86 parity: bàn phím PS/2 + NX sau IDT)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `463828d`
**Bối cảnh:** audit #9/#10 và mốc "x86_64 parity".

## 1. x86 có bàn phím (parity với virtio-input)

Trước vòng này x86 **không có đường input nào** (IRQ1 bị mask, không đọc port
0x60), nên `--input-check` phải in SKIP.

Đã thêm:
- `irq1_entry` trong `trap.S` (khuôn `irq4_entry`).
- IDT vector `0x21`, master PIC IMR `0xEC` (IRQ0+IRQ1+IRQ4).
- Khởi tạo i8042 theo thứ tự an toàn: `AD → flush OBF → đọc CCB → sửa CCB
  (bit0=1, bit6=1, xoá clock-inhibit) → AE → flush → F4 + chờ ACK → unmask
  IRQ1 sau cùng`.
- `x86_keyboard_irq_handler`: đọc hết byte đang chờ (bounded 16), bỏ tiền tố
  `0xE0`, theo dõi Shift, **chặn auto-repeat** bằng bitmap `DOWN`, log
  `PS2 KEY <scancode>` (bounded), `stash_rx` byte ASCII rồi `irq_raise(IRQ_UART)`
  + `wake_hart(1)` nếu cần.
- `input_check_x86()` trong `test-qemu.py`: boot Q35+OVMF, gửi `sendkey e` sớm
  và lặp (guest x86 thoát nhanh sau `SHELL READY`), yêu cầu **cả**
  `PS2 KEY 18` **và** `shell> e`.

Kết quả: `x86_64 input PASS (PS/2 keyboard reached the shell)` — chuỗi
PS/2 → IRQ1 → handler → UART RX → shell echo hoạt động thật.

## 2. NX: đóng hai tồn đọng audit #9

- `image_pages_nx()` chuyển xuống **sau** `setup_gdt_tss_idt()`: nếu mapping
  sai thì fault còn in được thay vì triple fault trước khi có IDT.
- Trang trampoline AP (0x9F000) được miễn NX **nếu** ảnh được nạp thấp tới mức
  chứa nó (thực tế base 0xdc4b000 nên không cần, marker vẫn `ranges=3`).

## 3. Kernel: K4 từ review bên ngoài

`make_ready` trước đây set `Ready` mù quáng: có thể đặt Ready cho task đang
`Running` trên hart khác (chạy hai lần), tự gỡ treo task `Suspended`, hoặc cho
task `BlockedBudget` chạy lố budget. Nay chỉ đánh thức từ
`Inactive/Ready/Blocked*`; các state khác giữ nguyên. 2 test mới.

Hai phát hiện còn lại:
- **K5** (suspend vs BlockedReply): hành vi hiện tại là hợp đồng "muốn treo
  server đang giữ client thì phải destroy"; đã ghi rõ, không sửa.
- **K6** (`charge_current_budget` trừ nhầm người): **không đúng** — hàm đã có
  guard `state != Running → return` nên task Blocked không bị trừ budget.

## 4. Kiểm chứng

- Kernel **146 test**, FS 25, harness 508.
- QEMU 3/3 cả ba kiến trúc; `--input-check` x86 2/2, aarch64 2/2, riscv 1/1.
- `gate.sh --with-tcp --with-input`: all steps passed (một lần qemu flake, chạy
  lại 3/3 rồi gate xanh).
