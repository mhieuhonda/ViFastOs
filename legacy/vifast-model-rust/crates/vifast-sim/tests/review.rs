//! Các thử nghiệm đánh giá thêm, không thuộc bộ test gốc.
use vifast_abi::*;
use vifast_sim::{build_str, default_images, program, Images, Machine, Sys};

fn boot(manifest: &str, extra: impl FnOnce(&mut Images)) -> Machine {
    let mut im = default_images();
    extra(&mut im);
    build_str(manifest, im, false).unwrap()
}

async fn server_two_recv(s: Sys) -> Result<(), String> {
    let ep = s.env("svc")?;
    let a = s.recv(ep).await;
    s.print(format!("recv A {}", a.err.name())).await;
    let b = s.recv(ep).await;
    s.print(format!("recv B {}", b.err.name())).await;
    let r1 = s.reply(Message::empty(1)).await;
    s.print(format!("reply1 {}", r1.err.name())).await;
    let r2 = s.reply(Message::empty(2)).await;
    s.print(format!("reply2 {}", r2.err.name())).await;
    Ok(())
}

async fn client_call(s: Sys, label: &'static str) -> Result<(), String> {
    let r = s.call(s.env("svc")?, Message::empty(0)).await;
    s.print(format!("{} {}", label, r.err.name())).await;
    Ok(())
}

#[test]
fn multiple_callers_overwrite_reply_slot() {
    let manifest = r#"
[objects]
svc = "endpoint"
[[tasks]]
name = "server"
image = "server"
priority = 100
caps = [{ name = "svc", slot = 1, obj = "svc", rights = "R" }]
[[tasks]]
name = "a"
image = "a"
priority = 90
caps = [{ name = "svc", slot = 1, obj = "svc", rights = "W" }]
[[tasks]]
name = "b"
image = "b"
priority = 80
caps = [{ name = "svc", slot = 1, obj = "svc", rights = "W" }]
"#;
    let mut m = boot(manifest, |im| {
        im.register("server", &[], |s| program(server_two_recv(s)));
        im.register("a", &[], |s| program(client_call(s, "A")));
        im.register("b", &[], |s| program(client_call(s, "B")));
    });
    m.run(200);
    println!("server={:?} a={:?} b={:?}", m.lines("server"), m.lines("a"), m.lines("b"));
    println!("a state={:?} b state={:?}", m.find_tcb("a").unwrap().1.state, m.find_tcb("b").unwrap().1.state);
}

async fn server_hold_caller(s: Sys) -> Result<(), String> {
    let ep = s.env("svc")?;
    let a = s.recv(ep).await;
    s.print(format!("recv A {}", a.err.name())).await;
    s.signal(s.env("ready")?).await;
    s.wait(s.env("resume")?).await;
    Ok(())
}

async fn supervisor_suspend(s: Sys) -> Result<(), String> {
    s.wait(s.env("ready")?).await;
    let r = s.sys(Syscall::TcbSuspend { tcb: s.env("server_tcb")? }).await;
    s.print(format!("suspend {}", r.err.name())).await;
    Ok(())
}

async fn client_a(s: Sys) -> Result<(), String> {
    let r = s.call(s.env("svc")?, Message::empty(0)).await;
    s.print(format!("A {}", r.err.name())).await;
    Ok(())
}

#[test]
fn suspend_server_does_not_release_waiting_client() {
    let manifest = r#"
[objects]
svc = "endpoint"
ready = "notification"
resume = "notification"
[[tasks]]
name = "server"
image = "server"
priority = 100
caps = [
  { name = "svc", slot = 1, obj = "svc", rights = "R" },
  { name = "ready", slot = 2, obj = "ready", rights = "W" },
  { name = "resume", slot = 3, obj = "resume", rights = "R" },
]
[[tasks]]
name = "client"
image = "client"
priority = 90
caps = [{ name = "svc", slot = 1, obj = "svc", rights = "W" }]
[[tasks]]
name = "sup"
image = "sup"
priority = 110
caps = [
  { name = "ready", slot = 1, obj = "ready", rights = "R" },
  { name = "server_tcb", slot = 2, tcb = "server" },
]
"#;
    let mut m = boot(manifest, |im| {
        im.register("server", &[], |s| program(server_hold_caller(s)));
        im.register("client", &[], |s| program(client_a(s)));
        im.register("sup", &[], |s| program(supervisor_suspend(s)));
    });
    m.run(200);
    println!("sup={:?} client={:?} server_state={:?} client_state={:?}", m.lines("sup"), m.lines("client"), m.find_tcb("server").unwrap().1.state, m.find_tcb("client").unwrap().1.state);
}

async fn wx_alias(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    let vs = s.env("vs")?;
    for (slot, ty, bits) in [(10, ObjType::Frame, 0), (11, ObjType::PageTable, 0), (12, ObjType::PageTable, 0)] {
        let r = s.retype(mem, ty, bits, slot).await;
        s.print(format!("retype {} {}", slot, r.err.name())).await;
    }
    let c = s.cap_copy(10, 13, 0).await;
    s.print(format!("copy {}", c.err.name())).await;
    for (pt, addr) in [(11u64, 0x4000_0000u64), (12, 0x4000_0000)] {
        let r = s.sys(Syscall::PageTableMap { page_table: pt, vspace: vs, vaddr: addr }).await;
        s.print(format!("ptmap {} {}", pt, r.err.name())).await;
    }
    let a = s.sys(Syscall::FrameMap { frame: 10, vspace: vs, vaddr: 0x4000_0000, rights: Rights::RW }).await;
    s.print(format!("map RW {}", a.err.name())).await;
    let b = s.sys(Syscall::FrameMap { frame: 13, vspace: vs, vaddr: 0x4010_0000, rights: Rights::READ | Rights::EXEC }).await;
    s.print(format!("map RX {}", b.err.name())).await;
    s.store(0x4000_0000, b"ABCDEFGH").await;
    let d = s.fetch(0x4010_0000).await;
    s.print(format!("fetch {:?}", d)).await;
    Ok(())
}

#[test]
fn wx_alias_physical_page() {
    let manifest = r#"
[[tasks]]
name = "p"
image = "p"
caps = [
  { name = "mem", slot = 1, untyped_bits = 14 },
  { name = "vs", slot = 2, self = "vspace" },
]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(wx_alias(s))));
    m.run(200);
    println!("{:?}", m.lines("p"));
}

async fn pt_double_map(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    let vs = s.env("vs")?;
    for (slot, ty, bits) in [(10, ObjType::Frame, 0), (11, ObjType::PageTable, 0), (12, ObjType::PageTable, 0)] {
        let r = s.retype(mem, ty, bits, slot).await;
        s.print(format!("retype {} {}", slot, r.err.name())).await;
    }
    for (pt, addr, label) in [(11u64, 0x4000_0000u64, "pt11@0x4000"), (12, 0x4000_0000, "pt12@0x4000")] {
        let r = s.sys(Syscall::PageTableMap { page_table: pt, vspace: vs, vaddr: addr }).await;
        s.print(format!("{} {}", label, r.err.name())).await;
    }
    let c = s.cap_copy(11, 13, 0).await;
    s.print(format!("copy pt11 {}", c.err.name())).await;
    let d = s.sys(Syscall::PageTableMap { page_table: 13, vspace: vs, vaddr: 0x8000_0000 }).await;
    s.print(format!("pt13@0x8000 {}", d.err.name())).await;
    let e = s.sys(Syscall::FrameMap { frame: 10, vspace: vs, vaddr: 0x4000_0000, rights: Rights::RW }).await;
    s.print(format!("map frame {}", e.err.name())).await;
    s.store(0x4000_0000, b"ALIAS").await;
    let out = s.load(0x8000_0000, 5).await;
    s.print(format!("load alias {:?}", out)).await;
    Ok(())
}

#[test]
fn page_table_double_map_alias() {
    let manifest = r#"
[[tasks]]
name = "p"
image = "p"
caps = [
  { name = "mem", slot = 1, untyped_bits = 14 },
  { name = "vs", slot = 2, self = "vspace" },
]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(pt_double_map(s))));
    m.run(200);
    println!("{:?}", m.lines("p"));
}

async fn zero_right_vspace(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    let img = s.env("img_p")?;
    let c = s.sys(Syscall::CapCopy { src: s.env("vs")?, dest_index: 10, rights: Rights::NONE, dest_cnode: 0 }).await;
    s.print(format!("copy NONE {}", c.err.name())).await;
    let r = s.retype(mem, ObjType::Tcb, 0, 11).await;
    s.print(format!("retype tcb {}", r.err.name())).await;
    let cfg = s.sys(Syscall::TcbConfigure {
        tcb: 11,
        cspace: s.env("cn")?,
        vspace: 10,
        priority: 50,
        mcp: 50,
        image: img as u32,
        args: [0; 4],
        fault_ep: 0,
    }).await;
    s.print(format!("configure {}", cfg.err.name())).await;
    Ok(())
}

#[test]
fn zero_rights_vspace_cap_can_configure_tcb() {
    let manifest = r#"
[[tasks]]
name = "p"
image = "p"
caps = [
  { name = "mem", slot = 1, untyped_bits = 14 },
  { name = "vs", slot = 2, self = "vspace" },
  { name = "cn", slot = 3, self = "cnode" },
]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(zero_right_vspace(s))));
    m.run(200);
    println!("{:?}", m.lines("p"));
}

async fn server_exit_without_reply(s: Sys) -> Result<(), String> {
    let ep = s.env("svc")?;
    let a = s.recv(ep).await;
    s.print(format!("recv {}", a.err.name())).await;
    Ok(())
}

async fn client_wait_forever(s: Sys) -> Result<(), String> {
    let r = s.call(s.env("svc")?, Message::empty(0)).await;
    s.print(format!("call {}", r.err.name())).await;
    Ok(())
}

#[test]
fn exited_server_object_is_pinned_by_boot_cap() {
    let manifest = r#"
[objects]
svc = "endpoint"
[[tasks]]
name = "server"
image = "server"
priority = 100
caps = [{ name = "svc", slot = 1, obj = "svc", rights = "R" }]
[[tasks]]
name = "client"
image = "client"
priority = 90
caps = [{ name = "svc", slot = 1, obj = "svc", rights = "W" }]
"#;
    let mut m = boot(manifest, |im| {
        im.register("server", &[], |s| program(server_exit_without_reply(s)));
        im.register("client", &[], |s| program(client_wait_forever(s)));
    });
    m.run(200);
    let server_alive = m.find_tcb("server").is_some();
    println!("server={:?} client={:?} server_alive={} client_state={:?}", m.lines("server"), m.lines("client"), server_alive, m.find_tcb("client").unwrap().1.state);
}

async fn device_exec(s: Sys) -> Result<(), String> {
    let f = s.env("cam_regs")?;
    let vs = s.env("vs")?;
    let u = s.sys(Syscall::FrameUnmap { frame: f }).await;
    s.print(format!("unmap {}", u.err.name())).await;
    let m = s.sys(Syscall::FrameMap { frame: f, vspace: vs, vaddr: 0x5000_0000, rights: Rights::READ | Rights::EXEC }).await;
    s.print(format!("map RX {}", m.err.name())).await;
    let d = s.fetch(0x5000_0000).await;
    s.print(format!("fetch len {}", d.len())).await;
    Ok(())
}

#[test]
fn device_mmio_can_be_mapped_executable() {
    let manifest = r#"
profile = "desktop"
[[devices]]
name = "camera"
kind = "dma_sensor"
mmio = 0x10000000
irq = 7
[[tasks]]
name = "p"
image = "p"
caps = [
  { name = "cam_regs", slot = 1, device = "camera", rights = "RWXG", map = 0x50000000 },
  { name = "vs", slot = 2, self = "vspace" },
]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(device_exec(s))));
    m.run(200);
    println!("{:?}", m.lines("p"));
}

async fn retype_rights(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    let vs = s.env("vs")?;
    for (slot, ty, bits) in [(10, ObjType::Frame, 0), (11, ObjType::PageTable, 0), (12, ObjType::PageTable, 0)] {
        let r = s.retype(mem, ty, bits, slot).await;
        s.print(format!("retype {} {}", slot, r.err.name())).await;
    }
    for pt in [11u64, 12] {
        let r = s.sys(Syscall::PageTableMap { page_table: pt, vspace: vs, vaddr: 0x4000_0000 }).await;
        s.print(format!("ptmap {} {}", pt, r.err.name())).await;
    }
    let m = s.sys(Syscall::FrameMap { frame: 10, vspace: vs, vaddr: 0x4000_0000, rights: Rights::READ | Rights::EXEC }).await;
    s.print(format!("map RX {}", m.err.name())).await;
    let d = s.fetch(0x4000_0000).await;
    s.print(format!("fetch {}", d.len())).await;
    Ok(())
}

#[test]
fn retype_ignores_untyped_rights() {
    let manifest = r#"
[[tasks]]
name = "p"
image = "p"
caps = [
  { name = "mem", slot = 1, untyped_bits = 14, rights = "W" },
  { name = "vs", slot = 2, self = "vspace" },
]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(retype_rights(s))));
    m.run(200);
    println!("{:?}", m.lines("p"));
}
