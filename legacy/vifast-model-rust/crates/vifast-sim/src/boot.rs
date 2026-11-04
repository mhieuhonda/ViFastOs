//! Dựng hệ thống từ manifest TOML. Đóng vai trò "root task" / công cụ build image.
//!
//! Hai profile dùng chung một kernel:
//! - `desktop`: task được phát untyped + IRQ control + sched control, nên tạo được
//!   tiến trình, nhận thiết bị, cấu hình ngân sách CPU lúc chạy.
//! - `space`: cấm phát các quyền đó; boot xong xoá luôn untyped gốc, IRQ control,
//!   sched control. Không ai còn quyền tạo object mới: hệ thống "tĩnh" nhờ
//!   capability, kernel không có chế độ riêng.
//!
//! Quy ước fault: badge của fault endpoint = slot chứa TCB cap của task đó trong supervisor.

use std::collections::BTreeMap;
use std::fmt;

use toml::{Table, Value};
use vifast_abi::*;
use vifast_kernel::{Cap, CNode, Frame, Header, IoSpace, Kernel, Kind, Object, ObjRef, SlotAddr, TcbConfig};

use crate::hw::{DmaSensor, Hw, KERNEL_RESERVED, RAM_PHYS_BASE};
use crate::machine::Machine;
use crate::user::{Env, Images};

pub const BOOT_CNODE_BITS: u32 = 12;
/// Địa chỉ ảo vùng RAM riêng của mỗi task.
pub const RAM_BASE: u64 = 0x10_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootError(pub String);

impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BootError {}

impl From<KError> for BootError {
    fn from(e: KError) -> Self {
        BootError(format!("lỗi kernel khi boot: {}", e.name()))
    }
}

fn err<T>(text: impl Into<String>) -> Result<T, BootError> {
    Err(BootError(text.into()))
}

fn as_u64(v: &Value, what: &str) -> Result<u64, BootError> {
    match v {
        Value::Integer(i) if *i >= 0 => Ok(*i as u64),
        Value::Boolean(b) => Ok(*b as u64),
        _ => err(format!("'{}' phải là số nguyên không âm", what)),
    }
}

fn get_u64(t: &Table, key: &str, default: u64) -> Result<u64, BootError> {
    match t.get(key) {
        Some(v) => as_u64(v, key),
        None => Ok(default),
    }
}

fn get_str<'a>(t: &'a Table, key: &str) -> Option<&'a str> {
    t.get(key).and_then(|v| v.as_str())
}

fn req_str<'a>(t: &'a Table, key: &str) -> Result<&'a str, BootError> {
    get_str(t, key).ok_or_else(|| BootError(format!("manifest thiếu '{}'", key)))
}

fn tables<'a>(t: &'a Table, key: &str) -> Result<Vec<&'a Table>, BootError> {
    match t.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_table().ok_or_else(|| BootError(format!("'{}' phải là mảng bảng", key))))
            .collect(),
        Some(_) => err(format!("'{}' phải là mảng bảng", key)),
    }
}

/// Đọc manifest từ file.
pub fn load_manifest(path: &str) -> Result<Table, BootError> {
    let text = std::fs::read_to_string(path).map_err(|e| BootError(format!("không đọc được {}: {}", path, e)))?;
    parse_manifest(&text)
}

pub fn parse_manifest(text: &str) -> Result<Table, BootError> {
    text.parse::<Table>().map_err(|e| BootError(format!("TOML lỗi: {}", e)))
}

/// Dựng máy từ manifest dạng chuỗi TOML.
pub fn build_str(manifest: &str, images: Images, verbose: bool) -> Result<Machine, BootError> {
    build(&parse_manifest(manifest)?, images, verbose)
}

pub fn build(m: &Table, images: Images, verbose: bool) -> Result<Machine, BootError> {
    let profile = get_str(m, "profile").unwrap_or("desktop").to_string();
    if profile != "desktop" && profile != "space" {
        return err(format!("profile không hợp lệ: {}", profile));
    }
    let mem = get_u64(m, "memory", 1 << 22)?;
    let harts = get_u64(m, "harts", 1)? as usize;
    let iommu = m.get("iommu").and_then(|v| v.as_bool()).unwrap_or(true);
    let mut machine = Machine::new(images, mem, harts, iommu, verbose);
    let ntasks = tables(m, "tasks")?.len();
    build_inner(&mut machine, m, &profile, mem)?;
    let text = format!(
        "profile={}, {} task, {} hart, iommu={}",
        profile,
        ntasks,
        machine.kernel.harts.len(),
        if iommu { "bật" } else { "tắt" }
    );
    machine.kernel.klog("boot", &text);
    Ok(machine)
}

struct Builder {
    cn: ObjRef,
    next: usize,
    ut: SlotAddr,
    irqc: SlotAddr,
    schedc: SlotAddr,
    dev_frame: BTreeMap<String, SlotAddr>,
    dev_io: BTreeMap<String, SlotAddr>,
}

impl Builder {
    fn new(k: &mut Kernel<Hw>, mem: u64) -> Result<Builder, BootError> {
        let cn = k.objs.insert(Object {
            hdr: Header { refs: 1, ..Header::default() },
            kind: Kind::CNode(CNode::with_bits(BOOT_CNODE_BITS)),
        });
        let dummy = SlotAddr { cnode: cn, index: 0 };
        let mut b = Builder {
            cn,
            next: 1,
            ut: dummy,
            irqc: dummy,
            schedc: dummy,
            dev_frame: BTreeMap::new(),
            dev_io: BTreeMap::new(),
        };
        let ut = k.objs.insert(Object {
            hdr: Header { paddr: RAM_PHYS_BASE + KERNEL_RESERVED, ..Header::default() },
            kind: Kind::Untyped(vifast_kernel::Untyped { size: mem, ..Default::default() }),
        });
        b.ut = b.put(k, Cap::new(ObjType::Untyped, ut, Rights::ALL))?;
        let irqc = k.objs.insert(Object::new(Kind::IrqControl));
        b.irqc = b.put(k, Cap::new(ObjType::IrqControl, irqc, Rights::ALL))?;
        let schedc = k.objs.insert(Object::new(Kind::SchedControl));
        b.schedc = b.put(k, Cap::new(ObjType::SchedControl, schedc, Rights::ALL))?;
        Ok(b)
    }

    fn fresh(&mut self) -> Result<SlotAddr, BootError> {
        if self.next >= (1usize << BOOT_CNODE_BITS) {
            return err("hết slot trong CNode boot");
        }
        let a = SlotAddr { cnode: self.cn, index: self.next };
        self.next += 1;
        Ok(a)
    }

    fn put(&mut self, k: &mut Kernel<Hw>, cap: Cap) -> Result<SlotAddr, BootError> {
        let a = self.fresh()?;
        k.insert_cap(a, cap, None)?;
        Ok(a)
    }

    fn cap(&self, k: &Kernel<Hw>, a: SlotAddr) -> Result<Cap, BootError> {
        k.cap_at(a).ok_or_else(|| BootError("slot boot rỗng".into()))
    }

    fn obj(&self, k: &Kernel<Hw>, a: SlotAddr) -> Result<ObjRef, BootError> {
        self.cap(k, a)?.obj.ok_or_else(|| BootError("cap boot không trỏ tới object".into()))
    }

    fn new_obj(&mut self, k: &mut Kernel<Hw>, ty: ObjType, bits: u32) -> Result<SlotAddr, BootError> {
        let dest = self.fresh()?;
        let ucap = self.cap(k, self.ut)?;
        k.retype_objects(self.ut, ucap, ty, bits, &[dest])?;
        Ok(dest)
    }

    /// Map frame, tự tạo page table trung gian khi thiếu (việc của root task).
    fn map(&mut self, k: &mut Kernel<Hw>, faddr: SlotAddr, vs: ObjRef, vaddr: u64, rights: Rights) -> Result<(), BootError> {
        loop {
            let fcap = self.cap(k, faddr)?;
            match k.map_frame(faddr, fcap, vs, vaddr, rights) {
                Ok(()) => return Ok(()),
                Err(KError::MissingPageTable) => {}
                Err(e) => return Err(e.into()),
            }
            let pt = self.new_obj(k, ObjType::PageTable, 0)?;
            let pcap = self.cap(k, pt)?;
            k.map_page_table(pt, pcap, vs, vaddr)?;
        }
    }
}

struct TaskInfo {
    name: String,
    spec: Table,
    cnode: SlotAddr,
    tcb: SlotAddr,
    vspace: Option<SlotAddr>,
}

fn obj_kind(s: &str) -> Result<ObjType, BootError> {
    match s {
        "endpoint" => Ok(ObjType::Endpoint),
        "notification" => Ok(ObjType::Notification),
        "frame" => Ok(ObjType::Frame),
        _ => err(format!("loại object không hợp lệ: {}", s)),
    }
}

fn build_inner(machine: &mut Machine, m: &Table, profile: &str, mem: u64) -> Result<(), BootError> {
    let mut b = Builder::new(&mut machine.kernel, mem)?;

    // Thiết bị: frame MMIO + IOSpace riêng cho mỗi thiết bị.
    for d in tables(m, "devices")? {
        let name = req_str(d, "name")?.to_string();
        let kind = req_str(d, "kind")?;
        if kind != "dma_sensor" {
            return err(format!("thiết bị không hỗ trợ: {}", kind));
        }
        let mmio = get_u64(d, "mmio", 0)?;
        let irq = get_u64(d, "irq", 0)? as u32;
        let mut dev = DmaSensor::new(&name, mmio, irq);
        let k = &mut machine.kernel;
        let fr = k.objs.insert(Object {
            hdr: Header { paddr: mmio, ..Header::default() },
            kind: Kind::Frame(Frame { device: true }),
        });
        let fslot = b.put(k, Cap::new(ObjType::Frame, fr, Rights::RW))?;
        b.dev_frame.insert(name.clone(), fslot);
        let io = k.objs.insert(Object::new(Kind::IoSpace(IoSpace::default())));
        let ioslot = b.put(k, Cap::new(ObjType::IoSpace, io, Rights::ALL))?;
        b.dev_io.insert(name.clone(), ioslot);
        dev.iospace = Some(io);
        machine.devices.push(dev);
    }

    let k = &mut machine.kernel;

    // Object dùng chung.
    let mut objects: BTreeMap<String, SlotAddr> = BTreeMap::new();
    if let Some(Value::Table(objs)) = m.get("objects") {
        for (name, v) in objs {
            let kind = v.as_str().ok_or_else(|| BootError(format!("object '{}' phải là chuỗi", name)))?;
            let slot = b.new_obj(k, obj_kind(kind)?, 0)?;
            objects.insert(name.clone(), slot);
        }
    }

    // Task: CNode + TCB, rồi VSpace (có thể dùng chung).
    let mut tasks: Vec<TaskInfo> = Vec::new();
    for spec in tables(m, "tasks")? {
        let name = req_str(spec, "name")?.to_string();
        if tasks.iter().any(|t| t.name == name) || objects.contains_key(&name) {
            return err(format!("trùng tên: {}", name));
        }
        let cnode = b.new_obj(k, ObjType::CNode, get_u64(spec, "cnode_bits", 6)? as u32)?;
        let tcb = b.new_obj(k, ObjType::Tcb, 0)?;
        tasks.push(TaskInfo { name, spec: spec.clone(), cnode, tcb, vspace: None });
    }
    for i in 0..tasks.len() {
        if get_str(&tasks[i].spec, "share_vspace").is_none() {
            tasks[i].vspace = Some(b.new_obj(k, ObjType::VSpace, 0)?);
        }
    }
    for i in 0..tasks.len() {
        if let Some(other) = get_str(&tasks[i].spec, "share_vspace").map(|s| s.to_string()) {
            let vs = tasks
                .iter()
                .find(|t| t.name == other)
                .and_then(|t| t.vspace)
                .ok_or_else(|| BootError(format!("share_vspace không hợp lệ: {}", other)))?;
            tasks[i].vspace = Some(vs);
        }
    }

    let image_ids: Vec<(String, u32)> =
        k.image_names.iter().enumerate().map(|(i, n)| (n.clone(), i as u32)).collect();

    for i in 0..tasks.len() {
        let (name, spec, cnode_slot, tcb_slot, vs_slot) = {
            let t = &tasks[i];
            (t.name.clone(), t.spec.clone(), t.cnode, t.tcb, t.vspace.expect("vspace đã gán"))
        };
        let cn_ref = b.obj(k, cnode_slot)?;
        let vs_ref = b.obj(k, vs_slot)?;
        let pages = if get_str(&spec, "share_vspace").is_some() { 0 } else { get_u64(&spec, "ram_pages", 1)? };
        for p in 0..pages {
            let f = b.new_obj(k, ObjType::Frame, 0)?;
            b.map(k, f, vs_ref, RAM_BASE + p * PAGE_SIZE, Rights::RW)?;
        }
        let mut env: Env = Env::new();
        env.insert("ram_base".into(), RAM_BASE);
        env.insert("ram_pages".into(), pages);
        for (img, id) in &image_ids {
            env.insert(format!("img_{}", img), *id as u64);
        }
        let mut used = Vec::new();
        for c in tables(&spec, "caps")? {
            let cname = req_str(c, "name")?.to_string();
            let slot = get_u64(c, "slot", 0)? as usize;
            if slot == 0 || used.contains(&slot) {
                return err(format!("{}: slot {} trùng hoặc bằng 0", name, slot));
            }
            used.push(slot);
            let dest = SlotAddr { cnode: cn_ref, index: slot };
            grant(k, &mut b, profile, &name, c, dest, &objects, &tasks, i, vs_ref, &mut env)?;
            env.insert(cname, slot as u64);
        }
        if let Some(Value::Table(extra)) = spec.get("env") {
            for (key, v) in extra {
                env.insert(key.clone(), as_u64(v, key)?);
            }
        }
        let (fault_ep, fault_badge) = match spec.get("fault").and_then(|v| v.as_table()) {
            Some(f) => {
                let oname = req_str(f, "obj")?;
                let slot = *objects.get(oname).ok_or_else(|| BootError(format!("không có object {}", oname)))?;
                (Some(b.obj(k, slot)?), get_u64(f, "badge", 0)?)
            }
            None => (None, 0),
        };
        let prio = get_u64(&spec, "priority", 100)? as u32;
        let image_name = req_str(&spec, "image")?;
        let image = image_ids
            .iter()
            .find(|(n, _)| n == image_name)
            .map(|(_, id)| *id)
            .ok_or_else(|| BootError(format!("{}: không có image '{}'", name, image_name)))?;
        let tref = b.obj(k, tcb_slot)?;
        let radix = k.objs.cnode(cn_ref)?.radix();
        let cfg = TcbConfig {
            name: name.clone(),
            image,
            priority: prio,
            mcp: get_u64(&spec, "mcp", prio as u64)? as u32,
            cspace: cn_ref,
            guard: 0,
            guard_bits: WORD_BITS - radix,
            vspace: vs_ref,
            fault_ep,
            fault_badge,
            args: [0; NUM_ARGS],
            affinity: get_u64(&spec, "core", 0)? as usize,
        };
        k.configure_tcb(tref, &cfg)?;
        if let Some(nname) = get_str(&spec, "bind_notification") {
            let slot = *objects.get(nname).ok_or_else(|| BootError(format!("không có object {}", nname)))?;
            let n = b.obj(k, slot)?;
            k.bind_notification(tref, n)?;
        }
        if spec.contains_key("budget") {
            let sc = b.new_obj(k, ObjType::SchedContext, 0)?;
            let sc_ref = b.obj(k, sc)?;
            k.configure_sc(sc_ref, get_u64(&spec, "budget", 0)?, get_u64(&spec, "period", 0)?)?;
            k.bind_sc(sc_ref, tref)?;
        }
        machine.boot_env.insert(tref, env);
    }

    for t in &tasks {
        let start = t.spec.get("start").and_then(|v| v.as_bool()).unwrap_or(true);
        if start {
            let tref = b.obj(k, t.tcb)?;
            k.make_ready(tref, None);
        }
    }

    if profile == "space" {
        // Khoá hệ thống: không còn untyped, IRQ control hay sched control ở đâu cả.
        for slot in [b.schedc, b.irqc, b.ut] {
            k.delete_slot(slot);
        }
    }
    Ok(())
}

/// Cấp một capability cho task theo một dòng `caps` trong manifest.
#[allow(clippy::too_many_arguments)]
fn grant(
    k: &mut Kernel<Hw>,
    b: &mut Builder,
    profile: &str,
    task: &str,
    c: &Table,
    dest: SlotAddr,
    objects: &BTreeMap<String, SlotAddr>,
    tasks: &[TaskInfo],
    me: usize,
    vs_ref: ObjRef,
    env: &mut Env,
) -> Result<(), BootError> {
    let rights = match get_str(c, "rights") {
        Some(r) => Rights::parse(r).ok_or_else(|| BootError(format!("{}: quyền không hợp lệ {}", task, r)))?,
        None => Rights::ALL,
    };
    let cname = req_str(c, "name")?.to_string();

    if let Some(oname) = get_str(c, "obj") {
        let src = *objects.get(oname).ok_or_else(|| BootError(format!("không có object {}", oname)))?;
        let derived = b.cap(k, src)?.derive(rights, get_u64(c, "badge", 0)?)?;
        k.insert_cap(dest, derived, Some(src))?;
        if c.contains_key("map") {
            let vaddr = get_u64(c, "map", 0)?;
            b.map(k, dest, vs_ref, vaddr, rights)?;
            env.insert(format!("{}_addr", cname), vaddr);
        }
        return Ok(());
    }
    if let Some(dname) = get_str(c, "device") {
        let src = *b.dev_frame.get(dname).ok_or_else(|| BootError(format!("không có thiết bị {}", dname)))?;
        let derived = b.cap(k, src)?.derive(rights, 0)?;
        k.insert_cap(dest, derived, Some(src))?;
        if c.contains_key("map") {
            let vaddr = get_u64(c, "map", 0)?;
            b.map(k, dest, vs_ref, vaddr, rights & Rights::RW)?;
            env.insert(format!("{}_addr", cname), vaddr);
        }
        return Ok(());
    }
    if let Some(dname) = get_str(c, "iospace") {
        let src = *b.dev_io.get(dname).ok_or_else(|| BootError(format!("không có thiết bị {}", dname)))?;
        let derived = b.cap(k, src)?.derive(rights, 0)?;
        k.insert_cap(dest, derived, Some(src))?;
        return Ok(());
    }
    for key in ["tcb", "cnode", "vspace"] {
        if let Some(other) = get_str(c, key) {
            let t = tasks
                .iter()
                .find(|t| t.name == other)
                .ok_or_else(|| BootError(format!("không có task {}", other)))?;
            let src = match key {
                "tcb" => t.tcb,
                "cnode" => t.cnode,
                _ => t.vspace.ok_or_else(|| BootError("vspace chưa có".into()))?,
            };
            let derived = b.cap(k, src)?.derive(rights, 0)?;
            k.insert_cap(dest, derived, Some(src))?;
            return Ok(());
        }
    }
    if let Some(which) = get_str(c, "self") {
        let t = &tasks[me];
        let src = match which {
            "tcb" => t.tcb,
            "cnode" => t.cnode,
            "vspace" => t.vspace.ok_or_else(|| BootError("vspace chưa có".into()))?,
            _ => return err(format!("{}: self không hợp lệ {}", task, which)),
        };
        let derived = b.cap(k, src)?.derive(rights, 0)?;
        k.insert_cap(dest, derived, Some(src))?;
        return Ok(());
    }
    if c.contains_key("irq") {
        let irq = get_u64(c, "irq", 0)? as u32;
        let irq_slot = b.fresh()?;
        k.create_irq_handler(b.irqc, irq, irq_slot)?;
        let derived = b.cap(k, irq_slot)?.derive(Rights::ALL, 0)?;
        k.insert_cap(dest, derived, Some(irq_slot))?;
        if let Some(nname) = get_str(c, "notification") {
            let nslot = *objects.get(nname).ok_or_else(|| BootError(format!("không có object {}", nname)))?;
            let ncap = b.cap(k, nslot)?;
            k.bind_irq(irq, ncap)?;
        }
        return Ok(());
    }
    let wants_untyped = c.contains_key("untyped_bits");
    let wants_irqc = c.contains_key("irq_control");
    let wants_schedc = c.contains_key("sched_control");
    if wants_untyped || wants_irqc || wants_schedc {
        if profile == "space" {
            return err(format!("{}: profile space không cho phép cấp untyped/irq_control/sched_control", task));
        }
        let src = if wants_untyped {
            b.new_obj(k, ObjType::Untyped, get_u64(c, "untyped_bits", 12)? as u32)?
        } else if wants_irqc {
            b.irqc
        } else {
            b.schedc
        };
        let derived = b.cap(k, src)?.derive(Rights::ALL, 0)?;
        k.insert_cap(dest, derived, Some(src))?;
        return Ok(());
    }
    err(format!("{}: không hiểu cap {}", task, cname))
}
