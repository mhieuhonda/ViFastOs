//! Phần cứng giả lập: RAM vật lý, TLB, thiết bị DMA, và `Hw` cài đặt `Platform`.
//!
//! Đây KHÔNG phải kernel. Trên máy thật, phần này là phần cứng hoặc QEMU.

use std::collections::VecDeque;

use vifast_kernel::{ObjRef, Platform};

/// Giống máy QEMU `virt` RISC-V.
pub const RAM_PHYS_BASE: u64 = 0x8000_0000;
/// 64 KB đầu RAM: ảnh kernel. User không bao giờ có cap tới vùng này.
pub const KERNEL_RESERVED: u64 = 0x1_0000;
pub const KERNEL_CANARY: &[u8] = b"VIFAST-KERNEL-CANARY";
pub const MMIO_SIZE: u64 = 0x1000;

pub struct PhysMem {
    pub ram: Vec<u8>,
}

impl PhysMem {
    pub fn new(ram_size: u64) -> PhysMem {
        let mut ram = vec![0u8; (KERNEL_RESERVED + ram_size) as usize];
        ram[..KERNEL_CANARY.len()].copy_from_slice(KERNEL_CANARY);
        PhysMem { ram }
    }

    fn offset(&self, pa: u64, n: usize) -> Option<usize> {
        let off = usize::try_from(pa.checked_sub(RAM_PHYS_BASE)?).ok()?;
        if off.checked_add(n)? <= self.ram.len() {
            Some(off)
        } else {
            None
        }
    }

    pub fn is_ram(&self, pa: u64, n: usize) -> bool {
        self.offset(pa, n).is_some()
    }

    pub fn read(&self, pa: u64, buf: &mut [u8]) -> bool {
        match self.offset(pa, buf.len()) {
            Some(off) => {
                buf.copy_from_slice(&self.ram[off..off + buf.len()]);
                true
            }
            None => false,
        }
    }

    pub fn write(&mut self, pa: u64, data: &[u8]) -> bool {
        match self.offset(pa, data.len()) {
            Some(off) => {
                self.ram[off..off + data.len()].copy_from_slice(data);
                true
            }
            None => false,
        }
    }

    pub fn read_u64(&self, pa: u64) -> u64 {
        let mut b = [0u8; 8];
        if self.read(pa, &mut b) {
            u64::from_le_bytes(b)
        } else {
            0
        }
    }

    pub fn kernel_canary_intact(&self) -> bool {
        &self.ram[..KERNEL_CANARY.len()] == KERNEL_CANARY
    }
}

/// TLB của một hart, gắn tag ASID. Thay thế FIFO khi đầy.
pub struct Tlb {
    capacity: usize,
    entries: VecDeque<(u32, u64, u64)>,
    pub hits: u64,
    pub misses: u64,
}

impl Tlb {
    pub fn new(capacity: usize) -> Tlb {
        Tlb { capacity, entries: VecDeque::new(), hits: 0, misses: 0 }
    }

    pub fn get(&mut self, asid: u32, vpn: u64) -> Option<u64> {
        let found = self.entries.iter().find(|e| e.0 == asid && e.1 == vpn).map(|e| e.2);
        if found.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        found
    }

    pub fn put(&mut self, asid: u32, vpn: u64, pte: u64) {
        self.entries.retain(|e| !(e.0 == asid && e.1 == vpn));
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((asid, vpn, pte));
    }

    /// `sfence.vma`: `None` = tất cả.
    pub fn flush(&mut self, asid: Option<u32>, vpn: Option<u64>) {
        match asid {
            None => self.entries.clear(),
            Some(a) => self.entries.retain(|e| !(e.0 == a && vpn.map_or(true, |v| e.1 == v))),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub tick: u64,
    pub source: String,
    pub text: String,
}

/// Nền tảng giả lập mà kernel chạy trên đó.
pub struct Hw {
    pub mem: PhysMem,
    pub tlbs: Vec<Tlb>,
    pub log: Vec<LogLine>,
    pub verbose: bool,
}

impl Platform for Hw {
    fn phys_read(&self, pa: u64, buf: &mut [u8]) -> bool {
        self.mem.read(pa, buf)
    }

    fn phys_write(&mut self, pa: u64, data: &[u8]) -> bool {
        self.mem.write(pa, data)
    }

    fn tlb_flush(&mut self, hart: usize, asid: Option<u32>, vpn: Option<u64>) {
        if let Some(t) = self.tlbs.get_mut(hart) {
            t.flush(asid, vpn);
        }
    }

    fn log(&mut self, tick: u64, source: &str, text: &str) {
        if self.verbose {
            println!("[{:6}] {:<14} {}", tick, source, text);
        }
        self.log.push(LogLine { tick, source: source.to_string(), text: text.to_string() });
    }
}

/// Thiết bị cảm biến/camera có DMA.
///
/// Thanh ghi (mỗi cái 8 byte): 0x00 DMA_ADDR, 0x08 DMA_LEN, 0x10 CTRL (ghi 1 = bắt đầu),
/// 0x18 STATUS (0 rảnh, 1 bận, 2 xong, 3 lỗi). Xong thì phát ngắt.
/// DMA_ADDR là địa chỉ thiết bị: có IOMMU thì là IOVA, không có thì là địa chỉ vật lý.
pub struct DmaSensor {
    pub name: String,
    pub mmio_base: u64,
    pub irq: u32,
    pub latency: u64,
    pub regs: [u64; 4],
    pub done_at: Option<u64>,
    pub counter: u64,
    pub iospace: Option<ObjRef>,
}

impl DmaSensor {
    pub const IDLE: u64 = 0;
    pub const BUSY: u64 = 1;
    pub const DONE: u64 = 2;
    pub const ERROR: u64 = 3;

    pub fn new(name: &str, mmio_base: u64, irq: u32) -> DmaSensor {
        DmaSensor {
            name: name.to_string(),
            mmio_base,
            irq,
            latency: 3,
            regs: [0, 0, 0, Self::IDLE],
            done_at: None,
            counter: 0,
            iospace: None,
        }
    }

    pub fn contains(&self, pa: u64, n: usize) -> bool {
        pa >= self.mmio_base && pa + n as u64 <= self.mmio_base + MMIO_SIZE
    }

    pub fn busy(&self) -> bool {
        self.done_at.is_some()
    }

    pub fn mmio_read(&self, off: u64, n: usize) -> Vec<u8> {
        let raw: Vec<u8> = self.regs.iter().flat_map(|r| r.to_le_bytes()).collect();
        let start = (off as usize).min(raw.len());
        let end = (start + n).min(raw.len());
        let mut out = raw[start..end].to_vec();
        out.resize(n, 0);
        out
    }

    /// Truy cập sai kích thước/căn lề thì phần cứng bỏ qua.
    pub fn mmio_write(&mut self, off: u64, data: &[u8], now: u64) {
        if off % 8 != 0 || data.len() != 8 || off >= 32 {
            return;
        }
        let mut b = [0u8; 8];
        b.copy_from_slice(data);
        let value = u64::from_le_bytes(b);
        match off / 8 {
            2 => {
                if value == 1 && self.regs[3] != Self::BUSY {
                    self.regs[3] = Self::BUSY;
                    self.done_at = Some(now + self.latency);
                }
            }
            r @ (0 | 1) => self.regs[r as usize] = value,
            _ => {}
        }
    }
}
