//! Bộ test hành vi kernel, port 1-1 từ `python-model/tests/test_kernel.py`.
//! Chạy: `cargo test -p vifast-sim`.

use toml::Value;
use vifast_abi::*;
use vifast_kernel::vm::{self, PTE_U, PTE_V};
use vifast_kernel::Kind;
use vifast_sim::hw::RAM_PHYS_BASE;
use vifast_sim::{build, build_str, default_images, parse_manifest, program, Images, Machine, Sys, DESKTOP_MANIFEST, SPACE_MANIFEST};

// ---------------------------------------------------------------------------
// tiện ích
// ---------------------------------------------------------------------------

fn boot(manifest: &str, extra: impl FnOnce(&mut Images)) -> Machine {
    let mut im = default_images();
    extra(&mut im);
    match build_str(manifest, im, false) {
        Ok(m) => m,
        Err(e) => panic!("boot lỗi: {}", e),
    }
}

async fn spin(s: Sys, label: &'static str, n: u32) -> Result<(), String> {
    for i in 0..n {
        s.print(format!("{}{}", label, i)).await;
    }
    Ok(())
}

fn words(r: &SysResult) -> Vec<u64> {
    r.msg.map(|m| m.words().to_vec()).unwrap_or_default()
}

// ===========================================================================
// Demo
// ===========================================================================

#[test]
fn space_profile() {
    let mut m = boot(SPACE_MANIFEST, |_| {});
    m.schedule_irq(5, 2, 8);
    m.run(1500);
    assert!(m.lines("attitude").contains(&"hoàn thành điều khiển tư thế".to_string()));
    let sup = m.lines("supervisor");
    assert_eq!(sup.iter().filter(|x| x.contains("restart payload")).count(), 3);
    assert!(sup.iter().any(|x| x.contains("DEGRADED: payload")));
    assert_eq!(m.lines("consumer").len(), 3);
    let samples: Vec<u64> = m
        .lines("attitude")
        .iter()
        .filter(|x| x.contains("mẫu IMU"))
        .map(|x| x.rsplit("= ").next().unwrap().parse().unwrap())
        .collect();
    let mut sorted = samples.clone();
    sorted.sort();
    assert_eq!(samples, sorted);
    assert!(*samples.last().unwrap() > 10);
}

#[test]
fn space_is_static() {
    let m = boot(SPACE_MANIFEST, |_| {});
    for (_, o) in m.kernel.objs.iter() {
        if let Kind::CNode(cn) = &o.kind {
            for s in &cn.slots {
                if let Some(c) = s.cap {
                    assert!(!matches!(c.obj_type, ObjType::Untyped | ObjType::IrqControl | ObjType::SchedControl));
                }
            }
        }
    }
}

#[test]
fn space_rejects_untyped() {
    let manifest = r#"
profile = "space"
[[tasks]]
name = "a"
image = "hello"
caps = [{ name = "mem", slot = 1, untyped_bits = 12 }]
"#;
    assert!(build_str(manifest, default_images(), false).is_err());
}

#[test]
fn desktop_profile() {
    let mut m = boot(DESKTOP_MANIFEST, |_| {});
    m.run(2000);
    assert_eq!(m.lines("console").iter().filter(|x| x.starts_with("[badge 7]")).count(), 3);
    assert_eq!(m.lines("pager").iter().filter(|x| x.contains("cấp trang")).count(), 3);
    assert!(m.lines("lazy").contains(&"đọc lại: trang 0".to_string()));
    let expected: Vec<String> = (1..=3).map(|i| format!("DMA xong (status=2): SENSOR-{}", i)).collect();
    assert_eq!(m.lines("cam_driver"), expected);
    assert!(m.kernel_canary_intact());
}

// ===========================================================================
// Capability
// ===========================================================================

async fn cap_server(s: Sys) -> Result<(), String> {
    let ep = s.env("svc")?;
    loop {
        s.recv(ep).await;
        s.reply(Message::new(0, &[42])).await;
    }
}

async fn cap_owner(s: Sys) -> Result<(), String> {
    s.send(s.env("handoff")?, Message::empty(0).with_cap(s.env("svc")?)).await;
    s.wait(s.env("done")?).await;
    s.sys(Syscall::CapRevoke { slot: s.env("svc")? }).await;
    s.signal(s.env("revoked")?).await;
    Ok(())
}

async fn cap_client(s: Sys) -> Result<(), String> {
    let r = s.recv_into(s.env("handoff")?, 5).await;
    let slot = r.msg.and_then(|m| m.cap).ok_or("không nhận được cap")?;
    let a = s.call(slot, Message::empty(1)).await;
    s.print(format!("trước: {} {:?}", a.err.name(), words(&a))).await;
    s.signal(s.env("done")?).await;
    s.wait(s.env("revoked")?).await;
    let b = s.call(slot, Message::empty(1)).await;
    s.print(format!("sau: {}", b.err.name())).await;
    Ok(())
}

#[test]
fn cap_transfer_and_revoke() {
    let manifest = r#"
[objects]
svc = "endpoint"
handoff = "endpoint"
done = "notification"
revoked = "notification"

[[tasks]]
name = "server"
image = "server"
priority = 100
caps = [{ name = "svc", slot = 1, obj = "svc", rights = "R" }]

[[tasks]]
name = "owner"
image = "owner"
priority = 90
caps = [
  { name = "svc", slot = 1, obj = "svc", rights = "W" },
  { name = "handoff", slot = 2, obj = "handoff", rights = "WG" },
  { name = "done", slot = 3, obj = "done", rights = "R" },
  { name = "revoked", slot = 4, obj = "revoked", rights = "W" },
]

[[tasks]]
name = "client"
image = "client"
priority = 80
caps = [
  { name = "handoff", slot = 1, obj = "handoff", rights = "R" },
  { name = "done", slot = 2, obj = "done", rights = "W" },
  { name = "revoked", slot = 3, obj = "revoked", rights = "R" },
]
"#;
    let mut m = boot(manifest, |im| {
        im.register("server", &[], |s| program(cap_server(s)));
        im.register("owner", &[], |s| program(cap_owner(s)));
        im.register("client", &[], |s| program(cap_client(s)));
    });
    m.run(500);
    assert_eq!(m.lines("client"), vec!["trước: OK [42]", "sau: INVALID_CAP"]);
}

async fn hog_mem(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    let mut n = 0;
    while s.retype(mem, ObjType::Frame, 0, 10 + n).await.is_ok() {
        n += 1;
    }
    s.print(format!("cấp được {} trang", n)).await;
    s.sys(Syscall::CapRevoke { slot: mem }).await;
    let r = s.retype(mem, ObjType::Frame, 0, 10).await;
    s.print(format!("sau revoke: {}", r.err.name())).await;
    Ok(())
}

#[test]
fn untyped_exhaustion_and_reclaim() {
    let manifest = r#"
[[tasks]]
name = "hog"
image = "hog"
priority = 100
cnode_bits = 5
caps = [{ name = "mem", slot = 1, untyped_bits = 14 }]

[[tasks]]
name = "bystander"
image = "bystander"
priority = 100
"#;
    let mut m = boot(manifest, |im| {
        im.register("hog", &[], |s| program(hog_mem(s)));
        im.register("bystander", &[], |s| program(spin(s, "B", 3)));
    });
    m.run(500);
    assert_eq!(m.lines("hog"), vec!["cấp được 4 trang", "sau revoke: OK"]);
    assert_eq!(m.lines("bystander"), vec!["B0", "B1", "B2"]);
}

async fn zero_check(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    let vs = s.env("vs")?;
    for round in 0..2 {
        s.retype(mem, ObjType::Frame, 0, 10).await;
        s.retype(mem, ObjType::PageTable, 0, 11).await;
        s.retype(mem, ObjType::PageTable, 0, 12).await;
        s.sys(Syscall::PageTableMap { page_table: 11, vspace: vs, vaddr: 0x4000_0000 }).await;
        s.sys(Syscall::PageTableMap { page_table: 12, vspace: vs, vaddr: 0x4000_0000 }).await;
        s.sys(Syscall::FrameMap { frame: 10, vspace: vs, vaddr: 0x4000_0000, rights: Rights::RW }).await;
        if round == 0 {
            s.store(0x4000_0000, b"BI-MAT").await;
            s.sys(Syscall::CapRevoke { slot: mem }).await;
        }
    }
    let data = s.load(0x4000_0000, 6).await;
    s.print(format!("{:?}", data)).await;
    Ok(())
}

/// Bộ nhớ thu hồi rồi cấp lại phải được xoá trắng (không rò dữ liệu cũ).
#[test]
fn retype_zeroes_memory() {
    let manifest = r#"
[[tasks]]
name = "p"
image = "p"
caps = [
  { name = "mem", slot = 1, untyped_bits = 14 },
  { name = "vs", slot = 2, self = "vspace" },
]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(zero_check(s))));
    m.run(200);
    assert_eq!(m.lines("p"), vec!["[0, 0, 0, 0, 0, 0]"]);
}

async fn ml_spawner(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    for (slot, ty, bits) in [(10, ObjType::CNode, 4), (11, ObjType::CNode, 8), (12, ObjType::Tcb, 0), (13, ObjType::VSpace, 0)] {
        s.retype(mem, ty, bits, slot).await;
    }
    s.sys(Syscall::CNodeMint { src: 11, dest_index: 2, guard: 0, guard_bits: 0, dest_cnode: 10 }).await;
    s.sys(Syscall::CapMint { src: s.env("console")?, dest_index: 5, rights: Rights::WRITE, badge: 9, dest_cnode: 11 }).await;
    s.sys(Syscall::CNodeMint { src: 10, dest_index: 14, guard: 0, guard_bits: 52, dest_cnode: 0 }).await;
    let r = s
        .sys(Syscall::TcbConfigure {
            tcb: 12,
            cspace: 14,
            vspace: 13,
            priority: 50,
            mcp: 50,
            image: s.env("img_mlclient")? as u32,
            args: [(2 << 8) | 5, 5, 0, 0],
            fault_ep: 0,
        })
        .await;
    s.print(format!("configure {}", r.err.name())).await;
    s.sys(Syscall::TcbResume { tcb: 12 }).await;
    Ok(())
}

async fn ml_client(s: Sys) -> Result<(), String> {
    let r = s.call(s.env("bad")?, Message::empty(1)).await; // cptr 1 cấp không còn hợp lệ
    s.print(format!("cptr phẳng: {}", r.err.name())).await;
    let r = s.call(s.env("ep")?, Message::empty(1)).await;
    s.print(format!("cptr 2 cấp: {} {:?}", r.err.name(), words(&r))).await;
    Ok(())
}

async fn ml_server(s: Sys) -> Result<(), String> {
    let r = s.recv(s.env("ep")?).await;
    s.reply(Message::new(0, &[r.badge])).await;
    Ok(())
}

/// CSpace 2 cấp: gốc radix 4 + guard 52 bit, cấp 2 radix 8. cptr = (2 << 8) | 5.
#[test]
fn multilevel_cspace_with_guard() {
    let manifest = r#"
[objects]
ep = "endpoint"

[[tasks]]
name = "server"
image = "mlserver"
priority = 100
caps = [{ name = "ep", slot = 1, obj = "ep", rights = "R" }]

[[tasks]]
name = "spawner"
image = "spawner"
priority = 90
caps = [
  { name = "mem", slot = 1, untyped_bits = 18 },
  { name = "console", slot = 2, obj = "ep", rights = "WG" },
]
"#;
    let mut m = boot(manifest, |im| {
        im.register("spawner", &[], |s| program(ml_spawner(s)));
        im.register("mlclient", &["ep", "bad"], |s| program(ml_client(s)));
        im.register("mlserver", &[], |s| program(ml_server(s)));
    });
    m.run(300);
    assert!(m.lines("spawner").contains(&"configure OK".to_string()));
    let out: Vec<String> = m.all_lines().into_iter().filter(|x| x.starts_with("cptr")).collect();
    assert_eq!(out, vec!["cptr phẳng: INVALID_CAP", "cptr 2 cấp: OK [9]"]);
}

async fn revoke_many(s: Sys) -> Result<(), String> {
    let ep = s.env("ep")?;
    for i in 0..40 {
        s.cap_copy(ep, 20 + i, 0).await;
    }
    let r = s.sys(Syscall::CapRevoke { slot: ep }).await;
    s.print(format!("revoke {}", r.err.name())).await;
    let r = s.call(20, Message::empty(0)).await;
    s.print(format!("bản copy: {}", r.err.name())).await;
    Ok(())
}

#[test]
fn revoke_is_preemptible() {
    let manifest = r#"
[objects]
ep = "endpoint"

[[tasks]]
name = "p"
image = "p"
cnode_bits = 7
caps = [{ name = "ep", slot = 1, obj = "ep", rights = "RW" }]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(revoke_many(s))));
    m.run(300);
    assert_eq!(m.lines("p"), vec!["revoke OK", "bản copy: INVALID_CAP"]);
    assert_eq!(m.kernel.stats.revoke_preempt, 4); // 40 cap / 8 mỗi lượt
}

async fn zombie(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    s.retype(mem, ObjType::CNode, 6, 10).await;
    s.cap_copy(10, 1, 10).await; // CNode chứa cap trỏ về chính nó
    s.sys(Syscall::CapDelete { slot: 10 }).await;
    s.print("đã xoá cap ngoài").await;
    s.sys(Syscall::CapRevoke { slot: mem }).await;
    for _ in 0..10 {
        s.yield_now().await;
    }
    let r = s.retype(mem, ObjType::Untyped, 14, 11).await;
    s.print(format!("cấp lại toàn bộ: {}", r.err.name())).await;
    Ok(())
}

#[test]
fn zombie_cnode_cycle_is_reclaimed() {
    let manifest = r#"
[[tasks]]
name = "p"
image = "p"
caps = [{ name = "mem", slot = 1, untyped_bits = 14 }]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(zombie(s))));
    m.run(300);
    assert_eq!(m.lines("p"), vec!["đã xoá cap ngoài", "cấp lại toàn bộ: OK"]);
    assert_eq!(m.kernel.stats.zombie_reaped, 1);
}

// ===========================================================================
// Lập lịch
// ===========================================================================

#[test]
fn priority() {
    let manifest = r#"
[[tasks]]
name = "lo"
image = "lo"
priority = 10

[[tasks]]
name = "hi"
image = "hi"
priority = 20
"#;
    let mut m = boot(manifest, |im| {
        im.register("lo", &[], |s| program(spin(s, "L", 3)));
        im.register("hi", &[], |s| program(spin(s, "H", 3)));
    });
    m.run(100);
    let order: Vec<String> = m
        .all_lines()
        .into_iter()
        .filter(|x| x.len() == 2 && (x.starts_with('L') || x.starts_with('H')))
        .collect();
    assert_eq!(order, vec!["H0", "H1", "H2", "L0", "L1", "L2"]);
}

#[test]
fn timer_round_robin() {
    let manifest = r#"
[[tasks]]
name = "a"
image = "a"
priority = 10

[[tasks]]
name = "b"
image = "b"
priority = 10
"#;
    let mut m = boot(manifest, |im| {
        im.register("a", &[], |s| program(spin(s, "A", 12)));
        im.register("b", &[], |s| program(spin(s, "B", 12)));
    });
    m.run(200);
    let seq: String = m
        .all_lines()
        .into_iter()
        .filter_map(|x| x.chars().next().filter(|c| *c == 'A' || *c == 'B'))
        .collect();
    assert!(seq.starts_with("AAAAABBBBB") || seq.starts_with("BBBBBAAAAA"), "seq = {}", seq);
    assert!(m.kernel.harts[0].timer_irqs > 3);
}

async fn mcp_spawner(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    s.retype(mem, ObjType::Tcb, 0, 10).await;
    s.retype(mem, ObjType::CNode, 4, 11).await;
    s.retype(mem, ObjType::VSpace, 0, 12).await;
    let r = s
        .sys(Syscall::TcbConfigure {
            tcb: 10,
            cspace: 11,
            vspace: 12,
            priority: 200,
            mcp: 0,
            image: s.env("img_hello")? as u32,
            args: [0; NUM_ARGS],
            fault_ep: 0,
        })
        .await;
    s.print(r.err.name()).await;
    Ok(())
}

#[test]
fn mcp_limits_priority() {
    let manifest = r#"
[[tasks]]
name = "s"
image = "spawner"
priority = 50
mcp = 50
caps = [{ name = "mem", slot = 1, untyped_bits = 16 }]
"#;
    let mut m = boot(manifest, |im| im.register("spawner", &[], |s| program(mcp_spawner(s))));
    m.run(100);
    assert_eq!(m.lines("s"), vec!["PRIO_TOO_HIGH"]);
}

async fn cpu_hog(s: Sys) -> Result<(), String> {
    let ram = s.env("ram_base")?;
    loop {
        s.load(ram, 8).await;
    }
}

async fn victim(s: Sys) -> Result<(), String> {
    let ram = s.env("ram_base")?;
    for i in 0..1000 {
        s.load(ram, 8).await;
        if i % 10 == 9 {
            s.print("tiến độ").await;
        }
    }
    Ok(())
}

fn hog_vs_victim(with_budget: bool) -> (usize, Machine) {
    let budget = if with_budget { "budget = 2\nperiod = 10\n" } else { "" };
    let manifest = format!(
        r#"
[[tasks]]
name = "hog"
image = "hog"
priority = 200
{}
[[tasks]]
name = "victim"
image = "victim"
priority = 10
"#,
        budget
    );
    let mut m = boot(&manifest, |im| {
        im.register("hog", &[], |s| program(cpu_hog(s)));
        im.register("victim", &[], |s| program(victim(s)));
    });
    m.run(300);
    (m.lines("victim").len(), m)
}

#[test]
fn budget_gives_temporal_isolation() {
    let (without, _) = hog_vs_victim(false);
    let (with_sc, m) = hog_vs_victim(true);
    assert_eq!(without, 0); // ưu tiên cao lặp vô hạn: victim chết đói
    assert!(with_sc > 15, "with_sc = {}", with_sc); // budget 2/10: victim nhận ~80% CPU
    assert!(m.kernel.stats.throttle > 20);
}

async fn worker(s: Sys) -> Result<(), String> {
    let ram = s.env("ram_base")?;
    for _ in 0..20 {
        s.load(ram, 8).await; // lệnh máy, không vào kernel
    }
    s.print("xong").await;
    Ok(())
}

#[test]
fn two_harts_run_in_parallel() {
    let manifest = |harts: u32| {
        format!(
            "harts = {}\n[[tasks]]\nname = \"a\"\nimage = \"w\"\ncore = 0\n[[tasks]]\nname = \"b\"\nimage = \"w\"\ncore = {}\n",
            harts,
            harts - 1
        )
    };
    let reg = |im: &mut Images| im.register("w", &[], |s| program(worker(s)));
    let mut one = boot(&manifest(1), reg);
    one.run(500);
    let mut two = boot(&manifest(2), reg);
    two.run(500);
    assert!((two.ticks() as f64) * 1.6 < one.ticks() as f64, "one={} two={}", one.ticks(), two.ticks());
    assert!(two.kernel.stats.lock_contention <= 1); // chỉ print cuối có thể trùng tick
}

#[test]
fn big_kernel_lock_serializes_syscalls() {
    let manifest = r#"
harts = 2
[[tasks]]
name = "a"
image = "a"
core = 0
[[tasks]]
name = "b"
image = "b"
core = 1
"#;
    let mut m = boot(manifest, |im| {
        im.register("a", &[], |s| program(spin(s, "A", 20)));
        im.register("b", &[], |s| program(spin(s, "B", 20)));
    });
    m.run(500);
    assert!(m.kernel.stats.lock_contention > 15);
    assert_eq!(m.lines("a").len() + m.lines("b").len(), 40);
}

// ===========================================================================
// Bộ nhớ
// ===========================================================================

#[test]
fn sv39_page_tables_live_in_physical_memory() {
    let manifest = r#"
[[tasks]]
name = "a"
image = "hello"
ram_pages = 2
"#;
    let m = boot(manifest, |_| {});
    let (t, _) = m.find_tcb("a").unwrap();
    let (root, _) = m.kernel.address_space(t).unwrap();
    let mem = &m.kernel.plat.mem;
    let pte = vm::walk(|pa: u64| mem.read_u64(pa), root, 0x10_1000).expect("phải có PTE");
    assert!(pte & PTE_V != 0 && pte & PTE_U != 0);
    assert!(vm::pte_pa(pte) >= RAM_PHYS_BASE);
}

async fn wx(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    let vs = s.env("vs")?;
    s.retype(mem, ObjType::Frame, 0, 10).await;
    let rwx = Rights::READ | Rights::WRITE | Rights::EXEC;
    let r = s.sys(Syscall::FrameMap { frame: 10, vspace: vs, vaddr: 0x10_3000, rights: rwx }).await;
    s.print(format!("RWX: {}", r.err.name())).await;
    let r = s.sys(Syscall::FrameMap { frame: 10, vspace: vs, vaddr: 0x10_3000, rights: Rights::READ | Rights::EXEC }).await;
    s.print(format!("RX: {}", r.err.name())).await;
    s.fetch(0x10_3000).await;
    s.print("fetch trang RX được").await;
    s.fetch(s.env("ram_base")?).await; // trang RW: không được thực thi
    s.print("không bao giờ tới đây").await;
    Ok(())
}

#[test]
fn wx_and_exec_permission() {
    let manifest = r#"
[[tasks]]
name = "p"
image = "p"
caps = [
  { name = "mem", slot = 1, untyped_bits = 14 },
  { name = "vs", slot = 2, self = "vspace" },
]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(wx(s))));
    m.run(100);
    assert_eq!(m.lines("p"), vec!["RWX: ILLEGAL_OPERATION", "RX: OK", "fetch trang RX được"]);
    assert_eq!(m.find_tcb("p").unwrap().1.state, ThreadState::Faulted);
}

async fn reader(s: Sys) -> Result<(), String> {
    let addr = s.env("buf_addr")?;
    loop {
        let d = s.load(addr, 4).await;
        s.print(String::from_utf8_lossy(&d).to_string()).await;
    }
}

async fn unmapper(s: Sys) -> Result<(), String> {
    s.store(s.env("buf_addr")?, b"DATA").await;
    for _ in 0..6 {
        s.yield_now().await;
    }
    s.sys(Syscall::FrameUnmap { frame: s.env("buf")? }).await;
    s.print("đã unmap").await;
    Ok(())
}

fn shootdown_case(shootdown: bool) -> Machine {
    let manifest = r#"
harts = 2
[objects]
buf = "frame"

[[tasks]]
name = "unmapper"
image = "unmapper"
core = 0
priority = 100
env = { buf_addr = 0x200000 }
caps = [{ name = "buf", slot = 1, obj = "buf", rights = "RW", map = 0x200000 }]

[[tasks]]
name = "reader"
image = "reader"
core = 1
priority = 100
share_vspace = "unmapper"
env = { buf_addr = 0x200000 }
"#;
    let mut m = boot(manifest, |im| {
        im.register("reader", &[], |s| program(reader(s)));
        im.register("unmapper", &[], |s| program(unmapper(s)));
    });
    m.kernel.tlb_shootdown_enabled = shootdown;
    m.run(60);
    m
}

#[test]
fn tlb_shootdown_across_harts() {
    let m = shootdown_case(true);
    assert_eq!(m.find_tcb("reader").unwrap().1.state, ThreadState::Faulted);
    assert!(m.kernel.stats.ipi > 0);
    // Không shootdown: hart 1 vẫn đọc được qua TLB cũ sau khi đã unmap (lỗ hổng).
    let bad = shootdown_case(false);
    let log = &bad.kernel.plat.log;
    let t_unmap = log.iter().find(|l| l.text == "đã unmap").expect("phải unmap").tick;
    assert!(log.iter().any(|l| l.source == "reader" && l.tick > t_unmap));
}

async fn asid_churn(s: Sys) -> Result<(), String> {
    let mem = s.env("mem")?;
    for _ in 0..3 {
        s.retype(mem, ObjType::VSpace, 0, 10).await;
        s.sys(Syscall::CapDelete { slot: 10 }).await;
    }
    s.print("ok").await;
    Ok(())
}

#[test]
fn vspace_destroy_frees_asid() {
    let manifest = r#"
[[tasks]]
name = "p"
image = "p"
caps = [{ name = "mem", slot = 1, untyped_bits = 14 }]
"#;
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(asid_churn(s))));
    m.run(100);
    assert_eq!(m.lines("p"), vec!["ok"]);
    assert_eq!(m.kernel.next_asid, 2); // 1 cho task p, 1 được dùng lại
}

// ===========================================================================
// DMA + IOMMU
// ===========================================================================

fn attack(iommu: bool) -> Machine {
    let manifest = format!(
        r#"
iommu = {}

[[devices]]
name = "camera"
kind = "dma_sensor"
mmio = 0x10000000
irq = 7

[objects]
cam_ntfn = "notification"

[[tasks]]
name = "evil"
image = "dma_attacker"
env = {{ target = {} }}
caps = [
  {{ name = "cam_regs", slot = 1, device = "camera", rights = "RW", map = 0x50000000 }},
  {{ name = "cam_irq", slot = 2, irq = 7, notification = "cam_ntfn" }},
  {{ name = "cam_ntfn", slot = 3, obj = "cam_ntfn", rights = "R" }},
]
"#,
        iommu, RAM_PHYS_BASE
    );
    let mut m = boot(&manifest, |_| {});
    m.run(100);
    m
}

#[test]
fn iommu_blocks_malicious_dma() {
    let m = attack(true);
    assert!(m.kernel_canary_intact());
    assert_eq!(m.lines("evil"), vec!["status=3"]);
    assert_eq!(m.kernel.stats.iommu_fault, 1);
}

#[test]
fn without_iommu_dma_corrupts_kernel() {
    let m = attack(false);
    assert!(!m.kernel_canary_intact());
    assert_eq!(m.lines("evil"), vec!["status=2"]);
}

// ===========================================================================
// Fault
// ===========================================================================

async fn flaky(s: Sys) -> Result<(), String> {
    s.recv(s.env("ep")?).await;
    Err("ValueError: server hỏng giữa chừng".to_string())
}

async fn waiting_client(s: Sys) -> Result<(), String> {
    let r = s.call(s.env("ep")?, Message::empty(1)).await;
    s.print(r.err.name()).await;
    Ok(())
}

#[test]
fn server_crash_cancels_waiting_client() {
    let manifest = r#"
[objects]
ep = "endpoint"
fault = "endpoint"

[[tasks]]
name = "supervisor"
image = "supervisor"
priority = 250
env = { max_restarts = 1 }
caps = [
  { name = "fault_ep", slot = 1, obj = "fault", rights = "R" },
  { name = "tcb_flaky", slot = 10, tcb = "flaky" },
]

[[tasks]]
name = "flaky"
image = "flaky"
priority = 100
fault = { obj = "fault", badge = 10 }
caps = [{ name = "ep", slot = 1, obj = "ep", rights = "R" }]

[[tasks]]
name = "client"
image = "client"
priority = 90
caps = [{ name = "ep", slot = 1, obj = "ep", rights = "W" }]
"#;
    let mut m = boot(manifest, |im| {
        im.register("flaky", &[], |s| program(flaky(s)));
        im.register("client", &[], |s| program(waiting_client(s)));
    });
    m.run(300);
    assert_eq!(m.lines("client"), vec!["CANCELLED"]);
}

async fn bad(s: Sys) -> Result<(), String> {
    let r = s.call(99, Message::empty(0)).await;
    s.print(format!("cptr sai: {}", r.err.name())).await;
    Err("RuntimeError: panic".to_string())
}

#[test]
fn fault_without_handler_stops_only_that_thread() {
    let manifest = r#"
[[tasks]]
name = "bad"
image = "bad"
priority = 100

[[tasks]]
name = "good"
image = "good"
priority = 10
"#;
    let mut m = boot(manifest, |im| {
        im.register("bad", &[], |s| program(bad(s)));
        im.register("good", &[], |s| program(spin(s, "G", 3)));
    });
    m.run(100);
    assert_eq!(m.find_tcb("bad").unwrap().1.state, ThreadState::Faulted);
    assert_eq!(m.lines("bad"), vec!["cptr sai: INVALID_CAP"]);
    assert_eq!(m.lines("good"), vec!["G0", "G1", "G2"]);
}

#[test]
fn dev_mode_halts_everything() {
    let mut manifest = parse_manifest(SPACE_MANIFEST).unwrap();
    if let Some(Value::Array(tasks)) = manifest.get_mut("tasks") {
        if let Some(Value::Table(sup)) = tasks.get_mut(0) {
            if let Some(Value::Table(env)) = sup.get_mut("env") {
                env.insert("dev_mode".to_string(), Value::Integer(1));
            }
        }
    }
    let mut m = build(&manifest, default_images(), false).unwrap();
    m.schedule_irq(5, 2, 8);
    m.run(1500);
    let sup = m.lines("supervisor");
    assert!(sup.iter().any(|x| x.contains("DEV MODE")));
    assert!(!sup.iter().any(|x| x.contains("restart")));
}

async fn abi_prog(s: Sys) -> Result<(), String> {
    s.call(99, Message::empty(0)).await;
    s.print("x").await;
    Ok(())
}

#[test]
fn trap_frame_abi() {
    let manifest = "[[tasks]]\nname = \"p\"\nimage = \"p\"\n";
    let mut m = boot(manifest, |im| im.register("p", &[], |s| program(abi_prog(s))));
    m.run(3);
    let t = m.find_tcb("p").unwrap().1;
    assert_eq!(t.regs.pc, 8); // 2 syscall, mỗi lần pc += 4
    assert_eq!(t.regs.a[0], KError::Ok as u64); // kết quả DebugPrint
}

// ===========================================================================
// Ngắt
// ===========================================================================

async fn irq_drv(s: Sys) -> Result<(), String> {
    let n = s.env("n")?;
    let r = s.wait(n).await;
    s.print(format!("ngắt 1, value={}", r.value)).await;
    for _ in 0..5 {
        s.yield_now().await;
    }
    s.print("ack").await;
    s.sys(Syscall::IrqAck { handler: s.env("irq")? }).await;
    let r = s.wait(n).await;
    s.print(format!("ngắt 2, value={}", r.value)).await;
    Ok(())
}

#[test]
fn irq_masked_until_ack() {
    let manifest = r#"
[objects]
n = "notification"

[[tasks]]
name = "drv"
image = "drv"
priority = 100
caps = [
  { name = "n", slot = 1, obj = "n", rights = "R" },
  { name = "irq", slot = 2, irq = 3, notification = "n" },
]
"#;
    let mut m = boot(manifest, |im| im.register("drv", &[], |s| program(irq_drv(s))));
    m.schedule_irq(3, 1, 0);
    m.schedule_irq(3, 3, 0); // tới khi đang bị che: pending
    m.schedule_irq(3, 4, 0); // gộp vào pending
    m.run(100);
    assert_eq!(m.lines("drv"), vec!["ngắt 1, value=1", "ack", "ngắt 2, value=1"]);
}
