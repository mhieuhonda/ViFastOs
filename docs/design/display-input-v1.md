# Thiết kế framebuffer + input MVP (dự thảo cho vòng 25)

Trạng thái: **draft**, chưa có code. Mục tiêu: đưa ViFastOs từ console text
sang framebuffer + bàn phím, đủ để chạy shell và demo đồ hoạ đơn giản.

## 1. Phần cứng mục tiêu

| Kiến trúc | Framebuffer | Input |
|---|---|---|
| x86_64 | GOP (UEFI) hoặc Bochs VBE (`0x1234:0x1111`) | PS/2 keyboard (IRQ1) |
| AArch64 | ramfb qua fw_cfg DMA | virtio-keyboard (MMIO) |
| RISC-V | ramfb qua fw_cfg DMA | virtio-keyboard (MMIO) |

MVP chọn **ramfb + virtio-keyboard MMIO** cho RISC-V/AArch64. x86 dùng GOP do
OVMF cung cấp; **chưa có PCI enumeration** nên x86 MVP dùng **PS/2 keyboard**
(IRQ1 qua PIC) thay vì virtio-keyboard. Nếu GOP không có thì fallback Bochs VBE.

## 1.1 ramfb: fw_cfg DMA và bộ nhớ liên tục

- ramfb không phải PCI: QEMU cấu hình qua fw_cfg (`etc/ramfb`), ghi struct
  `RAMFBCfg { addr, fourcc, flags, width, height, stride }` vào file fw_cfg.
- Cần **PA liên tục**: cấp một Frame lớn (2 MiB) từ Untyped (retype nhiều page
  liền nhau hoặc thêm `Frame::contiguous`) và ghi PA vào `addr`.
- fw_cfg DMA: QEMU virt (RISC-V/AArch64) đặt fw_cfg ở **MMIO** (base từ device
  tree/QEMU virt constant), x86 dùng **I/O port** `0x510/0x514`. Driver
  user-space được cấp device Frame/MMIO window qua BootInfo (không tự chọn PA),
  map bằng `IoMap`/`FrameMap` rồi mới ghi DMA address.
- Không có IOMMU cứng nên QEMU thấy PA thật; phải đảm bảo vùng PA nằm ngoài
  kernel image và không bị reclaim.

## 2. Driver model

- Mỗi driver là một **user-space service** giữ cap thiết bị:
  - `BlockDevice`-like: thêm `Framebuffer` và `InputDevice` object type.
  - MMIO window cấp qua `IoMap` (software IOMMU) hoặc `FrameMap` device.
- Kernel chỉ cung cấp:
  - phát hiện thiết bị bằng nguồn hiện có: MADT cho x86, hằng địa chỉ QEMU virt
  cho RISC-V/AArch64 (DT parser chưa có — việc của vòng sau);
  - IRQ object cho keyboard;
  - shared Frame cho framebuffer, map qua `FrameMap` thường (write-combining
    cần thêm cờ trong PageTableOps — chưa có, ghi là future).

## 3. Framebuffer protocol

```text
FB_INFO   (server → client) data=[width, height, stride, format, frame_cap]
FB_FLUSH  (client → server) data=[x, y, w, h]
FB_MAP    (root  → server)  data=[paddr, size, format]
```

- Server map framebuffer PA vào VSpace của client với quyền WRITE (không EXEC).
- `FB_FLUSH` chỉ cần cho ramfb (copy shadow buffer); GOP/linear thì không.
- Format MVP: `XRGB8888`, stride 4×width.

## 4. Input protocol

```text
INPUT_READY (kernel → server) = notification signal, không mang scancode
INPUT_READ  (client → server) data=[buf_va, max_events]
```

- Kernel **không** đẩy scancode: IRQ handler chỉ `IrqAck` + `Signal` notification
  cho driver user-space. Driver đọc MMIO/PS/2 data port, tự dịch keycode và
  đẩy vào ring buffer của server.
- Server giữ ring 256 event; client đọc theo lô.
- Không expose IRQ trực tiếp cho client.

## 5. Shell tích hợp

- Shell hiện tại dùng UART. Thêm backend chọn `ConsoleBackend::Uart | Framebuffer`.
- Vẽ ký tự 8×16 bitmap nhúng (128 glyph ASCII) vào framebuffer.
- Con trỏ nhấp nháy bằng timer tick; scroll bằng memmove vùng pixel.

## 6. Test plan

- Host test: glyph renderer (bitmap → pixel), ring buffer input, format/stride.
- QEMU: in `FB OK (w=1024 h=768 stride=4096)`; vẽ 4 ô màu và đọc lại pixel
  qua `FrameWrite`/shared frame → `FB PIXEL OK`.
- Input: gửi scancode giả qua virtio queue/PS/2 → `INPUT OK (events=N)`.
- Test tự động framebuffer: QEMU monitor `screendump` ra PPM, so checksum vùng
  4 ô màu (host script) → `FB SCREEN OK (crc=...)`; không cần đọc pixel qua
  guest.
- Harness: marker exact-one, không thay đổi hợp đồng cũ.

## 7. Rủi ro

- GOP framebuffer x86 có thể nằm ngoài 1 GiB identity map (OVMF thường cấp
  BAR > 4 GiB). Phương án: runtime tạo **device Frame cap** từ thông tin
  firmware (GOP/ACPI) và giao cho root qua BootInfo — giống `BlockDevice`;
  user không bao giờ được chọn PA tùy ý. Root `FrameMap` device Frame vào
  VSpace driver với WRITE (không cần identity); không map toàn bộ BAR vào
  kernel.
- ramfb yêu cầu PA liên tục; cấp Frame 2 MiB từ Untyped và ghi qua fw_cfg DMA.
- PS/2 IRQ1 trên x86 đi qua PIC (hiện đã có IRQ object cho timer/UART).
- Chưa có GPU 3D/compositor; MVP chỉ blit 2D.
