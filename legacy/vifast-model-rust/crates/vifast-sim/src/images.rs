//! Chương trình user space mẫu (tương đương các file ELF trong image hệ thống).
//!
//! Đây là code user, không phải kernel. Trên hệ thật chúng là các binary riêng
//! dùng crate `vifast-user`.

use std::collections::BTreeMap;

use vifast_abi::*;

use crate::user::{program, Images, Sys};

pub const GET: u64 = 1;
pub const PRINT: u64 = 2;

/// Đóng gói chuỗi (tối đa 63 byte) vào các word của message.
pub fn pack_text(text: &str) -> Vec<u64> {
    let bytes = text.as_bytes();
    let n = bytes.len().min(63);
    let mut raw = vec![n as u8];
    raw.extend_from_slice(&bytes[..n]);
    while raw.len() % 8 != 0 {
        raw.push(0);
    }
    raw.chunks(8)
        .map(|c| {
            let mut b = [0u8; 8];
            b.copy_from_slice(c);
            u64::from_le_bytes(b)
        })
        .collect()
}

pub fn unpack_text(words: &[u64]) -> String {
    let raw: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    if raw.is_empty() {
        return String::new();
    }
    let n = (raw[0] as usize).min(raw.len() - 1);
    String::from_utf8_lossy(&raw[1..1 + n]).into_owned()
}

pub fn u64_bytes(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

pub fn bytes_u64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    let n = b.len().min(8);
    a[..n].copy_from_slice(&b[..n]);
    u64::from_le_bytes(a)
}

// ---------------------------------------------------------------------------
// Hệ thống chung
// ---------------------------------------------------------------------------

/// Chính sách phục hồi ở user space: restart có giới hạn, quá thì DEGRADED.
pub async fn supervisor(s: Sys) -> Result<(), String> {
    let max_restarts = s.env_or("max_restarts", 3) as usize;
    let window = s.env_or("window", 500);
    let dev_mode = s.env_or("dev_mode", 0) != 0;
    let fault_ep = s.env("fault_ep")?;
    let names: BTreeMap<u64, String> = s
        .env_map()
        .iter()
        .filter(|(k, _)| k.starts_with("tcb_"))
        .map(|(k, v)| (*v, k[4..].to_string()))
        .collect();
    let mut history: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    loop {
        let r = s.recv(fault_ep).await;
        let Some(msg) = r.msg else { continue };
        if !r.is_ok() || msg.label != LABEL_FAULT {
            continue;
        }
        let kind = FaultKind::from_u64(msg.data[0]);
        let kname = kind.map_or("?", |k| k.name());
        let slot = r.badge;
        let name = names.get(&slot).cloned().unwrap_or_else(|| format!("slot {}", slot));
        if kind == Some(FaultKind::Exited) {
            s.print(format!("{} đã kết thúc", name)).await;
            continue;
        }
        if dev_mode {
            s.print(format!("DEV MODE: {} lỗi {}, dừng toàn hệ thống", name, kname)).await;
            for sl in names.keys() {
                s.sys(Syscall::TcbSuspend { tcb: *sl }).await;
            }
            break;
        }
        let now = s.get_time().await;
        let mut recent: Vec<u64> = history
            .get(&slot)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|x| now - x < window)
            .collect();
        if recent.len() >= max_restarts {
            history.insert(slot, recent);
            s.print(format!("DEGRADED: {} lỗi quá {} lần, không restart", name, max_restarts)).await;
            continue;
        }
        recent.push(now);
        let n = recent.len();
        history.insert(slot, recent);
        s.sys(Syscall::TcbRestart { tcb: slot }).await;
        s.print(format!("restart {} ({}) lần {}", name, kname, n)).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Profile space
// ---------------------------------------------------------------------------

/// Driver ở user space: ngắt đến qua bound notification, phục vụ request qua endpoint.
pub async fn imu_driver(s: Sys) -> Result<(), String> {
    let ep = s.env("imu_ep")?;
    let irq = s.env("imu_irq")?;
    let mut sample = 0u64;
    loop {
        let r = s.recv(ep).await;
        let label = r.msg.map_or(0, |m| m.label);
        if label == LABEL_NOTIFICATION {
            sample += 1; // giả lập đọc thanh ghi cảm biến
            s.sys(Syscall::IrqAck { handler: irq }).await;
        } else if label == GET {
            s.reply(Message::new(GET, &[sample])).await;
        }
    }
}

pub async fn attitude_ctrl(s: Sys) -> Result<(), String> {
    let iterations = s.env_or("iterations", 40);
    let imu = s.env("imu")?;
    let ram = s.env("ram_base")?;
    for i in 0..iterations {
        let r = s.call(imu, Message::empty(GET)).await;
        if !r.is_ok() {
            s.print(format!("lỗi gọi IMU: {}", r.err.name())).await;
            continue;
        }
        let sample = r.msg.map_or(0, |m| m.data[0]);
        s.store(ram, &u64_bytes(sample)).await;
        if i % 10 == 0 {
            s.print(format!("vòng {}: mẫu IMU = {}", i, sample)).await;
        }
        s.yield_now().await;
    }
    s.print("hoàn thành điều khiển tư thế").await;
    Ok(())
}

/// Tác vụ nặng, ưu tiên thấp, bị giới hạn ngân sách CPU, lỗi sau vài vòng.
pub async fn payload(s: Sys) -> Result<(), String> {
    let ram = s.env("ram_base")?;
    for _ in 0..s.env_or("crash_after", 6) {
        s.load(ram, 8).await;
    }
    s.print("xử lý ảnh... gặp lỗi").await;
    Err("RuntimeError: lỗi giả lập trong payload".to_string())
}

pub async fn producer(s: Sys) -> Result<(), String> {
    let buf = s.env("buf_addr")?;
    let ready = s.env("data_ready")?;
    let ack = s.env("data_ack")?;
    for i in 0..3 {
        let text = format!("khung ảnh {}", i).into_bytes();
        let mut data = (text.len() as u16).to_le_bytes().to_vec();
        data.extend_from_slice(&text);
        s.store(buf, &data).await;
        s.signal(ready).await;
        s.wait(ack).await;
    }
    Ok(())
}

pub async fn consumer(s: Sys) -> Result<(), String> {
    let buf = s.env("buf_addr")?;
    let ready = s.env("data_ready")?;
    let ack = s.env("data_ack")?;
    for _ in 0..3 {
        s.wait(ready).await;
        let n = bytes_u64(&s.load(buf, 2).await);
        let body = s.load(buf + 2, n).await;
        s.print(format!("nhận qua bộ nhớ chia sẻ: {}", String::from_utf8_lossy(&body))).await;
        s.signal(ack).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Driver thiết bị DMA
// ---------------------------------------------------------------------------

/// Driver camera: cấp vùng đệm cho thiết bị qua IOMMU rồi lập trình thanh ghi MMIO.
pub async fn cam_driver(s: Sys) -> Result<(), String> {
    let regs = s.env("cam_regs_addr")?;
    let buf = s.env("dmabuf_addr")?;
    let iova = 0x1000;
    let r = s
        .sys(Syscall::IoMap { frame: s.env("dmabuf")?, iospace: s.env("cam_io")?, iova, rights: Rights::RW })
        .await;
    if !r.is_ok() {
        s.print(format!("IoMap lỗi: {}", r.err.name())).await;
        return Ok(());
    }
    for _ in 0..s.env_or("frames", 3) {
        s.store(regs, &u64_bytes(iova)).await;
        s.store(regs + 0x08, &u64_bytes(16)).await;
        s.store(regs + 0x10, &u64_bytes(1)).await;
        s.wait(s.env("cam_ntfn")?).await;
        let status = bytes_u64(&s.load(regs + 0x18, 8).await);
        let data = s.load(buf, 16).await;
        let text = String::from_utf8_lossy(&data).trim_end_matches('\0').to_string();
        s.print(format!("DMA xong (status={}): {}", status, text)).await;
        s.sys(Syscall::IrqAck { handler: s.env("cam_irq")? }).await;
    }
    Ok(())
}

/// Driver độc hại: ra lệnh cho thiết bị ghi DMA thẳng vào địa chỉ vật lý của kernel.
pub async fn dma_attacker(s: Sys) -> Result<(), String> {
    let regs = s.env("cam_regs_addr")?;
    s.store(regs, &u64_bytes(s.env("target")?)).await;
    s.store(regs + 0x08, &u64_bytes(16)).await;
    s.store(regs + 0x10, &u64_bytes(1)).await;
    s.wait(s.env("cam_ntfn")?).await;
    let status = bytes_u64(&s.load(regs + 0x18, 8).await);
    s.print(format!("status={}", status)).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Profile desktop
// ---------------------------------------------------------------------------

pub async fn console(s: Sys) -> Result<(), String> {
    let ep = s.env("console_ep")?;
    loop {
        let r = s.recv(ep).await;
        let text = r.msg.map(|m| unpack_text(m.words())).unwrap_or_default();
        s.print(format!("[badge {}] {}", r.badge, text)).await;
        s.reply(Message::empty(0)).await;
    }
}

pub async fn hello(s: Sys) -> Result<(), String> {
    let console = s.env("console")?;
    for i in 0..3 {
        let words = pack_text(&format!("xin chào lần {}", i));
        let r = s.call(console, Message::new(PRINT, &words)).await;
        if !r.is_ok() {
            s.print(format!("console lỗi: {}", r.err.name())).await;
        }
    }
    Ok(())
}

/// Tạo tiến trình mới lúc chạy: TCB + CNode (có guard) + VSpace + page table + 1 trang RAM.
pub async fn init(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    let (s_tcb, s_cn, s_vs, s_fr, s_pt1, s_pt2, s_cng) = (20, 21, 22, 23, 24, 25, 26);
    let objs = [
        (s_tcb, ObjType::Tcb, 0),
        (s_cn, ObjType::CNode, 4),
        (s_vs, ObjType::VSpace, 0),
        (s_fr, ObjType::Frame, 0),
        (s_pt1, ObjType::PageTable, 0),
        (s_pt2, ObjType::PageTable, 0),
    ];
    for (slot, ty, bits) in objs {
        let r = s.retype(mem, ty, bits, slot).await;
        if !r.is_ok() {
            s.print(format!("retype lỗi: {}", r.err.name())).await;
            return Ok(());
        }
    }
    // CSpace 1 cấp, radix 4: guard 60 bit
    s.sys(Syscall::CNodeMint { src: s_cn, dest_index: s_cng, guard: 0, guard_bits: 60, dest_cnode: 0 }).await;
    s.sys(Syscall::CapMint { src: s.env("console")?, dest_index: 1, rights: Rights::WRITE, badge: 7, dest_cnode: s_cn })
        .await;
    s.sys(Syscall::PageTableMap { page_table: s_pt1, vspace: s_vs, vaddr: 0x10_0000 }).await;
    s.sys(Syscall::PageTableMap { page_table: s_pt2, vspace: s_vs, vaddr: 0x10_0000 }).await;
    let r = s.sys(Syscall::FrameMap { frame: s_fr, vspace: s_vs, vaddr: 0x10_0000, rights: Rights::RW }).await;
    if !r.is_ok() {
        s.print(format!("map lỗi: {}", r.err.name())).await;
        return Ok(());
    }
    let r = s
        .sys(Syscall::TcbConfigure {
            tcb: s_tcb,
            cspace: s_cng,
            vspace: s_vs,
            priority: 50,
            mcp: 50,
            image: s.env("img_hello")? as u32,
            args: [1, 0x10_0000, 0, 0],
            fault_ep: 0,
        })
        .await;
    if !r.is_ok() {
        s.print(format!("configure lỗi: {}", r.err.name())).await;
        return Ok(());
    }
    s.sys(Syscall::TcbResume { tcb: s_tcb }).await;
    s.print("đã tạo tiến trình hello lúc chạy").await;
    Ok(())
}

/// Pager ở user space: nhận VM fault, cấp trang (và page table nếu thiếu) rồi reply.
pub async fn pager(s: Sys) -> Result<(), String> {
    let ep = s.env("pager_ep")?;
    let mem = s.env("mem")?;
    let vs = s.env("lazy_vs")?;
    let (mut next_slot, mut pt_slot) = (30u64, 50u64);
    loop {
        let r = s.recv(ep).await;
        let Some(msg) = r.msg else { continue };
        let kind = FaultKind::from_u64(msg.data[0]);
        let addr = msg.data[1];
        if kind != Some(FaultKind::VmFault) {
            s.print(format!("bỏ qua fault {}", kind.map_or("?", |k| k.name()))).await;
            continue;
        }
        let page = addr & !(PAGE_SIZE - 1);
        let rr = s.retype(mem, ObjType::Frame, 0, next_slot).await;
        if !rr.is_ok() {
            s.print(format!("hết bộ nhớ: {}", rr.err.name())).await;
            continue;
        }
        loop {
            let m = s.sys(Syscall::FrameMap { frame: next_slot, vspace: vs, vaddr: page, rights: Rights::RW }).await;
            if m.err != KError::MissingPageTable {
                break;
            }
            s.retype(mem, ObjType::PageTable, 0, pt_slot).await;
            s.sys(Syscall::PageTableMap { page_table: pt_slot, vspace: vs, vaddr: page }).await;
            pt_slot += 1;
        }
        next_slot += 1;
        s.print(format!("cấp trang {:#x} theo yêu cầu", page)).await;
        s.reply(Message::empty(0)).await;
    }
}

pub async fn lazy(s: Sys) -> Result<(), String> {
    let base = 0x4000_0000u64;
    for i in 0..3u64 {
        s.store(base + i * PAGE_SIZE + 8, format!("trang {}", i).as_bytes()).await;
    }
    let data = s.load(base + 8, 7).await;
    s.print(format!("đọc lại: {}", String::from_utf8_lossy(&data))).await;
    Ok(())
}

/// Bộ image mặc định. Thứ tự đăng ký = mã image.
pub fn default_images() -> Images {
    let mut im = Images::default();
    im.register("supervisor", &[], |s| program(supervisor(s)));
    im.register("imu_driver", &[], |s| program(imu_driver(s)));
    im.register("attitude_ctrl", &[], |s| program(attitude_ctrl(s)));
    im.register("payload", &[], |s| program(payload(s)));
    im.register("producer", &[], |s| program(producer(s)));
    im.register("consumer", &[], |s| program(consumer(s)));
    im.register("cam_driver", &[], |s| program(cam_driver(s)));
    im.register("dma_attacker", &[], |s| program(dma_attacker(s)));
    im.register("console", &[], |s| program(console(s)));
    im.register("hello", &["console", "ram_base"], |s| program(hello(s)));
    im.register("init", &[], |s| program(init(s)));
    im.register("pager", &[], |s| program(pager(s)));
    im.register("lazy", &[], |s| program(lazy(s)));
    im
}
