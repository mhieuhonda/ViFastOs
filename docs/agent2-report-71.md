# ViFastOs — Báo cáo vòng 71 (demand-load nạp trang từ VFS thật)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `e54a1a3`
**Mốc:** Trang demand-load giờ được nạp từ **file `/page.bin` trong VFS**,
không còn là ảnh tĩnh trong server.

## 1. Thay đổi

- **ABI**: `BADGE_PAGER=3`; `pager::{DEMAND_FILE="/page.bin", DEMAND_VALUE,
  DEMAND_TAIL, DEMAND_BYTES=4096}`; `user_layout::PAGER_IPC_BUF_VA` (trang
  IPC buffer riêng của pager server, ngay sau stack pager3).
- **VFS server**: tạo `/page.bin` 4 KiB, nội dung xác định (byte i = i XOR
  0x5a, 8 byte đầu = DEMAND_VALUE, 8 byte cuối = DEMAND_TAIL), ghi theo khối
  128 B để **không cần static 4 KiB** (tránh server ELF phình frame trên x86).
  Log `FS PAGE FILE OK`.
- **Root**: cấp cho pager server một frame IPC buffer (map tại
  `PAGER_IPC_BUF_VA` trong root VSpace) + `tcb_set_ipc_buffer`, và mint cap
  VFS endpoint vào slot 11 với badge 3.
- **Pager server**: khi fault PAGER4_VA, retype frame rồi gọi VFS
  `OPEN → READ×32 → CLOSE`, mỗi khối 128 B ghi vào frame bằng `frame_write`,
  cuối cùng kiểm tra 8 byte cuối == DEMAND_TAIL. Log `DEMAND VFSLOAD OK` hoặc
  `DEMAND VFSLOAD BAD`.
- x86_64 giữ đường ảnh tĩnh cũ (hết Untyped cho frame IPC buffer).

## 2. Kết quả

- riscv64, aarch64: `FS PAGE FILE OK` + `DEMAND VFSLOAD OK` +
  `DEMAND LOAD OK` (client đọc đúng giá trị → chứng minh nội dung trang đến
  từ file).
- x86_64: `DEMAND LOAD OK` (đường ảnh tĩnh) — không hồi quy.
- Harness: `DEMAND VFSLOAD OK` exact-one trên riscv64/aarch64,
  `DEMAND VFSLOAD BAD` bị cấm; 498 → **501 cases**.

## 3. Lỗi gặp và cách xử lý

- **x86 panic OutOfMemory**: đặt static `[u8; 4096]` trong server làm ELF lớn
  thêm 1 trang → thiếu frame khi load module. Sửa: sinh file theo chunk 128 B,
  không static.
- **Marker nhiễu**: `DEMAND LOAD VFS OK` chứa chuỗi con `VFS OK` làm harness
  đếm `VFS OK` thành 2. Đổi thành `DEMAND VFSLOAD OK`.

## 4. Cổng

- QEMU 3/3×3, persist 7×3, harness 501, build 0 warning, `fmt` PASS.
- AArch64 static vẫn giữ khe 8 byte; RISC-V headroom 330352.
