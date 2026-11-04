//! Chương trình user trong simulator.
//!
//! Mỗi chương trình là một `async` block. `.await` trên một lệnh tương đương việc
//! CPU thực thi lệnh đó: nếu là `ecall` thì trap vào kernel. Simulator poll future
//! để lấy lệnh kế tiếp, thực thi, rồi poll lại kèm kết quả.
//!
//! Trên phần cứng thật, không có gì trong file này: chương trình là mã RISC-V,
//! "kết quả" nằm trong thanh ghi a0/a1 của TrapFrame.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use vifast_abi::{Message, Rights, SysResult, Syscall};

pub type Env = BTreeMap<String, u64>;
/// Chương trình user: `Err(text)` tương đương panic.
pub type Program = Pin<Box<dyn Future<Output = Result<(), String>>>>;

/// Một lệnh CPU mà simulator quan tâm.
#[derive(Clone, Debug)]
pub enum Instr {
    /// `ecall`
    Sys(Syscall),
    /// Lệnh load qua MMU.
    Load { vaddr: u64, len: u64 },
    /// Lệnh store qua MMU.
    Store { vaddr: u64, data: Vec<u8> },
    /// Nạp lệnh để thực thi (cần quyền X).
    Fetch { vaddr: u64 },
}

#[derive(Clone, Debug)]
pub enum Ret {
    None,
    Sys(SysResult),
    Data(Vec<u8>),
}

#[derive(Default)]
pub struct Chan {
    pub out: Option<Instr>,
    pub inp: Option<Ret>,
}

struct Exec {
    chan: Rc<RefCell<Chan>>,
    instr: Option<Instr>,
}

impl Future for Exec {
    type Output = Ret;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Ret> {
        if let Some(i) = self.instr.take() {
            self.chan.borrow_mut().out = Some(i);
            return Poll::Pending;
        }
        match self.chan.borrow_mut().inp.take() {
            Some(r) => Poll::Ready(r),
            None => Poll::Pending,
        }
    }
}

/// Bộ "lệnh máy" mà chương trình user dùng.
#[derive(Clone)]
pub struct Sys {
    chan: Rc<RefCell<Chan>>,
    env: Rc<Env>,
}

impl Sys {
    pub fn new(chan: Rc<RefCell<Chan>>, env: Env) -> Sys {
        Sys { chan, env: Rc::new(env) }
    }

    /// Đọc tham số. Thiếu thì trả lỗi (dùng `?`, tương đương panic KeyError).
    pub fn env(&self, key: &str) -> Result<u64, String> {
        self.env.get(key).copied().ok_or_else(|| format!("KeyError: '{}'", key))
    }

    pub fn env_or(&self, key: &str, default: u64) -> u64 {
        self.env.get(key).copied().unwrap_or(default)
    }

    pub fn env_map(&self) -> &Env {
        &self.env
    }

    pub async fn exec(&self, instr: Instr) -> Ret {
        Exec { chan: self.chan.clone(), instr: Some(instr) }.await
    }

    pub async fn sys(&self, sc: Syscall) -> SysResult {
        match self.exec(Instr::Sys(sc)).await {
            Ret::Sys(r) => r,
            _ => SysResult::ok(),
        }
    }

    pub async fn load(&self, vaddr: u64, len: u64) -> Vec<u8> {
        match self.exec(Instr::Load { vaddr, len }).await {
            Ret::Data(d) => d,
            _ => Vec::new(),
        }
    }

    pub async fn store(&self, vaddr: u64, data: &[u8]) {
        self.exec(Instr::Store { vaddr, data: data.to_vec() }).await;
    }

    pub async fn fetch(&self, vaddr: u64) -> Vec<u8> {
        match self.exec(Instr::Fetch { vaddr }).await {
            Ret::Data(d) => d,
            _ => Vec::new(),
        }
    }

    // ---- tiện ích cho syscall hay dùng ----

    pub async fn print(&self, text: impl Into<String>) {
        self.sys(Syscall::DebugPrint { text: text.into() }).await;
    }

    pub async fn send(&self, ep: u64, msg: Message) -> SysResult {
        self.sys(Syscall::Send { ep, msg }).await
    }

    pub async fn call(&self, ep: u64, msg: Message) -> SysResult {
        self.sys(Syscall::Call { ep, msg, recv_slot: 0 }).await
    }

    pub async fn recv(&self, ep: u64) -> SysResult {
        self.sys(Syscall::Recv { ep, recv_slot: 0 }).await
    }

    pub async fn recv_into(&self, ep: u64, recv_slot: u64) -> SysResult {
        self.sys(Syscall::Recv { ep, recv_slot }).await
    }

    pub async fn reply(&self, msg: Message) -> SysResult {
        self.sys(Syscall::Reply { msg }).await
    }

    pub async fn signal(&self, ntfn: u64) -> SysResult {
        self.sys(Syscall::Signal { ntfn }).await
    }

    pub async fn wait(&self, ntfn: u64) -> SysResult {
        self.sys(Syscall::Wait { ntfn }).await
    }

    pub async fn yield_now(&self) {
        self.sys(Syscall::Yield).await;
    }

    pub async fn get_time(&self) -> u64 {
        self.sys(Syscall::GetTime).await.value
    }

    pub async fn retype(&self, untyped: u64, obj_type: vifast_abi::ObjType, size_bits: u32, dest_index: u64) -> SysResult {
        self.sys(Syscall::Retype { untyped, obj_type, size_bits, dest_index, count: 1, dest_cnode: 0 }).await
    }

    pub async fn cap_copy(&self, src: u64, dest_index: u64, dest_cnode: u64) -> SysResult {
        self.sys(Syscall::CapCopy { src, dest_index, rights: Rights::ALL, dest_cnode }).await
    }
}

/// Poll chương trình một bước.
pub fn poll_program(p: &mut Program) -> Poll<Result<(), String>> {
    let mut cx = Context::from_waker(Waker::noop());
    p.as_mut().poll(&mut cx)
}

/// Tạo `Program` từ một async block. Async block nên kết thúc bằng
/// `Ok::<(), String>(())` để kiểu lỗi được suy ra.
pub fn program<F>(f: F) -> Program
where
    F: Future<Output = Result<(), String>> + 'static,
{
    Box::pin(f)
}

pub type Ctor = Rc<dyn Fn(Sys) -> Program>;

pub struct ImageDef {
    pub name: String,
    /// Tên các tham số a0..a3 khi thread được tạo lúc chạy bằng TcbConfigure.
    pub args: Vec<String>,
    pub ctor: Ctor,
}

/// Danh sách image. Chỉ số = mã image trong syscall TcbConfigure.
#[derive(Clone, Default)]
pub struct Images {
    pub list: Vec<Rc<ImageDef>>,
}

impl Images {
    /// Đăng ký (hoặc thay thế nếu trùng tên, giữ nguyên mã).
    pub fn register<F>(&mut self, name: &str, args: &[&str], f: F)
    where
        F: Fn(Sys) -> Program + 'static,
    {
        let def = Rc::new(ImageDef {
            name: name.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            ctor: Rc::new(f),
        });
        match self.list.iter().position(|d| d.name == name) {
            Some(i) => self.list[i] = def,
            None => self.list.push(def),
        }
    }

    pub fn id(&self, name: &str) -> Option<u32> {
        self.list.iter().position(|d| d.name == name).map(|i| i as u32)
    }

    pub fn get(&self, id: u32) -> Option<Rc<ImageDef>> {
        self.list.get(id as usize).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.list.iter().map(|d| d.name.clone()).collect()
    }
}
