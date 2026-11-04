"""Phần cứng giả lập: bộ nhớ vật lý, bus MMIO, TLB, thiết bị DMA.

Đây KHÔNG phải kernel. Trong Rust, phần này là phần cứng thật (hoặc QEMU).
Kernel chỉ được nói chuyện với nó qua các giao diện mà phần cứng thật cũng có:
đọc/ghi địa chỉ vật lý, nạp/xoá TLB, nhận ngắt.
"""
from __future__ import annotations

from collections import OrderedDict

RAM_PHYS_BASE = 0x8000_0000          # như máy QEMU virt RISC-V
KERNEL_RESERVED = 0x1_0000           # 64 KB đầu RAM: ảnh kernel, user không bao giờ có cap
KERNEL_CANARY = b"VIFAST-KERNEL-CANARY"
MMIO_SIZE = 0x1000


class BusError(Exception):
    """Truy cập địa chỉ vật lý không tồn tại."""


class PhysMem:
    def __init__(self, ram_size: int) -> None:
        self.ram = bytearray(KERNEL_RESERVED + ram_size)
        self.devices: list = []
        self.ram[0:len(KERNEL_CANARY)] = KERNEL_CANARY

    def _ram_off(self, pa: int, n: int) -> int | None:
        off = pa - RAM_PHYS_BASE
        if 0 <= off and off + n <= len(self.ram):
            return off
        return None

    def _device(self, pa: int, n: int):
        for dev in self.devices:
            if dev.mmio_base <= pa and pa + n <= dev.mmio_base + MMIO_SIZE:
                return dev
        return None

    def read(self, pa: int, n: int) -> bytes:
        off = self._ram_off(pa, n)
        if off is not None:
            return bytes(self.ram[off:off + n])
        dev = self._device(pa, n)
        if dev is not None:
            return dev.mmio_read(pa - dev.mmio_base, n)
        raise BusError(hex(pa))

    def write(self, pa: int, data: bytes) -> None:
        off = self._ram_off(pa, len(data))
        if off is not None:
            self.ram[off:off + len(data)] = data
            return
        dev = self._device(pa, len(data))
        if dev is not None:
            dev.mmio_write(pa - dev.mmio_base, bytes(data))
            return
        raise BusError(hex(pa))

    def read_u64(self, pa: int) -> int:
        return int.from_bytes(self.read(pa, 8), "little")

    def write_u64(self, pa: int, value: int) -> None:
        self.write(pa, value.to_bytes(8, "little"))

    def kernel_canary_intact(self) -> bool:
        return bytes(self.ram[0:len(KERNEL_CANARY)]) == KERNEL_CANARY


class Tlb:
    """TLB của một hart, gắn tag ASID. Thay thế FIFO khi đầy."""

    def __init__(self, capacity: int = 64) -> None:
        self.capacity = capacity
        self.entries: OrderedDict = OrderedDict()
        self.hits = 0
        self.misses = 0

    def get(self, asid: int, vpn: int):
        pte = self.entries.get((asid, vpn))
        if pte is None:
            self.misses += 1
        else:
            self.hits += 1
        return pte

    def put(self, asid: int, vpn: int, pte: int) -> None:
        if len(self.entries) >= self.capacity:
            self.entries.popitem(last=False)
        self.entries[(asid, vpn)] = pte

    def flush(self, asid: int | None = None, vpn: int | None = None) -> None:
        """sfence.vma: None = tất cả."""
        if asid is None:
            self.entries.clear()
            return
        for key in [k for k in self.entries if k[0] == asid and (vpn is None or k[1] == vpn)]:
            del self.entries[key]


class DmaSensor:
    """Thiết bị cảm biến/camera có DMA.

    Thanh ghi (mỗi cái 8 byte): 0x00 DMA_ADDR, 0x08 DMA_LEN, 0x10 CTRL (ghi 1 = bắt đầu),
    0x18 STATUS (0 rảnh, 1 bận, 2 xong, 3 lỗi). Xong thì phát ngắt.
    DMA_ADDR là địa chỉ thiết bị: có IOMMU thì là IOVA, không có thì là địa chỉ vật lý.
    """

    IDLE, BUSY, DONE, ERROR = 0, 1, 2, 3

    def __init__(self, name: str, mmio_base: int, irq: int, latency: int = 3) -> None:
        self.name = name
        self.mmio_base = mmio_base
        self.irq = irq
        self.latency = latency
        self.regs = [0, 0, 0, self.IDLE]
        self.done_at: int | None = None
        self.counter = 0
        self.kernel = None
        self.iospace = None

    def mmio_read(self, off: int, n: int) -> bytes:
        raw = b"".join(r.to_bytes(8, "little") for r in self.regs)
        return raw[off:off + n]

    def mmio_write(self, off: int, data: bytes) -> None:
        if off % 8 or len(data) != 8 or off >= 32:
            return                                  # truy cập sai: phần cứng bỏ qua
        reg = off // 8
        value = int.from_bytes(data, "little")
        if reg == 2:
            if value == 1 and self.regs[3] != self.BUSY:
                self.regs[3] = self.BUSY
                self.done_at = self.kernel.ticks + self.latency
        elif reg in (0, 1):
            self.regs[reg] = value

    @property
    def busy(self) -> bool:
        return self.done_at is not None

    def tick(self, kernel) -> None:
        if self.done_at is None or kernel.ticks < self.done_at:
            return
        self.done_at = None
        self.counter += 1
        length = min(self.regs[1], 64)
        payload = f"SENSOR-{self.counter}".encode()[:length].ljust(length, b"\0")
        ok = kernel.dma_write(self, self.regs[0], payload)
        self.regs[3] = self.DONE if ok else self.ERROR
        kernel.deliver_irq(self.irq)


DEVICE_KINDS = {"dma_sensor": DmaSensor}
