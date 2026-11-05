//! RISC-V QEMU `virt` platform: Sv39 MMU, SBI timer, trap entry and SMP.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use vifast_abi::{KError, Rights};
use vifast_arch::bkl::{BigKernelGuard, BigKernelLock};
use vifast_arch::{Arch, CpuState, MAX_HARTS};
use vifast_kernel::{Kernel, PageTableOps};
use vifast_runtime::{Platform, PlatformInfo};

use vifast_arch_riscv64::Riscv64;

pub struct RiscvPlatform;

pub const USER_CODE_VA: usize = vifast_abi::user_layout::CODE_VA;
pub const USER_DATA_VA: usize = vifast_abi::user_layout::DATA_VA;
pub const USER_STACK_BASE: usize = vifast_abi::user_layout::STACK_BASE;
pub const USER_STACK_LEN: usize = vifast_abi::user_layout::STACK_LEN;
const USER_SSTATUS: u64 = 1 << 5;
const TIMER_INTERVAL: u64 = 100_000;
const PTE_V: u64 = 1;
const PTE_R: u64 = 2;
const PTE_W: u64 = 4;
const PTE_X: u64 = 8;

core::arch::global_asm!(include_str!("trap.S"));
core::arch::global_asm!(include_str!("smp.S"));

#[repr(align(4096))]
struct Table([u64; 512]);

static mut ROOT: Table = Table([0; 512]);
static mut KERNEL_L1: Table = Table([0; 512]);
static mut DEVICE_L1: Table = Table([0; 512]);
#[allow(dead_code)]
#[repr(align(4096))]
struct DataPool([u8; 262144]);
static mut USER_DATA: DataPool = DataPool([0; 262144]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 65536]);
static mut METADATA: Metadata = Metadata([0; 65536]);
static mut CONTEXTS: [TrapFrame; 128] = [TrapFrame::empty(); 128];
#[allow(dead_code)]
#[repr(align(4096))]
struct BootPool([u8; 131072]);
static mut BOOT_POOL: BootPool = BootPool([0; 131072]);
static mut KERNEL_PTR: *mut Kernel = core::ptr::null_mut();
#[allow(dead_code)]
#[repr(align(16))]
#[derive(Clone, Copy)]
struct TrapStack([u8; 32768]);
static mut TRAP_STACKS: [TrapStack; MAX_HARTS] = [TrapStack([0; 32768]); MAX_HARTS];
static mut CPU_STATES: [CpuState; MAX_HARTS] = [CpuState::empty(); MAX_HARTS];
static SECONDARY_ONLINE: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_IPI_WAKES: AtomicUsize = AtomicUsize::new(0);
static SCHED_READY: AtomicBool = AtomicBool::new(false);
static RESCHED_SENT: AtomicUsize = AtomicUsize::new(0);
static RESCHED_WAKE: AtomicUsize = AtomicUsize::new(0);
static IDLE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static IDLE_HARTS: [AtomicBool; MAX_HARTS] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];
static RESCHED_PENDING: [AtomicBool; MAX_HARTS] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];
static USER_ENTRIES: [AtomicUsize; MAX_HARTS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
const MAINT_TLB: u32 = 1;
const MAINT_ICACHE: u32 = 2;
const MAINT_RESCHEDULE: u32 = 4;
static MAINT_SEQ: [AtomicUsize; MAX_HARTS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static MAINT_ACK: [AtomicUsize; MAX_HARTS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static MAINT_KIND: [AtomicU32; MAX_HARTS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static BKL: BigKernelLock = BigKernelLock::new();
static BKL_OWNER: AtomicUsize = AtomicUsize::new(usize::MAX);

struct PlatformBklGuard {
    _guard: BigKernelGuard,
    hart: usize,
}

impl Drop for PlatformBklGuard {
    fn drop(&mut self) {
        if BKL_OWNER.load(Ordering::Acquire) == self.hart {
            BKL_OWNER.store(usize::MAX, Ordering::Release);
        }
    }
}

fn bkl_lock() -> PlatformBklGuard {
    let state = cpu_state();
    let hart = if state.is_null() {
        0
    } else {
        unsafe { (*state).hart_id }
    };
    let guard = BKL.lock_with(poll_maintenance);
    BKL_OWNER.store(hart, Ordering::Release);
    PlatformBklGuard {
        _guard: guard,
        hart,
    }
}

fn assert_bkl_released() {
    let state = cpu_state();
    let hart = if state.is_null() {
        0
    } else {
        unsafe { (*state).hart_id }
    };
    assert!(
        BKL_OWNER.load(Ordering::Acquire) != hart,
        "BKL held on current hart before user return"
    );
}

fn poll_maintenance() {
    let state = cpu_state();
    if state.is_null() {
        return;
    }
    let hart = unsafe { (*state).hart_id };
    if hart >= MAX_HARTS {
        return;
    }
    let seq = MAINT_SEQ[hart].load(Ordering::Acquire);
    if MAINT_ACK[hart].load(Ordering::Acquire) >= seq {
        return;
    }
    let kind = MAINT_KIND[hart].swap(0, Ordering::AcqRel);
    if kind & MAINT_TLB != 0 {
        unsafe {
            core::arch::asm!("sfence.vma zero, zero");
        }
    }
    if kind & MAINT_ICACHE != 0 {
        unsafe {
            core::arch::asm!("fence.i");
        }
    }
    MAINT_ACK[hart].store(seq, Ordering::Release);
}

fn maintenance_target_online(target: usize) -> bool {
    target == 0 || (target == 1 && SECONDARY_ONLINE.load(Ordering::SeqCst) > 0)
}

fn send_maintenance(target: usize, kind: u32) -> bool {
    assert!(
        SECONDARY_ONLINE.load(Ordering::SeqCst) + 1 <= 2,
        "maintenance mailbox supports at most 2 online harts"
    );
    if target >= MAX_HARTS || !maintenance_target_online(target) {
        return false;
    }
    let state = cpu_state();
    if state.is_null() || unsafe { (*state).hart_id } == target {
        return false;
    }
    assert!(
        BKL_OWNER.load(Ordering::Acquire) != usize::MAX,
        "maintenance IPI sender must hold BKL"
    );
    let seq = MAINT_SEQ[target].fetch_add(1, Ordering::AcqRel) + 1;
    MAINT_KIND[target].fetch_or(kind, Ordering::AcqRel);
    if target == 1 {
        <Riscv64 as Arch>::send_ipi(1);
    } else {
        <Riscv64 as Arch>::send_ipi(0);
    }
    let mut spins = 0usize;
    while MAINT_ACK[target].load(Ordering::Acquire) < seq && spins < 5_000_000 {
        poll_maintenance();
        core::hint::spin_loop();
        spins += 1;
    }
    MAINT_ACK[target].load(Ordering::Acquire) >= seq
}

fn send_reschedule(target: usize) -> bool {
    assert!(
        BKL_OWNER.load(Ordering::Acquire) != usize::MAX,
        "reschedule sender must hold BKL"
    );
    if target >= MAX_HARTS || !maintenance_target_online(target) {
        return false;
    }
    // Fire-and-forget: only the target's idle window needs an IPI.  A task
    // that is already running will observe the new TCB on its next tick/Yield.
    if !IDLE_HARTS[target].load(Ordering::SeqCst) {
        return false;
    }
    // At most one reschedule IPI per idle interval; this keeps S <= I.
    if RESCHED_PENDING[target].swap(true, Ordering::SeqCst) {
        return false;
    }
    let seq = MAINT_SEQ[target].fetch_add(1, Ordering::AcqRel) + 1;
    MAINT_KIND[target].fetch_or(MAINT_RESCHEDULE, Ordering::AcqRel);
    let _ = seq;
    <Riscv64 as Arch>::send_ipi(target);
    RESCHED_SENT.fetch_add(1, Ordering::Relaxed);
    true
}

fn request_ipi() -> bool {
    let state = cpu_state();
    if state.is_null() {
        return false;
    }
    let self_hart = unsafe { (*state).hart_id };
    let target = if self_hart == 0 { 1 } else { 0 };
    send_maintenance(target, MAINT_TLB | MAINT_ICACHE)
}

fn assert_irqs_disabled() {
    let sstatus: u64;
    unsafe {
        core::arch::asm!("csrr {}, sstatus", out(reg) sstatus);
    }
    assert!(sstatus & (1 << 1) == 0, "BKL entered with SIE set");
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    regs: [u64; 32],
    sepc: u64,
    sstatus: u64,
    scause: u64,
}

impl TrapFrame {
    const fn empty() -> TrapFrame {
        TrapFrame {
            regs: [0; 32],
            sepc: 0,
            sstatus: 0,
            scause: 0,
        }
    }

    fn user(entry: usize, sp: usize) -> TrapFrame {
        let mut frame = TrapFrame::empty();
        frame.regs[2] = sp as u64;
        frame.sepc = entry as u64;
        frame.sstatus = USER_SSTATUS;
        frame
    }
}

fn pte(pa: u64, flags: u64) -> u64 {
    ((pa >> 12) << 10) | flags
}

fn table_pa(table: *const Table) -> u64 {
    table as u64
}

fn page_in_range(ptr: u64, len: usize, base: usize, size: usize) -> bool {
    ptr >= base as u64 && ptr as usize + len <= base + size
}

unsafe fn set_timer(at: u64) {
    core::arch::asm!(
        "ecall",
        in("a7") 0x5449_4D45usize,
        in("a6") 0usize,
        in("a0") at,
    );
}

#[no_mangle]
pub extern "C" fn rust_secondary(hartid: usize) -> ! {
    unsafe {
        init_cpu(hartid);
        core::arch::asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize);
        // Keep SIE clear: idle uses WFI plus explicit SIP polling, avoiding
        // an S-mode trap/sret path that OpenSBI rejects on this hart.
        core::arch::asm!("csrs sie, {}", in(reg) (1u64 << 5) | (1u64 << 1));
        set_timer(Riscv64::now() + TIMER_INTERVAL);
    }
    SECONDARY_ONLINE.fetch_add(1, Ordering::SeqCst);
    loop {
        if !SCHED_READY.load(Ordering::Acquire) {
            poll_maintenance();
            core::hint::spin_loop();
            continue;
        }
        // Set the idle flag while still holding BKL.  A sender can only
        // observe idle after enqueueing under BKL, so the recheck below closes
        // the enqueue-vs-WFI race.
        let picked = {
            let bkl = bkl_lock();
            let picked = RiscvPlatform::kernel().pick_on(1);
            if picked.is_none() {
                IDLE_HARTS[1].store(true, Ordering::SeqCst);
                if RiscvPlatform::kernel().has_ready_on(1) {
                    IDLE_HARTS[1].store(false, Ordering::SeqCst);
                }
            }
            if let Some(tcb) = picked {
                publish_current_tcb(Some(tcb.index as usize));
                if vifast_runtime::ensure_current_root::<RiscvPlatform>().is_err() {
                    drop(bkl);
                    Riscv64::exit(1);
                }
            }
            picked
        };
        if let Some(tcb) = picked {
            let context = RiscvPlatform::contexts()[tcb.index as usize];
            <RiscvPlatform as Platform>::enter_user(&context);
        }
        // No task and no newly-ready task: enter the masked-WFI idle window.
        IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        unsafe {
            core::arch::asm!("wfi");
        }
        IDLE_HARTS[1].store(false, Ordering::SeqCst);
        RESCHED_PENDING[1].store(false, Ordering::SeqCst);
        let sip: u64;
        unsafe {
            core::arch::asm!("csrr {}, sip", out(reg) sip);
        }
        if sip & (1u64 << 5) != 0 {
            SECONDARY_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
            unsafe {
                set_timer(Riscv64::now() + TIMER_INTERVAL);
            }
        }
        if sip & (1u64 << 1) != 0 {
            SECONDARY_IPI_WAKES.fetch_add(1, Ordering::Relaxed);
            let resched_wake = MAINT_KIND[1].load(Ordering::Acquire) & MAINT_RESCHEDULE != 0;
            unsafe {
                core::arch::asm!("csrc sip, {}", in(reg) 1u64 << 1);
            }
            poll_maintenance();
            if resched_wake {
                RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

unsafe fn build_kernel_page_tables() {
    let root = core::ptr::addr_of_mut!(ROOT) as *mut Table;
    let kernel_l1 = core::ptr::addr_of_mut!(KERNEL_L1) as *mut Table;
    let device_l1 = core::ptr::addr_of_mut!(DEVICE_L1) as *mut Table;
    (*root).0[0] = pte(table_pa(device_l1), PTE_V);
    (*root).0[2] = pte(table_pa(kernel_l1), PTE_V);
    (*kernel_l1).0[0] = pte(0x8000_0000, PTE_V | PTE_R | PTE_W | PTE_X);
    (*kernel_l1).0[1] = pte(0x8020_0000, PTE_V | PTE_R | PTE_W | PTE_X);
    (*device_l1).0[128] = pte(0x1000_0000, PTE_V | PTE_R | PTE_W);
}

unsafe fn init_cpu(hartid: usize) {
    let index = core::cmp::min(hartid, MAX_HARTS - 1);
    CPU_STATES[index].hart_id = hartid;
    CPU_STATES[index].current_tcb = 0;
    let stack = core::ptr::addr_of!(TRAP_STACKS[index]) as u64;
    CPU_STATES[index].trap_stack_top = stack + 32768;
    let state = core::ptr::addr_of!(CPU_STATES[index]) as usize;
    core::arch::asm!("mv tp, {}", in(reg) state);
    core::arch::asm!("csrw sscratch, zero");
}

fn cpu_state() -> *mut CpuState {
    let state: *mut CpuState;
    unsafe {
        core::arch::asm!("mv {}, tp", out(reg) state);
    }
    state
}

fn publish_current_tcb(tcb: Option<usize>) {
    let state = cpu_state();
    unsafe {
        (*state).current_tcb = tcb.unwrap_or(usize::MAX);
    }
}

fn active_root() -> u64 {
    unsafe { (*cpu_state()).active_root }
}

fn active_asid() -> u16 {
    unsafe { (*cpu_state()).active_asid }
}

fn set_active_root(root: u64, asid: u16) {
    unsafe {
        (*cpu_state()).active_root = root;
        (*cpu_state()).active_asid = asid;
    }
}

fn assert_current_tcb() {
    let state = cpu_state();
    let hart = unsafe { (*state).hart_id };
    let expected = RiscvPlatform::kernel()
        .current_on(hart)
        .map(|tcb| tcb.index as usize)
        .unwrap_or(usize::MAX);
    assert_eq!(
        unsafe { (*state).current_tcb },
        expected,
        "per-CPU current_tcb mismatch"
    );
}

fn active_root_matches_current() -> bool {
    unsafe {
        if KERNEL_PTR.is_null() || active_root() == 0 {
            return false;
        }
        let kernel = &*KERNEL_PTR;
        let Some(tcb) = kernel.current() else {
            return false;
        };
        kernel.current_root(tcb) == Some(active_root())
    }
}

impl Arch for RiscvPlatform {
    const NAME: &'static str = Riscv64::NAME;
    fn init() {
        Riscv64::init();
    }
    fn putc(byte: u8) {
        Riscv64::putc(byte);
    }
    fn now() -> u64 {
        Riscv64::now()
    }
    fn irq_disable() {
        Riscv64::irq_disable();
    }
    fn irq_enable() {
        Riscv64::irq_enable();
    }
    fn send_ipi(hart: usize) {
        Riscv64::send_ipi(hart);
    }
    fn halt() -> ! {
        Riscv64::halt()
    }
    fn exit(code: u8) -> ! {
        Riscv64::exit(code)
    }
}

impl vifast_kernel::console::Console for RiscvPlatform {
    const NAME: &'static str = Riscv64::NAME;

    fn write_byte(byte: u8) {
        Riscv64::putc(byte);
    }
}

struct PageOps;

impl vifast_kernel::PageTableOps for PageOps {
    fn levels(&self) -> usize {
        vifast_pte::riscv::LEVELS
    }

    fn index(&self, va: u64, level: usize) -> usize {
        vifast_pte::riscv::index(va, level)
    }

    fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
        unsafe { *((table_pa as *const u64).add(idx)) }
    }

    fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
        unsafe {
            *((table_pa as *mut u64).add(idx)) = pte;
            core::arch::asm!("sfence.vma");
        }
    }

    fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
        vifast_pte::riscv::encode_leaf(pa, rights)
    }

    fn encode_table(&self, pa: u64) -> u64 {
        vifast_pte::riscv::encode_table(pa)
    }

    fn decode(&self, entry: u64) -> vifast_kernel::Pte {
        vifast_pte::riscv::decode(entry)
    }

    fn decode_at(&self, entry: u64, level: usize) -> vifast_kernel::Pte {
        vifast_pte::riscv::decode_at(entry, level)
    }

    fn vspace_pages(&self) -> usize {
        1
    }

    fn install_kernel(&self, root_pa: u64, _aux_pa: u64) {
        unsafe {
            let dst = root_pa as *mut u64;
            let src = core::ptr::addr_of!(ROOT) as *const u64;
            for slot in vifast_pte::riscv::KERNEL_SHARED_SLOTS {
                *dst.add(*slot) = *src.add(*slot);
            }
        }
    }

    fn kernel_accessible_range(&self) -> (u64, u64) {
        (0x8000_0000, 0x8040_0000)
    }

    fn user_va_range(&self) -> (u64, u64) {
        (
            vifast_pte::riscv::USER_VA_START,
            vifast_pte::riscv::USER_VA_END,
        )
    }

    fn flush_va(&self, va: u64) {
        unsafe {
            core::arch::asm!("sfence.vma {}, zero", in(reg) va);
        }
    }

    fn flush_all_user(&self) {
        unsafe {
            core::arch::asm!("sfence.vma zero, zero");
        }
        if BKL.is_held() {
            let _ = request_ipi();
        }
    }

    fn sync_icache(&self, _pa: u64, _size: usize) {
        unsafe {
            core::arch::asm!("fence.i");
        }
        // fence.i is local; when called from a kernel mapping path under BKL,
        // the symmetric mailbox fences every other online hart.  Early boot
        // mapping runs before hart 1 can execute user code, so stay local.
        if BKL.is_held() {
            let _ = request_ipi();
        }
    }

    fn leaf_is_user(&self, pte: u64) -> bool {
        vifast_pte::riscv::is_user(pte)
    }

    fn copy_to_frame(&self, pa: u64, bytes: &[u8]) {
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), pa as *mut u8, bytes.len());
        }
    }

    fn copy_frame_to_frame(&self, src: u64, dst: u64, size: usize) {
        unsafe {
            core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, size);
        }
    }

    fn zero_frame(&self, pa: u64, size: usize) {
        unsafe {
            core::ptr::write_bytes(pa as *mut u8, 0, size);
        }
    }
}

/// Walk the real boot page table and verify a kernel leaf.
fn kernel_leaf_ok(root_pa: u64, va: u64, need_write: bool, need_exec: bool) -> bool {
    let mut table = root_pa;
    for level in 0..vifast_pte::riscv::LEVELS {
        let idx = vifast_pte::riscv::index(va, level);
        let pte = PageOps.read_pte(table, idx);
        match PageOps.decode_at(pte, level) {
            vifast_kernel::Pte::Table(next) => {
                if level + 1 == vifast_pte::riscv::LEVELS {
                    return false;
                }
                table = next;
            }
            vifast_kernel::Pte::Leaf(_, rights) => {
                return rights.contains(Rights::READ)
                    && !vifast_pte::riscv::is_user(pte)
                    && (!need_write || pte & vifast_pte::riscv::PTE_W != 0)
                    && (!need_exec || pte & vifast_pte::riscv::PTE_X != 0);
            }
            vifast_kernel::Pte::Invalid => return false,
        }
    }
    false
}

fn kernel_range_ok(root_pa: u64) -> bool {
    let (start, end) = PageOps.kernel_accessible_range();
    let mut va = start;
    while va < end {
        if !kernel_leaf_ok(root_pa, va, false, false) {
            return false;
        }
        va = match va.checked_add(4096) {
            Some(next) => next,
            None => break,
        };
    }
    true
}

static PAGE_OPS: PageOps = PageOps;

static mut KERNEL_STORAGE: Kernel = Kernel::new_const();

impl Platform for RiscvPlatform {
    type Context = TrapFrame;

    const USER_CODE_VA: usize = USER_CODE_VA;
    const USER_DATA_VA: usize = USER_DATA_VA;
    const ELF_MACHINE: u16 = vifast_pte::riscv::ELF_MACHINE;

    fn init_platform(info: PlatformInfo) {
        unsafe {
            build_kernel_page_tables();
            init_cpu(info.hartid);
        }
    }

    fn set_current_tcb(tcb: Option<usize>) {
        publish_current_tcb(tcb);
    }

    fn hart_id() -> usize {
        let state = cpu_state();
        if state.is_null() {
            0
        } else {
            unsafe { (*state).hart_id }
        }
    }

    fn secondary_wakeups() -> (usize, usize) {
        (
            SECONDARY_TIMER_WAKES.load(Ordering::Relaxed),
            SECONDARY_IPI_WAKES.load(Ordering::Relaxed),
        )
    }

    fn wake_hart(hart: usize) {
        assert!(
            SECONDARY_ONLINE.load(Ordering::SeqCst) + 1 <= 2,
            "reschedule mailbox supports at most 2 online harts"
        );
        if hart == 1 && SECONDARY_ONLINE.load(Ordering::SeqCst) > 0 {
            let _ = send_reschedule(1);
        }
    }

    fn user_entries(hart: usize) -> usize {
        USER_ENTRIES
            .get(hart)
            .map(|counter| counter.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    fn resched_sent() -> usize {
        RESCHED_SENT.load(Ordering::Relaxed)
    }

    fn resched_wake() -> usize {
        RESCHED_WAKE.load(Ordering::Relaxed)
    }

    fn idle_entries() -> usize {
        IDLE_ENTRIES.load(Ordering::Relaxed)
    }

    fn init_block() {
        let (lo, hi) = PageOps.kernel_accessible_range();
        let (dma_lo, dma_hi) = vifast_virtio::dma_range();
        if dma_lo < lo || dma_hi > hi {
            vifast_kernel::klog!(
                RiscvPlatform,
                "VIRTIO DMA OUTSIDE kernel range {:#x}..{:#x} (dma {:#x}..{:#x})",
                lo,
                hi,
                dma_lo,
                dma_hi
            );
            return;
        }
        if vifast_virtio::init_range(0x1000_1000, 0x1000_9000, 0x1000) {
            let (base, magic, version, device) = vifast_virtio::probe_info();
            vifast_kernel::klog!(
                RiscvPlatform,
                "VIRTIO OK base={:#x} magic={:#x} version={} device={}",
                base,
                magic,
                version,
                device
            );
        } else if vifast_virtio::legacy_seen() {
            vifast_kernel::klog!(RiscvPlatform, "BLOCK SKIP (virtio legacy)");
        } else {
            let (base, magic, version, device) = vifast_virtio::probe_info();
            vifast_kernel::klog!(
                RiscvPlatform,
                "VIRTIO SKIP base={:#x} magic={:#x} version={} device={}",
                base,
                magic,
                version,
                device
            );
        }
    }
    fn init_kernel() -> &'static mut Kernel {
        unsafe {
            let kernel = &mut *core::ptr::addr_of_mut!(KERNEL_STORAGE);
            kernel.reset();
            kernel
        }
    }

    fn set_kernel(kernel: *mut Kernel) {
        unsafe {
            KERNEL_PTR = kernel;
        }
    }

    fn kernel() -> &'static mut Kernel {
        unsafe { &mut *KERNEL_PTR }
    }

    fn boot_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BOOT_POOL) as *mut u8, 131072)
        }
    }

    fn data_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_DATA) as *mut u8, 262144)
        }
    }

    fn metadata_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(METADATA) as *mut u8, 65536)
        }
    }

    fn vspace_root() -> u64 {
        table_pa(core::ptr::addr_of!(ROOT))
    }

    fn switch_root(root_pa: u64, asid: u16) -> bool {
        unsafe {
            if root_pa == 0 || (root_pa == active_root() && asid == active_asid()) {
                return false;
            }
            set_active_root(root_pa, asid);
            let satp = (8u64 << 60) | ((asid as u64) << 44) | (root_pa >> 12);
            core::arch::asm!("csrw satp, {}", in(reg) satp);
            // Invalidate only the new address-space tag.  ASIDs are unique
            // until wrap, so entries belonging to other VSpaces remain valid.
            core::arch::asm!("sfence.vma zero, {}", in(reg) asid as u64);
        }
        true
    }

    fn page_table_ops() -> &'static dyn vifast_kernel::PageTableOps {
        &PAGE_OPS
    }

    fn kernel_image_end() -> u64 {
        core::ptr::addr_of!(__kernel_end) as u64
    }

    fn check_kernel_mappings(root_pa: u64) -> bool {
        kernel_range_ok(root_pa)
            && kernel_leaf_ok(root_pa, trap_entry as *const () as u64, false, true)
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(TRAP_STACKS) as u64,
                true,
                false,
            )
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(KERNEL_STORAGE) as u64,
                true,
                false,
            )
            && kernel_leaf_ok(root_pa, 0x1000_0000, true, false)
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if page_in_range(ptr, out.len(), USER_CODE_VA, 0x2_0000)
            || page_in_range(ptr, out.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, out.len(), USER_DATA_VA, 4096)
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
                vifast_abi::user_layout::ROOT_MODULE_END
                    - vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
            )
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::IPC_BUF_SERVER_VA,
                4096,
            )
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::IPC_BUF_CLIENT_VA,
                4096,
            )
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::ROOT_ELF_BASE,
                vifast_abi::user_layout::ROOT_ELF_STACK_TOP
                    - vifast_abi::user_layout::ROOT_ELF_BASE,
            )
        {
            unsafe {
                /* SUM is kept clear while in S-mode; enable it only for the
                explicit copy window. */
                core::arch::asm!("csrs sstatus, {}", in(reg) 1u64 << 18);
                core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), out.len());
                core::arch::asm!("csrc sstatus, {}", in(reg) 1u64 << 18);
            }
            Ok(())
        } else {
            Err(KError::InvalidArg)
        }
    }

    fn write_user(ptr: u64, input: &[u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if page_in_range(ptr, input.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, input.len(), USER_DATA_VA, 4096)
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
                vifast_abi::user_layout::ROOT_MODULE_END
                    - vifast_abi::user_layout::ROOT_MODULE_SERVER_VA,
            )
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::IPC_BUF_SERVER_VA,
                4096,
            )
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::IPC_BUF_CLIENT_VA,
                4096,
            )
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::ROOT_ELF_BASE,
                vifast_abi::user_layout::ROOT_ELF_STACK_TOP
                    - vifast_abi::user_layout::ROOT_ELF_BASE,
            )
        {
            unsafe {
                core::arch::asm!("csrs sstatus, {}", in(reg) 1u64 << 18);
                core::ptr::copy_nonoverlapping(input.as_ptr(), ptr as *mut u8, input.len());
                core::arch::asm!("csrc sstatus, {}", in(reg) 1u64 << 18);
            }
            Ok(())
        } else {
            Err(KError::InvalidArg)
        }
    }

    fn context_for_user(entry: usize, stack: usize) -> TrapFrame {
        TrapFrame::user(entry, stack)
    }

    fn set_context_arg0(context: &mut TrapFrame, value: u64) {
        context.regs[10] = value;
    }

    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        assert_bkl_released();
        // Boot hart releases hart 1 only here: the BKL assertion above must
        // run before hart 1 can acquire the global lock and make this fail.
        SCHED_READY.store(true, Ordering::Release);
        unsafe {
            core::arch::asm!("csrw sstatus, {}", in(reg) context.sstatus);
            core::arch::asm!("csrw sepc, {}", in(reg) context.sepc);
            core::arch::asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize);
            let cpu: *const CpuState;
            core::arch::asm!("mv {}, tp", out(reg) cpu);
            assert!(!cpu.is_null(), "RISC-V per-CPU pointer is null");
            // Kernel-only register: user code cannot redirect this pointer.
            core::arch::asm!("csrw sscratch, {}", in(reg) cpu);
            let hart = (*cpu).hart_id;
            if let Some(counter) = USER_ENTRIES.get(hart) {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            core::arch::asm!("csrs sie, {}", in(reg) 1u64 << 5);
            core::arch::asm!("mv tp, {}", in(reg) context.regs[4]);
            core::arch::asm!("mv sp, {}", in(reg) context.regs[2]);
            core::arch::asm!("mv a0, {}", in(reg) context.regs[10]);
            core::arch::asm!("sret", options(noreturn));
        }
    }

    fn write_result(
        context: &mut TrapFrame,
        error: KError,
        value: u64,
        label: u64,
        data: [u64; 4],
    ) {
        context.regs[10] = error as u64;
        context.regs[11] = value;
        context.regs[12] = label;
        context.regs[13] = data[0];
        context.regs[14] = data[1];
        context.regs[15] = data[2];
        context.regs[16] = data[3];
    }

    fn start_timer() {
        unsafe {
            set_timer(Riscv64::now() + TIMER_INTERVAL);
        }
    }

    fn reload_timer() {
        unsafe {
            set_timer(Riscv64::now() + TIMER_INTERVAL);
        }
    }

    fn ack_timer() {}

    fn block_read(lba: u64, out: &mut [u8; 512]) -> bool {
        vifast_virtio::read_sector(lba, out)
    }

    fn block_write(lba: u64, input: &[u8; 512]) -> bool {
        vifast_virtio::write_sector(lba, input)
    }

    fn start_smp() {
        let entry = secondary_entry as *const () as usize;
        let started = unsafe {
            let result: usize;
            core::arch::asm!(
                "ecall",
                in("a7") 0x4853_4dusize,
                in("a6") 0usize,
                in("a0") 1usize,
                in("a1") entry,
                in("a2") 0usize,
                lateout("a0") result,
            );
            result
        };
        let mut spins = 0u32;
        while SECONDARY_ONLINE.load(Ordering::SeqCst) == 0 && spins < 10_000_000 {
            core::hint::spin_loop();
            spins += 1;
        }
        if SECONDARY_ONLINE.load(Ordering::SeqCst) > 0 {
            let bkl = bkl_lock();
            let ipi_ok = request_ipi();
            drop(bkl);
            if ipi_ok {
                vifast_kernel::klog!(RiscvPlatform, "SMP OK (harts=2, ipi=1)");
            } else {
                vifast_kernel::klog!(RiscvPlatform, "SMP OK (harts=2, ipi=pending)");
            }
        } else {
            vifast_kernel::klog!(RiscvPlatform, "SMP SKIP (hsm_start={:#x})", started);
        }
    }
}

#[no_mangle]
extern "C" fn riscv_kernel_trap() -> ! {
    let cause: u64;
    unsafe {
        core::arch::asm!("csrr {}, scause", out(reg) cause);
    }
    vifast_kernel::klog!(RiscvPlatform, "KERNEL TRAP cause={:#x}", cause);
    Riscv64::exit(1)
}

extern "C" {
    fn trap_entry();
    fn secondary_entry();
    static __kernel_end: u8;
}

fn count_user_entry() {
    let state = cpu_state();
    if state.is_null() {
        return;
    }
    let hart = unsafe { (*state).hart_id };
    if let Some(counter) = USER_ENTRIES.get(hart) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[no_mangle]
pub extern "C" fn riscv_trap_handler(frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };
    // Count every return to user, not only the first enter_user.
    if frame.sstatus & (1 << 8) == 0 {
        count_user_entry();
    }
    if frame.scause == 0x8000_0000_0000_0001 {
        unsafe {
            core::arch::asm!("csrc sip, {}", in(reg) 1u64 << 1);
        }
        poll_maintenance();
        return;
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb();
    if frame.scause == 0x8000_0000_0000_0005 {
        vifast_runtime::timer::on_timer::<RiscvPlatform>(frame);
        drop(bkl);
        assert_bkl_released();
        return;
    }
    if frame.scause != 8 {
        /* SPP=1 means the fault came from S-mode.  That is a kernel bug,
        not a user fault, and must not be suspended. */
        if frame.sstatus & (1 << 8) != 0 {
            vifast_kernel::klog!(RiscvPlatform, "KERNEL FAULT cause={:#x}", frame.scause);
            Riscv64::exit(1);
        }
        let stval: u64;
        unsafe {
            core::arch::asm!("csrr {}, stval", out(reg) stval);
        }
        vifast_runtime::fault::suspend::<RiscvPlatform>(frame, frame.scause, stval);
        drop(bkl);
        assert_bkl_released();
        return;
    }
    let number = frame.regs[17];
    frame.sepc = frame.sepc.wrapping_add(4);
    let args = [
        frame.regs[10],
        frame.regs[11],
        frame.regs[12],
        frame.regs[13],
        frame.regs[14],
        frame.regs[15],
    ];
    vifast_runtime::syscall::dispatch::<RiscvPlatform>(frame, number, args);
    drop(bkl);
    assert_bkl_released();
}
