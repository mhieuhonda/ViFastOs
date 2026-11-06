//! Minimal VirtIO-MMIO block driver used by the AArch64 QEMU port.
//!
//! One split virtqueue and one 512-byte request are enough for the reference
//! filesystem.  Queue memory is identity-mapped static memory.

#![no_std]

use core::sync::atomic::{fence, Ordering};

pub const SECTOR_SIZE: usize = 512;
/// VirtIO device IDs (subset relevant to QEMU virt).
pub const DEVICE_NET: u32 = 1;
pub const DEVICE_BLOCK: u32 = 2;
const QUEUE_SIZE: usize = 8;
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Descriptor {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C, align(4))]
struct Available {
    flags: u16,
    idx: u16,
    ring: [u16; QUEUE_SIZE],
    event: u16,
}

#[repr(C, align(4))]
#[derive(Clone, Copy)]
struct UsedElem {
    id: u32,
    len: u32,
}

#[repr(C, align(4))]
struct Used {
    flags: u16,
    idx: u16,
    ring: [UsedElem; QUEUE_SIZE],
    event: u16,
}

#[repr(C, align(4096))]
struct Queue {
    desc: [Descriptor; QUEUE_SIZE],
    avail: Available,
    used: Used,
    header: [u32; 4],
    data: [u8; SECTOR_SIZE],
    status: u8,
    pad: [u8; 7],
}

const EMPTY_DESC: Descriptor = Descriptor {
    addr: 0,
    len: 0,
    flags: 0,
    next: 0,
};

static mut QUEUE: Queue = Queue {
    desc: [EMPTY_DESC; QUEUE_SIZE],
    avail: Available {
        flags: 0,
        idx: 0,
        ring: [0; QUEUE_SIZE],
        event: 0,
    },
    used: Used {
        flags: 0,
        idx: 0,
        ring: [UsedElem { id: 0, len: 0 }; QUEUE_SIZE],
        event: 0,
    },
    header: [0; 4],
    data: [0; SECTOR_SIZE],
    status: 0,
    pad: [0; 7],
};

/// Maximum Ethernet frame QEMU user-net delivers (1514) plus headroom.
pub const NET_BUF_SIZE: usize = 2048;
/// VirtIO-net header: flags(1) + gso_type(1) + hdr_len(2) + gso_size(2) +
/// csum_start(2) + csum_offset(2) = 10 byte. CHỈ thêm num_buffers(2) khi
/// VIRTIO_NET_F_MRG_RXBUF được thoả thuận — ta không bật, nên 12 byte sẽ
/// làm QEMU lệch frame 2 byte và slirp bỏ qua mọi gói (vòng 63).
pub const NET_HDR_SIZE: usize = 10;

/// Dedicated split-queue for VirtIO-net (vòng 45). Separate static from the
/// block `QUEUE` so net bring-up never disturbs the disk path; both live in
/// `.bss` after `KERNEL_STORAGE`/BKL (nm-confirmed 4 KiB block QUEUE at
/// 0x401ee000 with room to spare), not inside `Kernel`, so the 8-byte
/// AArch64 static gap is untouched.
#[repr(C, align(4096))]
struct NetQueue {
    desc: [Descriptor; QUEUE_SIZE],
    avail: Available,
    used: Used,
    hdr: [u8; NET_HDR_SIZE],
    buf: [u8; NET_BUF_SIZE],
}

static mut NET_QUEUE: NetQueue = NetQueue {
    desc: [EMPTY_DESC; QUEUE_SIZE],
    avail: Available {
        flags: 0,
        idx: 0,
        ring: [0; QUEUE_SIZE],
        event: 0,
    },
    used: Used {
        flags: 0,
        idx: 0,
        ring: [UsedElem { id: 0, len: 0 }; QUEUE_SIZE],
        event: 0,
    },
    hdr: [0; NET_HDR_SIZE],
    buf: [0; NET_BUF_SIZE],
};

/// Address and size of the dedicated net queue (for platform logging and
/// future TX/RX wiring). Read-only w.r.t. device state.
pub fn net_queue_info() -> (u64, usize) {
    let base = core::ptr::addr_of!(NET_QUEUE) as u64;
    (base, core::mem::size_of::<NetQueue>())
}

/// Second dedicated split-queue for VirtIO-net RX (vòng 48). Same layout as
/// the TX `NetQueue`; separate static so RX buffer posting never disturbs
/// the TX path or the block `QUEUE`. Also in `.bss`, outside `Kernel`
/// (AArch64 8-byte static gap untouched).
static mut NET_RX_QUEUE: NetQueue = NetQueue {
    desc: [EMPTY_DESC; QUEUE_SIZE],
    avail: Available {
        flags: 0,
        idx: 0,
        ring: [0; QUEUE_SIZE],
        event: 0,
    },
    used: Used {
        flags: 0,
        idx: 0,
        ring: [UsedElem { id: 0, len: 0 }; QUEUE_SIZE],
        event: 0,
    },
    hdr: [0; NET_HDR_SIZE],
    buf: [0; NET_BUF_SIZE],
};

pub fn net_rx_queue_info() -> (u64, usize) {
    let base = core::ptr::addr_of!(NET_RX_QUEUE) as u64;
    (base, core::mem::size_of::<NetQueue>())
}

/// Attach RX queue (index 0) to the dedicated `NET_RX_QUEUE` static and post
/// one receive buffer (hdr + 2048B buf, WRITE descriptors). Call once after
/// `net_init_tx` (device already DRIVER_OK). Mirrors the TX attach path.
pub fn net_init_rx() -> bool {
    fn write_at(base: u64, offset: u64, value: u32) {
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
    }
    fn write64_at(base: u64, offset: u64, value: u64) {
        write_at(base, offset, value as u32);
        write_at(base, offset + 4, (value >> 32) as u32);
    }
    let base = unsafe {
        if !NET_READY || NET_BASE == 0 {
            return false;
        }
        NET_BASE
    };
    let queue = core::ptr::addr_of_mut!(NET_RX_QUEUE);
    unsafe {
        let q = &mut *queue;
        q.avail.idx = 0;
        q.used.idx = 0;
        q.hdr = [0; NET_HDR_SIZE];
        let qb = q as *const NetQueue as u64;
        q.desc[0] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, hdr) as u64,
            len: NET_HDR_SIZE as u32,
            flags: DESC_WRITE | DESC_NEXT,
            next: 1,
        };
        q.desc[1] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, buf) as u64,
            len: NET_BUF_SIZE as u32,
            flags: DESC_WRITE,
            next: 0,
        };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        q.avail.ring[0] = 0;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        core::ptr::write_volatile(&mut q.avail.idx, 1);
    }
    write_at(base, 0x30, 0);
    write_at(base, 0x38, QUEUE_SIZE as u32);
    let qbase = queue as u64;
    write64_at(
        base,
        0x80,
        qbase + core::mem::offset_of!(NetQueue, desc) as u64,
    );
    write64_at(
        base,
        0x90,
        qbase + core::mem::offset_of!(NetQueue, avail) as u64,
    );
    write64_at(
        base,
        0xa0,
        qbase + core::mem::offset_of!(NetQueue, used) as u64,
    );
    write_at(base, 0x44, 0);
    true
}

/// Poll the RX used ring for one packet; if it is an ARP reply, return the
/// sender MAC (bytes 22..28 of the Ethernet+ARP frame, after the 12B
/// virtio-net header). Bounded spin, read-only parse. Returns None on
/// timeout or non-ARP-reply packet.
pub fn net_recv_arp() -> Option<[u8; 6]> {
    // Slirp có thể gửi probe ARP trước reply; bỏ qua gói không khớp và chờ
    // gói kế tiếp (tối đa 4 gói) — vòng 63.
    for _ in 0..4 {
        if !net_rx_poll() {
            return None;
        }
        let queue = core::ptr::addr_of_mut!(NET_RX_QUEUE);
        let mac = unsafe {
            let q = &mut *queue;
            // RX: header 10B vào `hdr`, frame Ethernet vào `buf[0..]`.
            let ethertype = u16::from_be_bytes([q.buf[12], q.buf[13]]);
            let oper = u16::from_be_bytes([q.buf[20], q.buf[21]]);
            if ethertype == 0x0806 && oper == 2 {
                let mut mac = [0u8; 6];
                mac.copy_from_slice(&q.buf[22..28]);
                Some(mac)
            } else {
                None
            }
        };
        if mac.is_some() {
            net_rx_post();
            return mac;
        }
        net_rx_post();
    }
    None
}

/// Đẩy một buffer RX mới vào avail ring và kick queue 0 (vòng 63). VirtIO
/// yêu cầu driver thông báo device sau khi thêm buffer; thiếu kick thì device
/// không bao giờ ghi gói vào RX (nguyên nhân "NO REPLY" trước đó).
fn net_rx_post() {
    let base = unsafe { NET_BASE };
    if base == 0 {
        return;
    }
    let queue = core::ptr::addr_of_mut!(NET_RX_QUEUE);
    unsafe {
        let q = &mut *queue;
        let slot = (q.avail.idx as usize) % QUEUE_SIZE;
        q.avail.ring[slot] = 0;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let next = q.avail.idx.wrapping_add(1);
        core::ptr::write_volatile(&mut q.avail.idx, next);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
    unsafe { core::ptr::write_volatile((base + 0x50) as *mut u32, 0) };
}

/// Chờ một gói RX mới (used.idx tiến), bound tuỳ ý.
fn net_rx_poll_bounded(bound: u64) -> bool {
    let queue = core::ptr::addr_of_mut!(NET_RX_QUEUE);
    unsafe {
        let q = &mut *queue;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let mut spins = 0u64;
        while core::ptr::read_volatile(&q.used.idx) == RX_SEEN && spins < bound {
            core::hint::spin_loop();
            spins += 1;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) == RX_SEEN {
            return false;
        }
        RX_SEEN = core::ptr::read_volatile(&q.used.idx);
        true
    }
}

/// Mặc định 2M spins cho gói cục bộ (ARP/ICMP).
fn net_rx_poll() -> bool {
    net_rx_poll_bounded(2_000_000)
}

/// Cấp lại buffer RX sau khi tiêu thụ một gói (vòng 63).
pub fn net_rx_rearm() {
    net_rx_post();
}

/// Gửi một Ethernet frame qua TX queue (kick + poll used, vòng 64).
/// Tách từ net_ping để UDP dùng lại; frame tối đa 60 byte (đã pad).
fn net_tx_frame(frame: &[u8]) -> Option<u32> {
    let base = unsafe {
        if !NET_READY || NET_BASE == 0 {
            return None;
        }
        NET_BASE
    };
    fn write_at(base: u64, offset: u64, value: u32) {
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
    }
    let queue = core::ptr::addr_of_mut!(NET_QUEUE);
    unsafe {
        let q = &mut *queue;
        q.hdr = [0; NET_HDR_SIZE];
        q.buf[..frame.len()].copy_from_slice(frame);
        let qb = q as *const NetQueue as u64;
        q.desc[0] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, hdr) as u64,
            len: NET_HDR_SIZE as u32,
            flags: DESC_NEXT,
            next: 1,
        };
        q.desc[1] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, buf) as u64,
            len: frame.len() as u32,
            flags: 0,
            next: 0,
        };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let next_avail = q.avail.idx.wrapping_add(1);
        q.avail.ring[(q.avail.idx as usize) % QUEUE_SIZE] = 0;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        core::ptr::write_volatile(&mut q.avail.idx, next_avail);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        write_at(base, 0x50, 1);
        let target = core::ptr::read_volatile(&q.used.idx).wrapping_add(1);
        let mut spins = 0u64;
        while core::ptr::read_volatile(&q.used.idx) != target && spins < 2_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) != target {
            return None;
        }
        let done = q.used.idx.wrapping_sub(1) as usize % QUEUE_SIZE;
        let elem = core::ptr::read_volatile(&q.used.ring[done]);
        Some(elem.len)
    }
}

/// Gửi UDP datagram tới `dst_ip:dst_port` (vòng 64). Ethernet dst dùng MAC
/// ảo slirp của đích (52:55:0a:00:02:xx). Payload tối đa 26 byte để vừa khung
/// 60B; đủ cho một DNS query ngắn.
pub fn net_send_udp(dst_ip: [u8; 4], dst_port: u16, payload: &[u8]) -> Option<u32> {
    let mac = unsafe { net_mac(NET_BASE)? };
    const SRC_IP: [u8; 4] = [10, 0, 2, 15];
    let udp_len = 8 + payload.len();
    let ip_len = 20 + udp_len;
    let total = 14 + ip_len;
    if total > NET_BUF_SIZE {
        return None;
    }
    let mut frame = [0u8; NET_BUF_SIZE];
    frame[0..6].copy_from_slice(&[0x52, 0x55, 0x0a, 0x00, 0x02, dst_ip[3]]);
    frame[6..12].copy_from_slice(&mac);
    frame[12..14].copy_from_slice(&[0x08, 0x00]);
    let ip = &mut frame[14..34];
    ip[0] = 0x45;
    ip[1] = 0;
    ip[2..4].copy_from_slice(&(ip_len as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&0x4321u16.to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 17; // UDP
    ip[10..12].copy_from_slice(&[0, 0]);
    ip[12..16].copy_from_slice(&SRC_IP);
    ip[16..20].copy_from_slice(&dst_ip);
    let ip_sum = checksum(&ip[..20]);
    ip[10..12].copy_from_slice(&ip_sum.to_be_bytes());
    let udp = &mut frame[34..34 + udp_len];
    udp[0..2].copy_from_slice(&49152u16.to_be_bytes());
    udp[2..4].copy_from_slice(&dst_port.to_be_bytes());
    udp[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    udp[6..8].copy_from_slice(&[0, 0]); // checksum 0 = optional trong IPv4
    udp[8..].copy_from_slice(payload);
    net_tx_frame(&frame[..total])
}

/// Chờ một UDP datagram từ `src_ip:src_port` (vòng 64). Trả về độ dài
/// payload nếu khớp (bỏ qua gói khác, tối đa 4 gói).
pub fn net_recv_udp(src_ip: [u8; 4], src_port: u16) -> Option<usize> {
    for _ in 0..4 {
        // DNS đi qua slirp forward ra host nên chậm hơn ICMP nội bộ; dùng
        // bound lớn hơn (8M spins) để không bỏ lỡ reply.
        if !net_rx_poll_bounded(8_000_000) {
            return None;
        }
        let queue = core::ptr::addr_of_mut!(NET_RX_QUEUE);
        let found = unsafe {
            let q = &mut *queue;
            let ethertype = u16::from_be_bytes([q.buf[12], q.buf[13]]);
            let ok = ethertype == 0x0800
                && q.buf[14 + 9] == 17
                && q.buf[14 + 12..14 + 16] == src_ip
                && u16::from_be_bytes([q.buf[34], q.buf[35]]) == src_port;
            if ok {
                Some(u16::from_be_bytes([q.buf[38], q.buf[39]]) as usize)
            } else {
                None
            }
        };
        if found.is_some() {
            net_rx_post();
            return found;
        }
        net_rx_post();
    }
    None
}

/// Parse câu trả lời DNS và lấy bản ghi A đầu tiên (vòng 68).
/// Bỏ qua header 12 byte và question, duyệt answer section; hỗ trợ tên nén
/// (pointer 0xC0) và bỏ qua bản ghi không phải A. Trả về IPv4 4 byte.
fn parse_dns_a(payload: &[u8]) -> Option<[u8; 4]> {
    if payload.len() < 12 {
        return None;
    }
    let qdcount = u16::from_be_bytes([payload[4], payload[5]]) as usize;
    let ancount = u16::from_be_bytes([payload[6], payload[7]]) as usize;
    let mut pos = 12usize;
    // Bỏ qua các question: QNAME (label stream) + QTYPE + QCLASS.
    let mut q = 0;
    while q < qdcount {
        loop {
            if pos >= payload.len() {
                return None;
            }
            let len = payload[pos];
            if len == 0 {
                pos += 1;
                break;
            }
            if len & 0xC0 == 0xC0 {
                pos += 2;
                break;
            }
            pos += 1 + len as usize;
        }
        pos += 4;
        q += 1;
    }
    let mut a = 0;
    while a < ancount {
        // NAME: label stream hoặc pointer nén.
        loop {
            if pos >= payload.len() {
                return None;
            }
            let len = payload[pos];
            if len == 0 {
                pos += 1;
                break;
            }
            if len & 0xC0 == 0xC0 {
                pos += 2;
                break;
            }
            pos += 1 + len as usize;
        }
        if pos + 10 > payload.len() {
            return None;
        }
        let rtype = u16::from_be_bytes([payload[pos], payload[pos + 1]]);
        let rdlen = u16::from_be_bytes([payload[pos + 8], payload[pos + 9]]) as usize;
        pos += 10;
        if pos + rdlen > payload.len() {
            return None;
        }
        if rtype == 1 && rdlen == 4 {
            return Some([
                payload[pos],
                payload[pos + 1],
                payload[pos + 2],
                payload[pos + 3],
            ]);
        }
        pos += rdlen;
        a += 1;
    }
    None
}

/// Chờ DNS response từ `src_ip:53` và trả về bản ghi A đầu tiên (vòng 68).
/// Kết hợp poll + parse để không tiêu thụ gói trước khi kịp đọc payload.
pub fn net_recv_dns_a(src_ip: [u8; 4]) -> Option<[u8; 4]> {
    for _ in 0..4 {
        if !net_rx_poll_bounded(8_000_000) {
            return None;
        }
        let result = unsafe {
            let q = &mut *core::ptr::addr_of_mut!(NET_RX_QUEUE);
            let ethertype = u16::from_be_bytes([q.buf[12], q.buf[13]]);
            let ok = ethertype == 0x0800
                && q.buf[14 + 9] == 17
                && q.buf[14 + 12..14 + 16] == src_ip
                && u16::from_be_bytes([q.buf[34], q.buf[35]]) == 53;
            if ok {
                let udp_len = u16::from_be_bytes([q.buf[38], q.buf[39]]) as usize;
                let payload_len = udp_len.saturating_sub(8);
                let start = 14 + 20 + 8;
                let end = (start + payload_len).min(q.buf.len());
                if end > start {
                    parse_dns_a(&q.buf[start..end])
                } else {
                    None
                }
            } else {
                None
            }
        };
        if result.is_some() {
            net_rx_post();
            return result;
        }
        net_rx_post();
    }
    None
}

/// Poll RX chờ ICMP echo reply (vòng 63). Trả về IP nguồn nếu thấy reply.
/// Bounded 2M spins như các poll khác; parse từ buf[0] (frame Ethernet).
pub fn net_recv_icmp() -> Option<[u8; 4]> {
    for _ in 0..4 {
        if !net_rx_poll() {
            return None;
        }
        let queue = core::ptr::addr_of_mut!(NET_RX_QUEUE);
        let src = unsafe {
            let q = &mut *queue;
            let ethertype = u16::from_be_bytes([q.buf[12], q.buf[13]]);
            if ethertype == 0x0800 && q.buf[14 + 9] == 1 && q.buf[14 + 20] == 0 {
                let mut src = [0u8; 4];
                src.copy_from_slice(&q.buf[14 + 12..14 + 16]);
                Some(src)
            } else {
                None
            }
        };
        if src.is_some() {
            net_rx_post();
            return src;
        }
        net_rx_post();
    }
    None
}

/// Snapshot RX khi có gói: trả về (used_len, ethertype, 6 byte đầu frame).
/// Dùng để debug vòng 50: biết có gì đến dù không phải ARP reply.
pub fn net_recv_peek() -> Option<(u32, u16, [u8; 6])> {
    let queue = core::ptr::addr_of_mut!(NET_RX_QUEUE);
    unsafe {
        let q = &mut *queue;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) == 0 {
            return None;
        }
        // Phần tử vừa hoàn tất là ring[(used.idx - 1) % QUEUE_SIZE], không
        // phải ring[0] — quan trọng khi gửi nhiều gói (vòng 63).
        let done = q.used.idx.wrapping_sub(1) as usize % QUEUE_SIZE;
        let elem = core::ptr::read_volatile(&q.used.ring[done]);
        let ethertype = u16::from_be_bytes([q.buf[12], q.buf[13]]);
        let mut head = [0u8; 6];
        head.copy_from_slice(&q.buf[0..6]);
        Some((elem.len, ethertype, head))
    }
}

/// One's-complement checksum (RFC 1071) over a byte slice.
fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[i], bytes[i + 1]]) as u32;
        i += 2;
    }
    if i < bytes.len() {
        sum += (bytes[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Build an ICMP echo request to `dst_ip` in `NET_QUEUE.buf` and transmit it
/// (vòng 63). Không cần ARP: slirp xử lý gói IP trực tiếp. Ethernet dst dùng
/// broadcast vì chưa biết MAC gateway. Trả về Some(len) khi used ring chạy.
pub fn net_ping(dst_ip: [u8; 4]) -> Option<u32> {
    let (base, mac) = unsafe {
        if !NET_READY || NET_BASE == 0 {
            return None;
        }
        (NET_BASE, net_mac(NET_BASE)?)
    };
    fn write_at(base: u64, offset: u64, value: u32) {
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
    }
    const SRC: [u8; 4] = [10, 0, 2, 15];
    const PAYLOAD: [u8; 4] = *b"VIFA";
    let icmp_len = 8 + PAYLOAD.len();
    let ip_len = 20 + icmp_len;
    let mut frame = [0u8; 60];
    // Slirp nhận gói IP khi Ethernet dst là MAC ảo của nó (52:55:0a:00:02:02
    // cho 10.0.2.2); broadcast bị bỏ qua cho gói IP (thử nghiệm vòng 63).
    frame[0..6].copy_from_slice(&[0x52, 0x55, 0x0a, 0x00, 0x02, 0x02]);
    frame[6..12].copy_from_slice(&mac);
    frame[12..14].copy_from_slice(&[0x08, 0x00]);
    // IPv4 header at frame[14..34].
    let ip = &mut frame[14..34];
    ip[0] = 0x45;
    ip[1] = 0;
    ip[2..4].copy_from_slice(&(ip_len as u16).to_be_bytes());
    ip[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
    ip[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip[8] = 64;
    ip[9] = 1; // ICMP
    ip[10..12].copy_from_slice(&[0, 0]);
    ip[12..16].copy_from_slice(&SRC);
    ip[16..20].copy_from_slice(&dst_ip);
    let ip_sum = checksum(&ip[..20]);
    ip[10..12].copy_from_slice(&ip_sum.to_be_bytes());
    // ICMP echo request at frame[34..].
    let icmp = &mut frame[34..34 + icmp_len];
    icmp[0] = 8;
    icmp[1] = 0;
    icmp[2..4].copy_from_slice(&[0, 0]);
    icmp[4..6].copy_from_slice(&1u16.to_be_bytes());
    icmp[6..8].copy_from_slice(&1u16.to_be_bytes());
    icmp[8..].copy_from_slice(&PAYLOAD);
    let icmp_sum = checksum(&icmp[..icmp_len]);
    icmp[2..4].copy_from_slice(&icmp_sum.to_be_bytes());
    let queue = core::ptr::addr_of_mut!(NET_QUEUE);
    unsafe {
        let q = &mut *queue;
        q.hdr = [0; NET_HDR_SIZE];
        q.buf[..60].copy_from_slice(&frame);
        let qb = q as *const NetQueue as u64;
        q.desc[0] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, hdr) as u64,
            len: NET_HDR_SIZE as u32,
            flags: DESC_NEXT,
            next: 1,
        };
        q.desc[1] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, buf) as u64,
            len: frame.len() as u32,
            flags: 0,
            next: 0,
        };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let next_avail = q.avail.idx.wrapping_add(1);
        q.avail.ring[(q.avail.idx as usize) % QUEUE_SIZE] = 0;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        core::ptr::write_volatile(&mut q.avail.idx, next_avail);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        write_at(base, 0x50, 1);
        let target = core::ptr::read_volatile(&q.used.idx).wrapping_add(1);
        let mut spins = 0u64;
        while core::ptr::read_volatile(&q.used.idx) != target && spins < 2_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) != target {
            return None;
        }
        let done = q.used.idx.wrapping_sub(1) as usize % QUEUE_SIZE;
        let elem = core::ptr::read_volatile(&q.used.ring[done]);
        Some(elem.len)
    }
}

static mut NET_READY: bool = false;
/// Số phần tử used ring đã thấy trên RX (dùng để biết gói mới).
static mut RX_SEEN: u16 = 0;
static mut NET_BASE: u64 = 0;

/// Negotiate a VirtIO-net device at `base`: reset, ACK+DRIVER, read features,
/// accept the MAC/STATUS/VERSION_1 subset we need for a 1-queue-pair MVP,
/// attach queue 1 (TX) to the dedicated `NET_QUEUE` static, then DRIVER_OK.
/// Queue 0 (RX) is left for the next round. Returns the accepted feature word.
/// Mirrors `init_at` (block) but never touches the block queue or globals.
/// Negotiate a VirtIO-net device at `base` and attach BOTH queues before
/// DRIVER_OK (vòng 49). Vòng 48 showed attaching RX after the TX DRIVER_OK
/// kills the guest under QEMU, so queue 0 (RX, with one posted WRITE buffer)
/// and queue 1 (TX) are wired while the device is still in FEATURES_OK.
/// Returns the accepted feature word. Never touches the block queue.
pub fn net_init_both(base: u64) -> Option<u64> {
    fn read_at(base: u64, offset: u64) -> u32 {
        unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
    }
    fn write_at(base: u64, offset: u64, value: u32) {
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
    }
    fn attach(base: u64, index: u32, qbase: u64) {
        fn write_at(base: u64, offset: u64, value: u32) {
            unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
        }
        fn write64_at(base: u64, offset: u64, value: u64) {
            write_at(base, offset, value as u32);
            write_at(base, offset + 4, (value >> 32) as u32);
        }
        write_at(base, 0x30, index);
        write_at(base, 0x38, QUEUE_SIZE as u32);
        write64_at(
            base,
            0x80,
            qbase + core::mem::offset_of!(NetQueue, desc) as u64,
        );
        write64_at(
            base,
            0x90,
            qbase + core::mem::offset_of!(NetQueue, avail) as u64,
        );
        write64_at(
            base,
            0xa0,
            qbase + core::mem::offset_of!(NetQueue, used) as u64,
        );
        // QueueReady luôn ghi 1 (không phải index): ghi 0 cho queue 0 khiến
        // RX không bao giờ ready — nguyên nhân RX im lặng ở vòng 63.
        write_at(base, 0x44, 1);
    }
    let magic = read_at(base, 0);
    let version = read_at(base, 4);
    let device = read_at(base, 8);
    if magic != 0x7472_6976 || version != 2 || device != DEVICE_NET {
        return None;
    }
    const WANT: u64 = (1 << 5) | (1 << 16) | (1 << 32);
    write_at(base, 0x70, 0);
    write_at(base, 0x70, 1 | 2);
    let offered = read_at(base, 0x10) as u64 | ((read_at(base, 0x14) as u64) << 32);
    let accepted = offered & WANT;
    write_at(base, 0x20, accepted as u32);
    write_at(base, 0x24, (accepted >> 32) as u32);
    write_at(base, 0x70, 1 | 2 | 8);
    if read_at(base, 0x70) & 8 == 0 {
        return None;
    }
    // Queue 0 (RX): reset rings, post one WRITE buffer (hdr + buf).
    let rx = core::ptr::addr_of_mut!(NET_RX_QUEUE);
    unsafe {
        (*rx).avail.idx = 0;
        (*rx).used.idx = 0;
        (*rx).hdr = [0; NET_HDR_SIZE];
        let qb = rx as *const NetQueue as u64;
        (*rx).desc[0] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, hdr) as u64,
            len: NET_HDR_SIZE as u32,
            flags: DESC_WRITE | DESC_NEXT,
            next: 1,
        };
        (*rx).desc[1] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, buf) as u64,
            len: NET_BUF_SIZE as u32,
            flags: DESC_WRITE,
            next: 0,
        };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        (*rx).avail.ring[0] = 0;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        core::ptr::write_volatile(&mut (*rx).avail.idx, 1);
    }
    attach(base, 0, rx as u64);
    // Kick queue 0: VirtIO yêu cầu notify sau khi thêm buffer RX, nếu không
    // device không ghi gói nào vào RX (vòng 63).
    write_at(base, 0x50, 0);
    // Queue 1 (TX): reset rings, no buffer yet (send posts per packet).
    let tx = core::ptr::addr_of_mut!(NET_QUEUE);
    unsafe {
        (*tx).avail.idx = 0;
        (*tx).used.idx = 0;
        (*tx).hdr = [0; NET_HDR_SIZE];
    }
    attach(base, 1, tx as u64);
    write_at(base, 0x70, 1 | 2 | 8 | 4);
    if read_at(base, 0x70) & 4 == 0 {
        return None;
    }
    unsafe {
        NET_READY = true;
        NET_BASE = base;
    }
    Some(accepted)
}

pub fn net_init_tx(base: u64) -> Option<u64> {
    net_init_both(base)
}

pub fn net_ready() -> bool {
    unsafe { NET_READY }
}

/// Build an ARP who-has request in `NET_QUEUE.buf` and transmit it on the
/// already-negotiated TX queue (vòng 47). Uses our MAC from `net_mac`,
/// asks for the QEMU user-net gateway 10.0.2.2, tells 10.0.2.15 (the usual
/// guest address). Two descriptors: 12-byte virtio-net header (zeroed) +
/// 42-byte Ethernet+ARP. Kicks the queue and polls the used ring with a
/// bounded spin; returns the device-reported length on success.
/// Never touches the block queue. Safe to call once after `net_init_tx`.
pub fn net_send_arp() -> Option<u32> {
    net_send_arp_to([10, 0, 2, 2])
}

/// Như `net_send_arp` nhưng chỉ định target IP (vòng 63: gửi thử cả gateway
/// 10.0.2.2 và DNS 10.0.2.3 để biết slirp đáp địa chỉ nào).
pub fn net_send_arp_to(tpa: [u8; 4]) -> Option<u32> {
    let (base, mac) = unsafe {
        if !NET_READY || NET_BASE == 0 {
            return None;
        }
        (NET_BASE, net_mac(NET_BASE)?)
    };
    fn write_at(base: u64, offset: u64, value: u32) {
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
    }
    // Ethernet: broadcast dst, our src, ethertype ARP (0x0806).
    // ARP: htype=1, ptype=0x0800, hlen=6, plen=4, oper=1 (request),
    // sha=our mac, spa=10.0.2.15, tha=00:00:00:00:00:00, tpa=10.0.2.2.
    const SPA: [u8; 4] = [10, 0, 2, 15];
    // 42B ARP + 18B zero padding = 60B Ethernet minimum (không tính FCS).
    // Vòng 50: một số backend bỏ qua frame runt; QEMU user-net cũng vậy.
    let mut frame = [0u8; 60];
    frame[0..6].copy_from_slice(&[0xff; 6]);
    frame[6..12].copy_from_slice(&mac);
    frame[12..14].copy_from_slice(&[0x08, 0x06]);
    frame[14..16].copy_from_slice(&[0x00, 0x01]);
    frame[16..18].copy_from_slice(&[0x08, 0x00]);
    frame[18] = 6;
    frame[19] = 4;
    frame[20..22].copy_from_slice(&[0x00, 0x01]);
    frame[22..28].copy_from_slice(&mac);
    frame[28..32].copy_from_slice(&SPA);
    frame[32..38].copy_from_slice(&[0; 6]);
    frame[38..42].copy_from_slice(&tpa);
    let queue = core::ptr::addr_of_mut!(NET_QUEUE);
    unsafe {
        let q = &mut *queue;
        q.hdr = [0; NET_HDR_SIZE];
        q.buf[..60].copy_from_slice(&frame);
        let qb = q as *const NetQueue as u64;
        q.desc[0] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, hdr) as u64,
            len: NET_HDR_SIZE as u32,
            flags: DESC_NEXT,
            next: 1,
        };
        q.desc[1] = Descriptor {
            addr: qb + core::mem::offset_of!(NetQueue, buf) as u64,
            len: frame.len() as u32,
            flags: 0,
            next: 0,
        };
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let next_avail = q.avail.idx.wrapping_add(1);
        q.avail.ring[(q.avail.idx as usize) % QUEUE_SIZE] = 0;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        core::ptr::write_volatile(&mut q.avail.idx, next_avail);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        write_at(base, 0x50, 1);
        let target = core::ptr::read_volatile(&q.used.idx).wrapping_add(1);
        let mut spins = 0u64;
        // Bound ngắn như RX (vòng 49): 20M spins mô phỏng vượt timeout QEMU.
        while core::ptr::read_volatile(&q.used.idx) != target && spins < 2_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) != target {
            return None;
        }
        // Phần tử vừa hoàn tất là ring[(used.idx - 1) % QUEUE_SIZE], không
        // phải ring[0] — quan trọng khi gửi nhiều gói (vòng 63).
        let done = q.used.idx.wrapping_sub(1) as usize % QUEUE_SIZE;
        let elem = core::ptr::read_volatile(&q.used.ring[done]);
        Some(elem.len)
    }
}

static mut BASE: u64 = 0;
static mut READY: bool = false;
static mut LAST_BASE: u64 = 0;
static mut LAST_MAGIC: u32 = 0;
static mut LAST_VERSION: u32 = 0;
static mut LAST_DEVICE: u32 = 0;
static mut LEGACY_SEEN: bool = false;

fn write32(offset: u64, value: u32) {
    unsafe { core::ptr::write_volatile((BASE + offset) as *mut u32, value) }
}

fn queue_ptr() -> *mut Queue {
    core::ptr::addr_of_mut!(QUEUE)
}

fn read_probe(candidate: u64) -> (u32, u32, u32) {
    // Probe with the candidate address directly before publishing BASE.  This
    // keeps the first MMIO read independent of the mutable global and makes a
    // bad slot fail before any device state is touched.
    let magic = unsafe { core::ptr::read_volatile(candidate as *const u32) };
    let version = unsafe { core::ptr::read_volatile((candidate + 4) as *const u32) };
    let device = unsafe { core::ptr::read_volatile((candidate + 8) as *const u32) };
    unsafe {
        BASE = candidate;
        LAST_BASE = candidate;
        LAST_MAGIC = magic;
        LAST_VERSION = version;
        LAST_DEVICE = device;
    }
    (magic, version, device)
}

pub fn probe_info() -> (u64, u32, u32, u32) {
    unsafe { (LAST_BASE, LAST_MAGIC, LAST_VERSION, LAST_DEVICE) }
}

/// Read the 6-byte MAC from a VirtIO-net config space (offset 0x100).
/// Returns None when the slot is not a net device.
pub fn net_mac(base: u64) -> Option<[u8; 6]> {
    let magic = unsafe { core::ptr::read_volatile(base as *const u32) };
    let device = unsafe { core::ptr::read_volatile((base + 8) as *const u32) };
    if magic != 0x7472_6976 || device != DEVICE_NET {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, byte) in mac.iter_mut().enumerate() {
        *byte = unsafe { core::ptr::read_volatile((base + 0x100 + i as u64) as *const u8) };
    }
    Some(mac)
}

/// Read a VirtIO-net device's offered feature bits (DeviceFeatures, 0x10/0x14).
/// Read-only: safe to call during the pre-init probe (vòng 39-44 showed only
/// post-claim state writes can hang QEMU). Returns None for non-net slots.
pub fn net_features(base: u64) -> Option<u64> {
    let magic = unsafe { core::ptr::read_volatile(base as *const u32) };
    let device = unsafe { core::ptr::read_volatile((base + 8) as *const u32) };
    if magic != 0x7472_6976 || device != DEVICE_NET {
        return None;
    }
    let lo = unsafe { core::ptr::read_volatile((base + 0x10) as *const u32) };
    let hi = unsafe { core::ptr::read_volatile((base + 0x14) as *const u32) };
    Some(lo as u64 | ((hi as u64) << 32))
}

fn init_at(candidate: u64) -> bool {
    fn read_at(base: u64, offset: u64) -> u32 {
        unsafe { core::ptr::read_volatile((base + offset) as *const u32) }
    }
    fn write_at(base: u64, offset: u64, value: u32) {
        unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
    }
    fn write64_at(base: u64, offset: u64, value: u64) {
        write_at(base, offset, value as u32);
        write_at(base, offset + 4, (value >> 32) as u32);
    }

    let (magic, version, device) = read_probe(candidate);
    if magic != 0x7472_6976 || version != 2 || device != 2 {
        return false;
    }
    write_at(candidate, 0x70, 0);
    write_at(candidate, 0x70, 1 | 2);
    write_at(candidate, 0x20, 0);
    write_at(candidate, 0x24, 0);
    write_at(candidate, 0x70, 1 | 2 | 8);
    if read_at(candidate, 0x70) & 8 == 0 {
        return false;
    }
    let queue = queue_ptr();
    unsafe {
        (*queue).avail.idx = 0;
        (*queue).used.idx = 0;
        (*queue).header = [0; 4];
        (*queue).data = [0; SECTOR_SIZE];
        (*queue).status = 0xff;
    }
    write_at(candidate, 0x30, 0);
    write_at(candidate, 0x38, QUEUE_SIZE as u32);
    let base = queue as u64;
    write64_at(
        candidate,
        0x80,
        base + core::mem::offset_of!(Queue, desc) as u64,
    );
    write64_at(
        candidate,
        0x90,
        base + core::mem::offset_of!(Queue, avail) as u64,
    );
    write64_at(
        candidate,
        0xa0,
        base + core::mem::offset_of!(Queue, used) as u64,
    );
    write_at(candidate, 0x44, 1);
    write_at(candidate, 0x70, 1 | 2 | 8 | 4);
    unsafe { READY = true };
    true
}

/// Scan a VirtIO-MMIO window and report every present device slot.
/// The callback receives `(base, device_id)`; slot 0 is the block device
/// used by `init`. Device IDs: 1 = net, 2 = block, 4 = entropy, 9 = console.
pub fn scan_devices(start: u64, end: u64, stride: u64, mut visit: impl FnMut(u64, u32)) {
    let mut base = start;
    while base < end {
        let magic = unsafe { core::ptr::read_volatile(base as *const u32) };
        let version = unsafe { core::ptr::read_volatile((base + 4) as *const u32) };
        let device = unsafe { core::ptr::read_volatile((base + 8) as *const u32) };
        if magic == 0x7472_6976 && (version == 1 || version == 2) && device != 0 {
            visit(base, device);
        }
        base += stride;
    }
}

/// Scan a VirtIO-MMIO window.  The caller supplies the window and stride so
/// RISC-V can use its own slot range while AArch64 keeps the existing one.
pub fn init_range(start: u64, end: u64, stride: u64) -> bool {
    unsafe {
        READY = false;
        LEGACY_SEEN = false;
        BASE = 0;
    }
    let mut base = start;
    while base < end {
        let (magic, version, device) = read_probe(base);
        if magic == 0x7472_6976 && device == 2 {
            if version == 1 {
                unsafe { LEGACY_SEEN = true };
            } else if version == 2 && init_at(base) {
                return true;
            }
        }
        base += stride;
    }
    false
}

/// Scan the AArch64 QEMU virtio-mmio window (stride 0x200, 32 slots).
pub fn init() -> bool {
    init_range(0x0a00_0000, 0x0a00_4000, 0x200)
}

pub fn initialized() -> bool {
    unsafe { READY }
}

pub fn legacy_seen() -> bool {
    unsafe { LEGACY_SEEN }
}

/// DMA range covered by the static queue.  It must be inside the platform's
/// kernel-accessible identity range.
pub fn dma_range() -> (u64, u64) {
    let base = core::ptr::addr_of!(QUEUE) as u64;
    (base, base + core::mem::size_of::<Queue>() as u64)
}

fn request(write: bool, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    if unsafe { !READY || BASE == 0 } {
        return false;
    }
    let queue = queue_ptr();
    unsafe {
        let q = &mut *queue;
        q.header = [
            if write { 1 } else { 0 },
            0,
            sector as u32,
            (sector >> 32) as u32,
        ];
        if write {
            q.data.copy_from_slice(buffer);
        }
        q.status = 0xff;
        let base = q as *const Queue as u64;
        q.desc[0] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, header) as u64,
            len: 16,
            flags: DESC_NEXT,
            next: 1,
        };
        q.desc[1] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, data) as u64,
            len: SECTOR_SIZE as u32,
            flags: DESC_NEXT | if write { 0 } else { DESC_WRITE },
            next: 2,
        };
        q.desc[2] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, status) as u64,
            len: 1,
            flags: DESC_WRITE,
            next: 0,
        };
        let slot = core::ptr::read_volatile(&q.avail.idx) as usize % QUEUE_SIZE;
        q.avail.ring[slot] = 0;
        fence(Ordering::SeqCst);
        let next_avail = core::ptr::read_volatile(&q.avail.idx).wrapping_add(1);
        core::ptr::write_volatile(&mut q.avail.idx, next_avail);
        fence(Ordering::SeqCst);
        write32(0x50, 0);
        let target = core::ptr::read_volatile(&q.used.idx).wrapping_add(1);
        let mut spins = 0u64;
        while core::ptr::read_volatile(&q.used.idx) != target && spins < 20_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        fence(Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) != target
            || core::ptr::read_volatile(&q.status) != 0
        {
            return false;
        }
        if !write {
            buffer.copy_from_slice(&q.data);
        }
    }
    true
}

fn request_with_retry(write: bool, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    for _ in 0..3 {
        if request(write, sector, buffer) {
            return true;
        }
        // QEMU's virtio-mmio device can need a fresh queue after an early
        // probe; reinitialise the same slot before retrying.
        let base = unsafe { BASE };
        if base != 0 && !init_at(base) {
            break;
        }
    }
    false
}

pub fn read_sector(sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    request_with_retry(false, sector, buffer)
}
pub fn write_sector(sector: u64, buffer: &[u8; SECTOR_SIZE]) -> bool {
    let mut local = [0u8; SECTOR_SIZE];
    local.copy_from_slice(buffer);
    request_with_retry(true, sector, &mut local)
}

fn request_flush() -> bool {
    if unsafe { !READY || BASE == 0 } {
        return false;
    }
    let queue = queue_ptr();
    unsafe {
        let q = &mut *queue;
        // VIRTIO_BLK_T_FLUSH has no data payload; header + status are enough.
        q.header = [4, 0, 0, 0];
        q.status = 0xff;
        let base = q as *const Queue as u64;
        q.desc[0] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, header) as u64,
            len: 16,
            flags: DESC_NEXT,
            next: 1,
        };
        q.desc[1] = Descriptor {
            addr: base + core::mem::offset_of!(Queue, status) as u64,
            len: 1,
            flags: DESC_WRITE,
            next: 0,
        };
        let slot = core::ptr::read_volatile(&q.avail.idx) as usize % QUEUE_SIZE;
        q.avail.ring[slot] = 0;
        fence(Ordering::SeqCst);
        let next_avail = core::ptr::read_volatile(&q.avail.idx).wrapping_add(1);
        core::ptr::write_volatile(&mut q.avail.idx, next_avail);
        fence(Ordering::SeqCst);
        write32(0x50, 0);
        let target = core::ptr::read_volatile(&q.used.idx).wrapping_add(1);
        let mut spins = 0u64;
        while core::ptr::read_volatile(&q.used.idx) != target && spins < 20_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        fence(Ordering::SeqCst);
        if core::ptr::read_volatile(&q.used.idx) != target
            || core::ptr::read_volatile(&q.status) != 0
        {
            return false;
        }
    }
    true
}

pub fn flush() -> bool {
    for _ in 0..3 {
        if request_flush() {
            return true;
        }
        let base = unsafe { BASE };
        if base != 0 && !init_at(base) {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::checksum;

    /// RFC 1071 mẫu IPv4 header: checksum tính ra phải đúng 0xb861.
    #[test]
    fn checksum_matches_rfc1071_ipv4_header() {
        let header_with_zero = [
            0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0x00, 0x00, 0xc0, 0xa8,
            0x00, 0x01, 0xc0, 0xa8, 0x00, 0xc7,
        ];
        assert_eq!(checksum(&header_with_zero), 0xb861);
    }

    /// Với checksum đã điền, tổng kiểm tra phải bằng 0 (bù một).
    #[test]
    fn checksum_of_valid_header_is_zero() {
        let header = [
            0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0xb8, 0x61, 0xc0, 0xa8,
            0x00, 0x01, 0xc0, 0xa8, 0x00, 0xc7,
        ];
        assert_eq!(checksum(&header), 0);
    }

    /// DNS response tối giản: 1 question (a.example) + 1 answer A 93.184.216.34.
    #[test]
    fn dns_a_record_parsed() {
        let mut p = [0u8; 48];
        let bytes: [u8; 43] = [
            0x12, 0x34, 0x81, 0x80, // id, flags
            0x00, 0x01, 0x00, 0x01, // qd=1 an=1
            0x00, 0x00, 0x00, 0x00, // ns, ar
            0x01, b'a', 0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x00, 0x00, 0x01, 0x00,
            0x01, // QTYPE A, QCLASS IN
            0xc0, 0x0c, // pointer tới question name
            0x00, 0x01, 0x00, 0x01, // TYPE A, CLASS IN
            0x00, 0x00, 0x00, 0x3c, // TTL 60
            0x00, 0x04, // RDLENGTH
            93, 184, 216, 34,
        ];
        p[..bytes.len()].copy_from_slice(&bytes);
        assert_eq!(
            super::parse_dns_a(&p[..bytes.len()]),
            Some([93, 184, 216, 34])
        );
    }

    /// Không có answer A thì trả None.
    #[test]
    fn dns_without_a_record_is_none() {
        let bytes: [u8; 19] = [
            0x12, 0x34, 0x81, 0x80, // id, flags
            0x00, 0x01, 0x00, 0x00, // qd=1 an=0
            0x00, 0x00, 0x00, 0x00, // ns, ar
            0x01, b'a', 0x00, // QNAME
            0x00, 0x01, 0x00, 0x01, // QTYPE A, QCLASS IN
        ];
        assert_eq!(super::parse_dns_a(&bytes), None);
    }

    /// Độ dài lẻ được đệm 0 ở byte cuối.
    #[test]
    fn checksum_handles_odd_length() {
        assert_eq!(
            checksum(&[0x12, 0x34, 0x56]),
            checksum(&[0x12, 0x34, 0x56, 0x00])
        );
    }
}

// ---------------------------------------------------------------------------
// VirtIO-GPU tối thiểu (vòng 66, M4 display).
// Chỉ dùng control queue (queue 0): GET_DISPLAY_INFO, RESOURCE_CREATE_2D,
// RESOURCE_ATTACH_BACKING, SET_SCANOUT, TRANSFER_TO_HOST_2D, RESOURCE_FLUSH.
// ---------------------------------------------------------------------------

pub const GPU_DEVICE_ID: u32 = 16;
/// Chiều rộng/cao khung hình ta tạo (đủ nhỏ để static buffer vừa).
pub const GPU_WIDTH: u32 = 160;
pub const GPU_HEIGHT: u32 = 120;
const GPU_FMT_B8G8R8X8: u32 = 2;
const GPU_RESP_OK_NODATA: u32 = 0x1100;
const GPU_RESP_OK_DISPLAY_INFO: u32 = 0x1101;
const GPU_MAX_DESC: usize = 8;

#[repr(C, align(16))]
struct GpuDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C, align(16))]
struct GpuAvail {
    flags: u16,
    idx: u16,
    ring: [u16; GPU_MAX_DESC],
}

#[repr(C, align(16))]
struct GpuUsed {
    flags: u16,
    idx: u16,
    ring: [GpuUsedElem; GPU_MAX_DESC],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuUsedElem {
    id: u32,
    len: u32,
}

#[repr(C, align(16))]
struct GpuQueue {
    desc: [GpuDesc; GPU_MAX_DESC],
    avail: GpuAvail,
    used: GpuUsed,
    cmd: [u8; 256],
    resp: [u8; 512],
    framebuffer: [u32; (GPU_WIDTH * GPU_HEIGHT) as usize],
}

static mut GPU_QUEUE: GpuQueue = GpuQueue {
    desc: [const {
        GpuDesc {
            addr: 0,
            len: 0,
            flags: 0,
            next: 0,
        }
    }; GPU_MAX_DESC],
    avail: GpuAvail {
        flags: 0,
        idx: 0,
        ring: [0; GPU_MAX_DESC],
    },
    used: GpuUsed {
        flags: 0,
        idx: 0,
        ring: [GpuUsedElem { id: 0, len: 0 }; GPU_MAX_DESC],
    },
    cmd: [0; 256],
    resp: [0; 512],
    framebuffer: [const { 0u32 }; (GPU_WIDTH * GPU_HEIGHT) as usize],
};

static mut GPU_BASE: u64 = 0;
static mut GPU_READY: bool = false;

fn gpu_write32(base: u64, offset: u64, value: u32) {
    unsafe { core::ptr::write_volatile((base + offset) as *mut u32, value) }
}

/// Khởi tạo virtio-gpu tại `base`: reset, ACK/DRIVER, queue 0, DRIVER_OK.
pub fn gpu_init(base: u64) -> bool {
    unsafe {
        GPU_READY = false;
        GPU_BASE = 0;
        let q = &mut *core::ptr::addr_of_mut!(GPU_QUEUE);
        q.avail.idx = 0;
        q.used.idx = 0;
    }
    gpu_write32(base, 0x70, 0); // reset
    gpu_write32(base, 0x70, 1); // ACK
    gpu_write32(base, 0x70, 3); // ACK | DRIVER
                                // Không cần feature nào (không virgl, không resource uuid).
    gpu_write32(base, 0x14, 0);
    gpu_write32(base, 0x10, 0);
    gpu_write32(base, 0x70, 11); // ACK|DRIVER|FEATURES_OK
    gpu_write32(base, 0x30, 0); // control queue
    let qb = core::ptr::addr_of_mut!(GPU_QUEUE) as u64;
    let desc_addr = qb + core::mem::offset_of!(GpuQueue, desc) as u64;
    let avail_addr = qb + core::mem::offset_of!(GpuQueue, avail) as u64;
    let used_addr = qb + core::mem::offset_of!(GpuQueue, used) as u64;
    gpu_write32(base, 0x38, GPU_MAX_DESC as u32);
    gpu_write32(base, 0x80, desc_addr as u32);
    gpu_write32(base, 0x84, (desc_addr >> 32) as u32);
    gpu_write32(base, 0x90, avail_addr as u32);
    gpu_write32(base, 0x94, (avail_addr >> 32) as u32);
    gpu_write32(base, 0xa0, used_addr as u32);
    gpu_write32(base, 0xa4, (used_addr >> 32) as u32);
    gpu_write32(base, 0x44, 1); // QueueReady
    gpu_write32(base, 0x70, 15); // DRIVER_OK
    unsafe {
        GPU_BASE = base;
        GPU_READY = true;
    }
    // GET_DISPLAY_INFO để xác nhận kênh điều khiển sống.
    gpu_command(0x0100, &[])
}

/// Gửi một lệnh điều khiển: `cmd_type` + payload; chờ response. Trả về true
/// khi response là OK (0x1100/0x1101). Payload chép ngay sau header 24 byte.
fn gpu_command(cmd_type: u32, payload: &[u8]) -> bool {
    let base = unsafe { GPU_BASE };
    if unsafe { !GPU_READY } || base == 0 {
        return false;
    }
    let qb = core::ptr::addr_of_mut!(GPU_QUEUE) as u64;
    let q = unsafe { &mut *core::ptr::addr_of_mut!(GPU_QUEUE) };
    let cmd_len = 24 + payload.len();
    if cmd_len > q.cmd.len() {
        return false;
    }
    q.cmd[0..4].copy_from_slice(&cmd_type.to_le_bytes());
    for byte in q.cmd[4..24].iter_mut() {
        *byte = 0;
    }
    q.cmd[24..cmd_len].copy_from_slice(payload);
    for byte in q.resp.iter_mut() {
        *byte = 0;
    }
    q.desc[0] = GpuDesc {
        addr: qb + core::mem::offset_of!(GpuQueue, cmd) as u64,
        len: cmd_len as u32,
        flags: DESC_NEXT,
        next: 1,
    };
    q.desc[1] = GpuDesc {
        addr: qb + core::mem::offset_of!(GpuQueue, resp) as u64,
        len: q.resp.len() as u32,
        flags: DESC_WRITE,
        next: 0,
    };
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    let next = q.avail.idx.wrapping_add(1);
    q.avail.ring[(q.avail.idx as usize) % GPU_MAX_DESC] = 0;
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    unsafe { core::ptr::write_volatile(&mut q.avail.idx, next) };
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    gpu_write32(base, 0x50, 0);
    let target = unsafe { core::ptr::read_volatile(&q.used.idx) }.wrapping_add(1);
    let mut spins = 0u64;
    while unsafe { core::ptr::read_volatile(&q.used.idx) } != target && spins < 4_000_000 {
        core::hint::spin_loop();
        spins += 1;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    if unsafe { core::ptr::read_volatile(&q.used.idx) } != target {
        return false;
    }
    let resp_type = u32::from_le_bytes([q.resp[0], q.resp[1], q.resp[2], q.resp[3]]);
    resp_type == GPU_RESP_OK_NODATA || resp_type == GPU_RESP_OK_DISPLAY_INFO
}

/// Dựng khung hình: tạo resource 2D, attach backing, set scanout, tô màu và
/// flush. Trả về true nếu mọi lệnh thành công.
pub fn gpu_draw_test_pattern() -> bool {
    if unsafe { !GPU_READY } {
        return false;
    }
    // RESOURCE_CREATE_2D: resource_id=1, format, w, h.
    let mut create = [0u8; 16];
    create[0..4].copy_from_slice(&1u32.to_le_bytes());
    create[4..8].copy_from_slice(&GPU_FMT_B8G8R8X8.to_le_bytes());
    create[8..12].copy_from_slice(&GPU_WIDTH.to_le_bytes());
    create[12..16].copy_from_slice(&GPU_HEIGHT.to_le_bytes());
    if !gpu_command(0x0101, &create) {
        return false;
    }
    // RESOURCE_ATTACH_BACKING: resource 1, 1 entry {addr, length, pad}.
    let qb = core::ptr::addr_of_mut!(GPU_QUEUE) as u64;
    let fb_addr = qb + core::mem::offset_of!(GpuQueue, framebuffer) as u64;
    let mut attach = [0u8; 24];
    attach[0..4].copy_from_slice(&1u32.to_le_bytes());
    attach[4..8].copy_from_slice(&1u32.to_le_bytes());
    attach[8..16].copy_from_slice(&fb_addr.to_le_bytes());
    attach[16..20].copy_from_slice(&(GPU_WIDTH * GPU_HEIGHT * 4).to_le_bytes());
    if !gpu_command(0x0106, &attach) {
        return false;
    }
    // SET_SCANOUT: rect(0,0,w,h), scanout_id=0, resource_id=1.
    let mut scanout = [0u8; 24];
    scanout[0..4].copy_from_slice(&0u32.to_le_bytes());
    scanout[4..8].copy_from_slice(&0u32.to_le_bytes());
    scanout[8..12].copy_from_slice(&GPU_WIDTH.to_le_bytes());
    scanout[12..16].copy_from_slice(&GPU_HEIGHT.to_le_bytes());
    scanout[16..20].copy_from_slice(&0u32.to_le_bytes());
    scanout[20..24].copy_from_slice(&1u32.to_le_bytes());
    if !gpu_command(0x0103, &scanout) {
        return false;
    }
    // Tô màu: dải màu RGB để screendump dễ nhận biết (X8R8G8B8 little-endian
    // nên word = 0x00RRGGBB).
    let q = unsafe { &mut *core::ptr::addr_of_mut!(GPU_QUEUE) };
    let mut y = 0u32;
    while y < GPU_HEIGHT {
        let mut x = 0u32;
        while x < GPU_WIDTH {
            let r = (x * 255 / GPU_WIDTH) & 0xff;
            let g = (y * 255 / GPU_HEIGHT) & 0xff;
            let b = 0x40u32;
            q.framebuffer[(y * GPU_WIDTH + x) as usize] = (r << 16) | (g << 8) | b;
            x += 1;
        }
        y += 1;
    }
    // TRANSFER_TO_HOST_2D: rect + offset + resource_id.
    let mut transfer = [0u8; 32];
    transfer[0..4].copy_from_slice(&0u32.to_le_bytes());
    transfer[4..8].copy_from_slice(&0u32.to_le_bytes());
    transfer[8..12].copy_from_slice(&GPU_WIDTH.to_le_bytes());
    transfer[12..16].copy_from_slice(&GPU_HEIGHT.to_le_bytes());
    transfer[16..24].copy_from_slice(&0u64.to_le_bytes());
    transfer[24..28].copy_from_slice(&1u32.to_le_bytes());
    if !gpu_command(0x0105, &transfer) {
        return false;
    }
    // RESOURCE_FLUSH: rect + resource_id.
    let mut flush = [0u8; 24];
    flush[0..4].copy_from_slice(&0u32.to_le_bytes());
    flush[4..8].copy_from_slice(&0u32.to_le_bytes());
    flush[8..12].copy_from_slice(&GPU_WIDTH.to_le_bytes());
    flush[12..16].copy_from_slice(&GPU_HEIGHT.to_le_bytes());
    flush[16..20].copy_from_slice(&1u32.to_le_bytes());
    gpu_command(0x0104, &flush)
}

/// Địa chỉ (và độ dài) framebuffer để test có thể kiểm tra.
pub fn gpu_framebuffer_info() -> (u64, u32, u32) {
    let qb = core::ptr::addr_of_mut!(GPU_QUEUE) as u64;
    let fb = qb + core::mem::offset_of!(GpuQueue, framebuffer) as u64;
    (fb, GPU_WIDTH, GPU_HEIGHT)
}
