# 13. VirtIO-MMIO drivers

Bản bare-metal dùng **virtio-mmio phiên bản 2 (modern)** cho RISC-V64 và
AArch64; x86_64 không có VirtIO (dùng ATA PIO + UEFI). Không có PCI, không có
MSI: mọi thiết bị được thăm dò bằng magic `0x74726976` ("virt") tại cửa sổ MMIO
của từng máy.

## 1. Cửa sổ thăm dò

| Arch | Cửa sổ | Bước |
|---|---|---|
| riscv64 (`virt`) | `0x1000_1000 .. 0x1000_9000` | `0x1000` |
| aarch64 (`virt`) | `0x0a00_0000 .. 0x0a00_4000` | `0x200` |

`scan_devices(start, end, stride, visit)` trả `(base, device_id)` cho mọi slot
có magic hợp lệ và version 1/2. Các platform dùng nó để:
- block: `init_range`/`init` (slot đầu tiên id 2),
- net: id 1, gpu: id 16, input: id 18, rng: id 4.

## 2. Trình tự khởi tạo chung (đã trả giá bằng lỗi thật)

1. Ghi `Status = 0` (reset) → `1` (ACK) → `3` (ACK|DRIVER).
2. Thoả thuận feature: chọn word cao (`0x14`), ghi `DriverFeatures` (`0x10`)
   với bit 0 = `VIRTIO_F_VERSION_1`; đọc lại `Status` phải có bit 3
   (`FEATURES_OK`).
3. Cấu hình queue: `QueueSel (0x30)`, `QueueNum (0x38)` (luỹ thừa 2, ≤
   `QueueNumMax` 0x34), `QueueDesc/Driver/Device` tại `0x80/0x90/0xa0` (mỗi
   cái 2 thanh ghi 32-bit), rồi **`QueueReady (0x44) = 1`**.
4. `Status = 15` (DRIVER_OK).
5. Đăng buffer vào avail ring rồi **kick** `QueueNotify (0x50) = index`.
6. Sau mỗi gói đã dùng: đọc `used.idx`, xử lý, **re-post** descriptor và kick
   lại.

Ba lỗi từng làm RX "im lặng" suốt nhiều vòng:
- `QueueReady` phải ghi **1**, không phải index queue (ghi 0 cho queue 0 làm
  RX không bao giờ ready).
- Header virtio-net phải **10 byte** khi không thoả thuận `MRG_RXBUF`; dùng 12
  byte làm QEMU lệch frame 2 byte và slirp bỏ mọi gói.
- Thiếu kick sau khi đăng buffer và thiếu re-post sau khi tiêu thụ.

Với thiết bị hai queue (net, input), cấu hình **cả hai queue trước DRIVER_OK**;
attach queue sau DRIVER_OK từng làm treo QEMU.

## 3. Từng thiết bị

### Block (id 2) — `vifast-virtio`
- Queue 0, 3 descriptor: header (device-đọc), data 512 byte, status (ghi).
- `read_sector`/`write_sector`/`flush`; dùng bởi VFS server qua syscall block.

### Net (id 1)
- RX queue 0: descriptor header 10 byte + buffer 2048 byte; TX queue 1.
- Feature chấp nhận: `MAC | STATUS | VERSION_1` (`accepted=0x10020`).
- API: `net_init_both`, `net_send_arp_to(tpa)`, `net_ping(ip)` (ICMP + checksum
  RFC 1071), `net_send_udp`, `net_recv_udp`, `net_recv_icmp`, `net_recv_dns_a`.
- `net_rx_poll_bounded` đếm `RX_SEEN`; DNS dùng bound lớn hơn vì phải qua
  slirp forward ra host.
- Marker: `VIRTIO NET READY`, `VIRTIO NET PING REPLY`, `VIRTIO NET DNS sent`
  (cứng), `VIRTIO NET DNS A` (best-effort).

### GPU (id 16)
- Control queue 0; lệnh 24-byte header + payload, response type `0x1100`
  (OK) / `0x1101` (display info).
- Chuỗi: `GET_DISPLAY_INFO` → `RESOURCE_CREATE_2D` (B8G8R8X8) →
  `RESOURCE_ATTACH_BACKING` → `SET_SCANOUT` → `TRANSFER_TO_HOST_2D` →
  `RESOURCE_FLUSH`.
- Framebuffer 160×120 (76.8 KB) để vừa headroom kernel; vẽ gradient + font
  5x7 ("ViFastOs") qua `gpu_fill_rect`/`gpu_draw_text`.
- Marker: `VIRTIO GPU READY`, `VIRTIO GPU DRAW OK`.

### Input (id 18) — bàn phím
- Event queue 0: 16 descriptor 8 byte đăng sẵn; status queue 1 attach cho đúng
  chuẩn (không kick).
- `input_poll()` trả `InputEvent { kind, code, value }`; `input_key_ascii()`
  map mã phím Linux sang ASCII (Shift, Enter, Backspace, Space).
- Platform poll trong timer IRQ hart 0, đẩy byte vào `RxQueue` của UART và
  `irq_raise(IRQ_UART)` — shell hiện có đọc như gõ phím.
- Marker: `VIRTIO INPUT READY`, `VIRTIO KEY <code>` (tối đa 16).

### RNG (id 4)
- Queue 0, một buffer 64 byte ghi được; `rng_read(&mut buf)` đăng buffer, kick,
  poll used rồi copy ra ngoài (tối đa 8 byte/lần).
- Platform đọc hai mẫu 8 byte và yêu cầu khác nhau.
- Marker: `VIRTIO RNG OK <hex>`, `VIRTIO RNG READY`.

## 4. Kiểm chứng tự động

| Test | Nội dung |
|---|---|
| `test-qemu.py` | QEMU 3 kiến trúc × 3 lần; marker theo `harness_spec.py` |
| `--gpu-check` | screendump PPM + so pixel gradient/hộp/chữ |
| `--input-check` | HMP `sendkey e` + yêu cầu `VIRTIO KEY 18` |
| `--interactive` | chardev socket gõ `help` qua UART |

Mã nguồn: `crates/vifast-virtio/src/lib.rs` (toàn bộ driver),
`crates/vifast-platform-riscv64/src/platform.rs`,
`crates/vifast-platform-aarch64/src/platform.rs`.
