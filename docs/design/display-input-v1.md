# Thiết kế framebuffer + input MVP (dự thảo cho vòng 25)

Trạng thái: **draft**, chưa có code. Mục tiêu: đưa ViFastOs từ console text
sang framebuffer + bàn phím, đủ để chạy shell và demo đồ hoạ đơn giản.

## 1. Phần cứng mục tiêu

| Kiến trúc | Framebuffer | Input |
|---|---|---|
| x86_64 | GOP (UEFI) hoặc Bochs VBE (`0x1234:0x1111`) | PS/2 keyboard (IRQ1) |
| AArch64 | ramfb (QEMU `virt`) | PL031? Không; dùng virtio-keyboard |
| RISC-V | ramfb (QEMU `virt`) | virtio-keyboard |

MVP chọn **ramfb + virtio-keyboard** cho RISC-V/AArch64 (QEMU hỗ trợ sẵn),
x86 dùng GOP do OVMF cung cấp. Nếu GOP không có, fallback Bochs VBE.

## 2. Driver model

- Mỗi driver là một **user-space service** giữ cap thiết bị:
  - `BlockDevice`-like: thêm `Framebuffer` và `InputDevice` object type.
  - MMIO window cấp qua `IoMap` (software IOMMU) hoặc `FrameMap` device.
- Kernel chỉ cung cấp:
  - phát hiện thiết bị (ACPI/DT) và tạo cap;
  - IRQ object cho keyboard;
  - shared memory frame cho framebuffer (write-combining map).

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
INPUT_EVENT (kernel → server) data=[scancode, modifiers, pressed, timestamp]
INPUT_READ  (client → server) data=[buf_va, max_events]
```

- Kernel IRQ handler chỉ đẩy scancode vào ring buffer (lock-free) và signal
  notification; server dịch sang keycode.
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
- Input: gửi scancode giả qua virtio queue → `INPUT OK (events=N)`.
- Harness: marker exact-one, không thay đổi hợp đồng cũ.

## 7. Rủi ro

- GOP framebuffer nằm ngoài 1 GiB identity map của x86 → cần map tạm hoặc
  IOMMU window.
- ramfb yêu cầu địa chỉ PA liên tục; cấp bằng Frame lớn (2 MiB) từ Untyped.
- PS/2 IRQ1 trên x86 đi qua PIC (hiện đã có IRQ object cho timer/UART).
- Chưa có GPU 3D/compositor; MVP chỉ blit 2D.
