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
const UART_BASE: usize = 0x1000_0000;
const UART_IER: usize = 1;
const UART_LSR: usize = 5;
const UART_RBR: usize = 0;
const UART_LSR_DR: u8 = 0x01;
const PLIC_BASE: usize = 0x0c00_0000;
const PLIC_UART_IRQ: u32 = 10;
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
struct DataPool([u8; 311296]);
static mut USER_DATA: DataPool = DataPool([0; 311296]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 81920]);
static mut METADATA: Metadata = Metadata([0; 81920]);
// Per-object user contexts indexed by `ObjRef::index`.  The kernel object
// table has MAX_OBJECTS slots, so the array must cover that full domain.
const _: () = assert!(vifast_kernel::MAX_TCBS <= u8::MAX as usize);
static mut CONTEXTS: [TrapFrame; vifast_kernel::MAX_TCBS] =
    [TrapFrame::empty(); vifast_kernel::MAX_TCBS];
#[allow(dead_code)]
#[repr(align(4096))]
struct BootPool([u8; 131072]);
static mut BOOT_POOL: BootPool = BootPool([0; 131072]);
struct KernelPointer(core::cell::UnsafeCell<*mut Kernel>);
// Safety: boot installs the pointer once before any secondary hart starts and
// only reads it afterwards; all kernel entry points run under the BKL.
unsafe impl Sync for KernelPointer {}
static KERNEL_PTR: KernelPointer =
    KernelPointer(core::cell::UnsafeCell::new(core::ptr::null_mut()));
#[allow(dead_code)]
#[repr(align(16))]
#[derive(Clone, Copy)]
struct TrapStack([u8; 32768]);
static mut TRAP_STACKS: [TrapStack; MAX_HARTS] = [TrapStack([0; 32768]); MAX_HARTS];
static mut CPU_STATES: [CpuState; MAX_HARTS] = [CpuState::empty(); MAX_HARTS];
static SECONDARY_ONLINE: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static BSP_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static BSP_RATE_BASE: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_USER_TICKS: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_PREEMPTS: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_IPI_WAKES: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_NONE_WAKES: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_WAIT_BLOCKS: AtomicUsize = AtomicUsize::new(0);
static POST_STARTED: AtomicBool = AtomicBool::new(false);
static POST_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static POST_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static POST_IDLE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
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
const MAINT_TLB_VA: u32 = 8;
static TLB_VA_FLUSH: AtomicUsize = AtomicUsize::new(0);
static TLB_IPI_SENT: AtomicUsize = AtomicUsize::new(0);
static TLB_IPI_SKIPPED: AtomicUsize = AtomicUsize::new(0);
static MAINT_TLB_ADDR: [AtomicUsize; MAX_HARTS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static MAINT_TLB_PAGES: [AtomicUsize; MAX_HARTS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static MAINT_TLB_ASID: [AtomicU32; MAX_HARTS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
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

/// QEMU virt exposes the SiFive test finisher at this physical address.
const SIFIVE_TEST: u64 = 0x10_0000;
const SIFIVE_TEST_FAIL: u32 = 0x3333;

/// QEMU's `sifive_test` finisher takes the process exit code in the upper
/// 16 bits; the low half is the FAIL command.  Writing only FAIL exits 0.
pub fn panic_exit(code: u8) -> ! {
    let value = ((code as u32) << 16) | SIFIVE_TEST_FAIL;
    unsafe {
        core::ptr::write_volatile(SIFIVE_TEST as *mut u32, value);
    }
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

pub fn panic_halt_others() {
    let hart = current_hart();
    if hart == 0 {
        if SECONDARY_ONLINE.load(Ordering::SeqCst) > 0 {
            Riscv64::send_ipi(1);
        }
    } else {
        Riscv64::send_ipi(0);
    }
}

fn current_hart() -> usize {
    let state = cpu_state();
    if state.is_null() {
        0
    } else {
        unsafe { (*state).hart_id }
    }
}

fn bkl_lock() -> BigKernelGuard {
    BKL.lock_with_hart(current_hart(), poll_maintenance)
}

fn assert_bkl_released() {
    let hart = current_hart();
    assert!(
        !BKL.held_by(hart),
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
    if kind & MAINT_TLB_VA != 0 {
        let va = MAINT_TLB_ADDR[hart].load(Ordering::Acquire) as u64;
        let pages = MAINT_TLB_PAGES[hart].load(Ordering::Acquire);
        let asid = MAINT_TLB_ASID[hart].load(Ordering::Acquire);
        let mut page = 0;
        while page < pages {
            unsafe {
                core::arch::asm!("sfence.vma {}, {}", in(reg) va + (page as u64) * 4096, in(reg) asid as u64);
            }
            page += 1;
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
        BKL.held_by(current_hart()),
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

#[cfg_attr(feature = "test-skip-tlb-ipi", allow(dead_code))]
fn send_tlb_va(target: usize, asid: u16, va: u64, pages: usize) -> bool {
    if target >= MAX_HARTS || !maintenance_target_online(target) {
        return false;
    }
    let state = cpu_state();
    if state.is_null() || unsafe { (*state).hart_id } == target {
        return false;
    }
    if !BKL.is_held() {
        return false;
    }
    MAINT_TLB_ADDR[target].store(va as usize, Ordering::Release);
    MAINT_TLB_PAGES[target].store(pages, Ordering::Release);
    MAINT_TLB_ASID[target].store(asid as u32, Ordering::Release);
    send_maintenance(target, MAINT_TLB_VA)
}

fn send_reschedule(target: usize) -> bool {
    assert!(
        BKL.held_by(current_hart()),
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
    // Vòng 80 (audit P0-1): dùng checked_add để con trỏ gần u64::MAX không
    // tràn qua phép cộng và lọt kiểm tra cửa sổ.
    if ptr < base as u64 {
        return false;
    }
    let Some(end) = ptr.checked_add(len as u64) else {
        return false;
    };
    let Some(limit) = (base as u64).checked_add(size as u64) else {
        return false;
    };
    end <= limit
}

unsafe fn set_timer(at: u64) {
    core::arch::asm!(
        "ecall",
        in("a7") 0x5449_4D45usize,
        in("a6") 0usize,
        in("a0") at,
    );
}

/// Account one WFI wake.  If several sources are pending, prefer the IPI
/// reason so that one sleep contributes exactly one wakeup.
fn record_secondary_wake(sip: u64) {
    if sip & (1u64 << 5) != 0 {
        SECONDARY_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
        if POST_STARTED.load(Ordering::Relaxed) {
            POST_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
        }
    }
    if sip & (1u64 << 1) != 0 {
        SECONDARY_IPI_WAKES.fetch_add(1, Ordering::Relaxed);
        unsafe {
            core::arch::asm!("csrc sip, {}", in(reg) 1u64 << 1);
        }
        poll_maintenance();
    } else if sip & (1u64 << 5) != 0 {
        SECONDARY_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
        if POST_STARTED.load(Ordering::Relaxed) {
            POST_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
        }
    } else {
        SECONDARY_NONE_WAKES.fetch_add(1, Ordering::Relaxed);
    }
    if sip & (1u64 << 5) != 0 {
        unsafe {
            set_timer(Riscv64::now() + TIMER_INTERVAL);
        }
    }
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
        if vifast_arch::panic::is_panicking() {
            Riscv64::halt();
        }
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
            let slot = RiscvPlatform::kernel().context_slot(tcb).unwrap_or(0) as usize;
            let context = RiscvPlatform::contexts()[slot];
            <RiscvPlatform as Platform>::enter_user(&context);
        }
        // No task and no newly-ready task: enter the masked-WFI idle window.
        IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        if POST_STARTED.load(Ordering::Relaxed) {
            POST_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        }
        unsafe {
            core::arch::asm!("wfi");
        }
        IDLE_HARTS[1].store(false, Ordering::SeqCst);
        RESCHED_PENDING[1].store(false, Ordering::SeqCst);
        let sip: u64;
        unsafe {
            core::arch::asm!("csrr {}, sip", out(reg) sip);
        }
        let resched_wake = MAINT_KIND[1].load(Ordering::Acquire) & MAINT_RESCHEDULE != 0;
        record_secondary_wake(sip);
        if resched_wake {
            RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
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
    (*device_l1).0[0] = pte(0x0, PTE_V | PTE_R | PTE_W);
    (*device_l1).0[96] = pte(PLIC_BASE as u64, PTE_V | PTE_R | PTE_W);
    // PLIC context registers start at +2 MiB (a second 2 MiB block).
    (*device_l1).0[97] = pte((PLIC_BASE + 0x20_0000) as u64, PTE_V | PTE_R | PTE_W);
    (*device_l1).0[128] = pte(0x1000_0000, PTE_V | PTE_R | PTE_W);
}

fn plic_read(offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((PLIC_BASE + offset) as *const u32) }
}

fn plic_write(offset: usize, value: u32) {
    unsafe { core::ptr::write_volatile((PLIC_BASE + offset) as *mut u32, value) };
}

fn plic_claim() -> u32 {
    plic_read(0x200000 + 0x1000 + 0x4)
}

fn plic_complete(irq: u32) {
    plic_write(0x200000 + 0x1000 + 0x4, irq);
}

fn init_plic() {
    // Priority 1 for UART0, enable it for S-mode context 1 (hart 0), and
    // accept all priorities at the threshold.
    plic_write(PLIC_UART_IRQ as usize * 4, 1);
    plic_write(0x2000 + 0x80, 1 << PLIC_UART_IRQ);
    plic_write(0x200000 + 0x1000, 0);
    unsafe {
        core::ptr::write_volatile((UART_BASE + UART_IER) as *mut u8, 0x01);
    }
}

/// Poll virtio-input trong timer IRQ (hart 0) và đẩy ký tự vào RxQueue của
/// UART để shell hiện có đọc được — không cần thay đổi client. Không log gì
/// trong IRQ (tránh mượn BKL lần hai); shell sẽ echo ký tự như gõ phím thật.
fn poll_virtio_input() {
    if !vifast_virtio::input_ready() {
        return;
    }
    let mut queued = false;
    let mut count = 0;
    while count < 16 {
        let Some(event) = vifast_virtio::input_poll() else {
            break;
        };
        count += 1;
        // Log tối đa 16 phím nhấn để test kiểm chứng mã phím (bounded, an
        // toàn trong IRQ); không log phím nhả để tránh nhiễu.
        if event.kind == 1 && event.value != 0 {
            static mut KEY_LOG: u32 = 0;
            unsafe {
                if KEY_LOG < 16 {
                    KEY_LOG += 1;
                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO KEY {}", event.code);
                }
            }
        }
        if let Some(byte) = vifast_virtio::input_key_ascii(event) {
            Riscv64::stash_rx(byte);
            queued = true;
            // Vòng 95: cập nhật màn hình bằng mã phím vừa bấm (chỉ vài phím
            // đầu để bound công việc trong IRQ) — mốc "display tương tác".
            static mut ECHO_DONE: u32 = 0;
            unsafe {
                if ECHO_DONE < 4 && vifast_virtio::gpu_echo_key(event.code) {
                    ECHO_DONE += 1;
                    vifast_kernel::klog!(RiscvPlatform, "INPUT ECHO {}", event.code);
                }
            }
        }
    }
    if queued {
        let _ = RiscvPlatform::kernel().irq_raise(vifast_abi::IRQ_UART);
    }
}

fn drain_uart_rx() {
    unsafe {
        loop {
            let lsr = core::ptr::read_volatile((UART_BASE + UART_LSR) as *const u8);
            if lsr & UART_LSR_DR == 0 {
                break;
            }
            let byte = core::ptr::read_volatile((UART_BASE + UART_RBR) as *const u8);
            Riscv64::stash_rx(byte);
        }
    }
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
        let kernel_ptr = *KERNEL_PTR.0.get();
        if kernel_ptr.is_null() || active_root() == 0 {
            return false;
        }
        let kernel = &*kernel_ptr;
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
    fn getc() -> Option<u8> {
        Riscv64::getc()
    }
    fn stash_rx(byte: u8) {
        Riscv64::stash_rx(byte);
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
        // The virt-machine success path uses the PASS command.  A nonzero
        // code keeps the same upper-16-bit encoding as panic_exit.
        let value = if code == 0 {
            0x5555u32
        } else {
            ((code as u32) << 16) | SIFIVE_TEST_FAIL
        };
        unsafe {
            core::ptr::write_volatile(SIFIVE_TEST as *mut u32, value);
        }
        Riscv64::halt()
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

    fn flush_va_asid(&self, asid: u16, va: u64, pages: usize, targets: u32) {
        TLB_VA_FLUSH.fetch_add(1, Ordering::Relaxed);
        let mut page = 0;
        while page < pages {
            unsafe {
                core::arch::asm!("sfence.vma {}, {}", in(reg) va + (page as u64) * 4096, in(reg) asid as u64);
            }
            page += 1;
        }
        if !BKL.is_held() {
            return;
        }
        let me = current_hart();
        #[cfg(not(feature = "test-skip-tlb-ipi"))]
        for target in 0..MAX_HARTS {
            if target == me || !maintenance_target_online(target) {
                continue;
            }
            if targets & (1 << target) == 0 {
                TLB_IPI_SKIPPED.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            if send_tlb_va(target, asid, va, pages) {
                TLB_IPI_SENT.fetch_add(1, Ordering::Relaxed);
            }
        }
        #[cfg(feature = "test-skip-tlb-ipi")]
        let _ = (me, targets);
    }

    fn flush_all_asid(&self, asid: u16, targets: u32) {
        TLB_VA_FLUSH.fetch_add(1, Ordering::Relaxed);
        let _ = asid;
        unsafe {
            core::arch::asm!("sfence.vma zero, zero");
        }
        if !BKL.is_held() {
            return;
        }
        let me = current_hart();
        for target in 0..MAX_HARTS {
            if target == me || !maintenance_target_online(target) {
                continue;
            }
            if targets & (1 << target) == 0 {
                TLB_IPI_SKIPPED.fetch_add(1, Ordering::Relaxed);
            } else if send_maintenance(target, MAINT_TLB) {
                TLB_IPI_SENT.fetch_add(1, Ordering::Relaxed);
            }
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

/// TCP echo server thường trú (vòng 79): trạng thái nằm trong static, được
/// bơm từ IRQ timer nên phục vụ bất kỳ lúc nào guest còn sống, không cần cửa
/// sổ chờ trong init_block. Mỗi tick xử lý tối đa 2 segment với bound nhỏ để
/// IRQ không bị giữ lâu.
struct TcpServer {
    conn: bool,
    client_ip: [u8; 4],
    client_port: u16,
    client_seq: u32,
    our_seq: u32,
    echoes: u32,
    accepts: u32,
}

static mut TCP_SERVER: TcpServer = TcpServer {
    conn: false,
    client_ip: [0; 4],
    client_port: 0,
    client_seq: 0,
    our_seq: 0x5150_0000,
    echoes: 0,
    accepts: 0,
};

const TCP_PORT: u16 = 8080;
const TCP_IRQ_TX_BOUND: u64 = 250_000;

/// Khởi tạo server một lần trong init_block: lấy ISN từ rng (nếu có), nạp
/// buffer RX và in marker để harness/test biết server đã sẵn sàng.
fn tcp_init() {
    if !vifast_virtio::net_ready() {
        return;
    }
    let mut seed = [0u8; 8];
    let isn = if vifast_virtio::rng_read(&mut seed) == 8 {
        u32::from_le_bytes([seed[0], seed[1], seed[2], seed[3]])
    } else {
        0x5150_0000
    };
    unsafe {
        let server = &mut *core::ptr::addr_of_mut!(TCP_SERVER);
        server.our_seq = isn;
    }
    vifast_virtio::net_rx_refill(8);
    vifast_kernel::klog!(RiscvPlatform, "VIRTIO TCP LISTEN OK port={}", TCP_PORT);
}

/// Một nhịp phục vụ TCP, gọi từ timer IRQ hart 0.
fn tcp_pump() {
    if !vifast_virtio::net_ready() {
        return;
    }
    let mut payload = [0u8; 512];
    let mut budget = 2;
    while budget > 0 {
        budget -= 1;
        let Some(seg) = vifast_virtio::net_recv_tcp_bounded(TCP_PORT, &mut payload, 2_000) else {
            break;
        };
        let server = unsafe { &mut *core::ptr::addr_of_mut!(TCP_SERVER) };
        if !server.conn {
            if seg.flags & vifast_virtio::TCP_SYN != 0 && seg.flags & vifast_virtio::TCP_ACK == 0 {
                server.client_ip = seg.src_ip;
                server.client_port = seg.src_port;
                server.client_seq = seg.seq.wrapping_add(1);
                server.conn = true;
                let _ = vifast_virtio::net_send_tcp_bounded(
                    server.client_ip,
                    server.client_port,
                    TCP_PORT,
                    server.our_seq,
                    server.client_seq,
                    vifast_virtio::TCP_SYN | vifast_virtio::TCP_ACK,
                    &[],
                    TCP_IRQ_TX_BOUND,
                );
                server.our_seq = server.our_seq.wrapping_add(1);
                server.accepts = server.accepts.wrapping_add(1);
                vifast_kernel::klog!(RiscvPlatform, "VIRTIO TCP ACCEPT");
            }
            continue;
        }
        if seg.flags & vifast_virtio::TCP_RST != 0 {
            server.conn = false;
            vifast_kernel::klog!(RiscvPlatform, "VIRTIO TCP RESET");
            continue;
        }
        if seg.flags & vifast_virtio::TCP_ACK != 0 {
            let len = seg.payload_len as u32;
            if len > 0 && seg.seq == server.client_seq {
                server.client_seq = server.client_seq.wrapping_add(len);
                let _ = vifast_virtio::net_send_tcp_bounded(
                    server.client_ip,
                    server.client_port,
                    TCP_PORT,
                    server.our_seq,
                    server.client_seq,
                    vifast_virtio::TCP_ACK | vifast_virtio::TCP_PSH,
                    &payload[..len as usize],
                    TCP_IRQ_TX_BOUND,
                );
                server.our_seq = server.our_seq.wrapping_add(len);
                server.echoes = server.echoes.wrapping_add(1);
            }
            if seg.flags & vifast_virtio::TCP_FIN != 0 {
                server.client_seq = server.client_seq.wrapping_add(1);
                let _ = vifast_virtio::net_send_tcp_bounded(
                    server.client_ip,
                    server.client_port,
                    TCP_PORT,
                    server.our_seq,
                    server.client_seq,
                    vifast_virtio::TCP_ACK | vifast_virtio::TCP_FIN,
                    &[],
                    TCP_IRQ_TX_BOUND,
                );
                server.conn = false;
                vifast_kernel::klog!(RiscvPlatform, "VIRTIO TCP CLOSE");
            }
        }
    }
}

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
        init_plic();
        // Pha 4 (vòng 54-56): probe DTB header read-only. Vòng 56 đọc thêm
        // version (off 20) + strings_off (off 12) — cùng 7 word header đã kiểm
        // chứng an toàn, không chạm struct block (treo vòng 55).
        if info.dtb != 0 {
            let magic = unsafe { core::ptr::read_volatile(info.dtb as *const u32) };
            vifast_kernel::klog!(RiscvPlatform, "DTB magic={:#x}", u32::from_be(magic));
            if u32::from_be(magic) == 0xd00dfeed {
                let dtb = info.dtb;
                let rd = |off: u64| unsafe {
                    u32::from_be(core::ptr::read_volatile((dtb + off) as *const u32))
                };
                vifast_kernel::klog!(
                    RiscvPlatform,
                    "FDT size={} struct_off={} strings_off={} version={}",
                    rd(4),
                    rd(8),
                    rd(12),
                    rd(20)
                );
            }
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

    fn secondary_timer_ticks() -> usize {
        SECONDARY_TIMER_TICKS.load(Ordering::Relaxed)
    }

    fn timer_rate() -> Option<(usize, usize)> {
        let base = BSP_RATE_BASE.load(Ordering::Acquire);
        Some((
            SECONDARY_TIMER_TICKS.load(Ordering::Relaxed),
            BSP_TIMER_TICKS.load(Ordering::Relaxed).saturating_sub(base),
        ))
    }

    fn tlb_stats() -> Option<(usize, usize, usize)> {
        Some((
            TLB_VA_FLUSH.load(Ordering::Relaxed),
            TLB_IPI_SENT.load(Ordering::Relaxed),
            TLB_IPI_SKIPPED.load(Ordering::Relaxed),
        ))
    }

    fn boot_stack_usage() -> Option<(usize, usize)> {
        extern "C" {
            static __stack_bottom: u8;
            static __stack_top: u8;
        }
        let bottom = core::ptr::addr_of!(__stack_bottom) as usize;
        let top = core::ptr::addr_of!(__stack_top) as usize;
        let mut cursor = bottom;
        while cursor < top {
            if unsafe { core::ptr::read_volatile(cursor as *const u8) } != 0xA5 {
                return Some((top - cursor, top - bottom));
            }
            cursor += 1;
        }
        Some((0, top - bottom))
    }

    fn secondary_user_ticks() -> usize {
        SECONDARY_USER_TICKS.load(Ordering::Relaxed)
    }

    fn secondary_preempts() -> usize {
        SECONDARY_PREEMPTS.load(Ordering::Relaxed)
    }

    fn secondary_idle_entries() -> usize {
        IDLE_ENTRIES.load(Ordering::Relaxed)
    }

    fn secondary_none_wakes() -> usize {
        SECONDARY_NONE_WAKES.load(Ordering::Relaxed)
    }

    fn secondary_wait_blocked() {
        let count = SECONDARY_WAIT_BLOCKS.fetch_add(1, Ordering::SeqCst) + 1;
        if count == 2 {
            POST_TIMER_TICKS.store(0, Ordering::Relaxed);
            POST_TIMER_WAKES.store(0, Ordering::Relaxed);
            POST_IDLE_ENTRIES.store(0, Ordering::Relaxed);
            POST_STARTED.store(true, Ordering::SeqCst);
        }
    }

    fn post_idle_stats() -> Option<(usize, usize, usize)> {
        if !POST_STARTED.load(Ordering::SeqCst) {
            return None;
        }
        Some((
            POST_TIMER_TICKS.load(Ordering::Relaxed),
            POST_IDLE_ENTRIES.load(Ordering::Relaxed),
            POST_TIMER_WAKES.load(Ordering::Relaxed),
        ))
    }

    fn post_idle_ready() -> bool {
        if !POST_STARTED.load(Ordering::SeqCst) {
            return false;
        }
        let ticks = POST_TIMER_TICKS.load(Ordering::Relaxed);
        ticks >= 5
            && POST_IDLE_ENTRIES.load(Ordering::Relaxed) >= ticks.saturating_sub(2)
            && POST_TIMER_WAKES.load(Ordering::Relaxed) >= ticks.saturating_sub(2)
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

    fn sched_harts() -> u32 {
        0b11
    }

    fn can_wait_for_work() -> bool {
        true
    }

    fn boot_handoff_from() -> bool {
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!(__boot_handoff_from)) != 0 }
    }

    fn wait_for_work(hart: usize) {
        loop {
            if vifast_arch::panic::is_panicking() {
                Riscv64::halt();
            }
            poll_maintenance();
            if RiscvPlatform::kernel().has_ready_on(hart) {
                return;
            }
            IDLE_HARTS[hart].store(true, Ordering::SeqCst);
            if RiscvPlatform::kernel().has_ready_on(hart) {
                IDLE_HARTS[hart].store(false, Ordering::SeqCst);
                return;
            }
            if POST_STARTED.load(Ordering::Relaxed) {
                POST_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
            }
            unsafe {
                core::arch::asm!("wfi");
            }
            IDLE_HARTS[hart].store(false, Ordering::SeqCst);
            RESCHED_PENDING[hart].store(false, Ordering::SeqCst);
            let sip: u64;
            unsafe {
                core::arch::asm!("csrr {}, sip", out(reg) sip);
            }
            record_secondary_wake(sip);
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

    fn bkl_stats() -> [(usize, usize); 2] {
        [BKL.stats(0), BKL.stats(1)]
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
        // Pha 4 (vòng 52-53): probe fw_cfg read-only (QEMU virt: 0x10100000).
        // Vòng 53 thử đọc file dir (selector 0x19) nhưng treo sau ROOT ELF
        // (rc=-9) — ghi selector MMIO có side-effect chưa rõ. Revert về
        // sig-only (vòng 52 xanh); nghiên cứu fw_cfg DMA/select kỹ hơn.
        {
            let fw_base = 0x1010_0000u64;
            let sig = unsafe { core::ptr::read_volatile(fw_base as *const u32) };
            vifast_kernel::klog!(RiscvPlatform, "FWCFG base={:#x} sig={:#x}", fw_base, sig);
        }
        // Vòng 66 (M4 display): dò + khởi tạo virtio-gpu (device id 16),
        // tạo resource 2D 320x240, set scanout và vẽ dải màu để QEMU
        // screendump kiểm chứng được pixel thật.
        vifast_virtio::scan_devices(0x1000_1000, 0x1000_9000, 0x1000, |base, device| {
            if device == 16 {
                let gver = unsafe { core::ptr::read_volatile((base + 4) as *const u32) };
                vifast_kernel::klog!(RiscvPlatform, "VIRTIO GPU base={:#x} ver={}", base, gver);
                if vifast_virtio::gpu_init(base) {
                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO GPU READY");
                    if vifast_virtio::gpu_draw_test_pattern() {
                        let (fb, w, h) = vifast_virtio::gpu_framebuffer_info();
                        vifast_kernel::klog!(
                            RiscvPlatform,
                            "VIRTIO GPU DRAW OK fb={:#x} {}x{}",
                            fb,
                            w,
                            h
                        );
                    } else {
                        vifast_kernel::klog!(RiscvPlatform, "VIRTIO GPU DRAW FAIL");
                    }
                } else {
                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO GPU INIT FAIL");
                }
            }
        });
        // Vòng 70 (M4 input): virtio-input (bàn phím). Thiết bị nhận sendkey
        // từ QEMU monitor; driver đẩy ký tự vào RxQueue để shell đọc.
        vifast_virtio::scan_devices(0x1000_1000, 0x1000_9000, 0x1000, |base, device| {
            if device == vifast_virtio::DEVICE_INPUT {
                let iver = unsafe { core::ptr::read_volatile((base + 4) as *const u32) };
                vifast_kernel::klog!(RiscvPlatform, "VIRTIO INPUT base={:#x} ver={}", base, iver);
                if vifast_virtio::input_init(base) {
                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO INPUT READY base={:#x}", base);
                } else {
                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO INPUT INIT FAIL");
                }
            }
        });
        {
            let (qaddr, qsize) = vifast_virtio::net_queue_info();
            vifast_kernel::klog!(
                RiscvPlatform,
                "VIRTIO NETQ addr={:#x} size={}",
                qaddr,
                qsize
            );
        }
        for base in [0x1000_1000u64, 0x1000_7000, 0x1000_8000] {
            let magic = unsafe { core::ptr::read_volatile(base as *const u32) };
            let device = unsafe { core::ptr::read_volatile((base + 8) as *const u32) };
            vifast_kernel::klog!(
                RiscvPlatform,
                "VIRTIO PRE base={:#x} magic={:#x} device={}",
                base,
                magic,
                device
            );
            if device == vifast_virtio::DEVICE_NET {
                if let Some(mac) = vifast_virtio::net_mac(base) {
                    let feats = vifast_virtio::net_features(base).unwrap_or(0);
                    vifast_kernel::klog!(
                        RiscvPlatform,
                        "VIRTIO NET base={:#x} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} feats={:#x}",
                        base,
                        mac[0],
                        mac[1],
                        mac[2],
                        mac[3],
                        mac[4],
                        mac[5],
                        feats
                    );
                    // Vòng 46: init TX queue cho net (device riêng, chưa claim).
                    // Vòng 48: HOÃN post RX sau DRIVER_OK — QEMU treo/timeout
                    // khi gắn queue 0 sau khi TX đã DRIVER_OK (rc=-9 sau
                    // SELFTEST). Giữ TX-only, RX chờ vòng sau (cấu hình cả 2
                    // queue trước DRIVER_OK).
                    // Vòng 49: net_init_tx giờ gắn cả RX+TX trước DRIVER_OK;
                    // chờ reply trên RX đã post sẵn.
                    match vifast_virtio::net_init_tx(base) {
                        Some(acc) => {
                            vifast_kernel::klog!(
                                RiscvPlatform,
                                "VIRTIO NET READY base={:#x} accepted={:#x}",
                                base,
                                acc
                            );
                            // Vòng 47: gửi ARP request đầu tiên qua TX.
                            // Vòng 63: gửi ARP cho gateway 10.0.2.2 và DNS
                            // 10.0.2.3 để xem slirp đáp địa chỉ nào.
                            for (label, tpa) in [("GW", [10u8, 0, 2, 2]), ("DNS", [10u8, 0, 2, 3])]
                            {
                                match vifast_virtio::net_send_arp_to(tpa) {
                                    Some(len) => vifast_kernel::klog!(
                                        RiscvPlatform,
                                        "VIRTIO NET ARP {} sent len={}",
                                        label,
                                        len
                                    ),
                                    None => vifast_kernel::klog!(
                                        RiscvPlatform,
                                        "VIRTIO NET ARP {} FAIL",
                                        label
                                    ),
                                }
                            }
                            // Vòng 63: ICMP echo tới gateway — slirp xử lý IP trực tiếp.
                            // Đây là bằng chứng TX+RX end-to-end của stack tối thiểu.
                            vifast_virtio::net_rx_rearm();
                            match vifast_virtio::net_ping([10u8, 0, 2, 2]) {
                                Some(len) => vifast_kernel::klog!(
                                    RiscvPlatform,
                                    "VIRTIO NET PING sent len={}",
                                    len
                                ),
                                None => vifast_kernel::klog!(RiscvPlatform, "VIRTIO NET PING FAIL"),
                            }
                            match vifast_virtio::net_recv_icmp() {
                                Some(src) => vifast_kernel::klog!(
                                    RiscvPlatform,
                                    "VIRTIO NET PING REPLY from {}.{}.{}.{}",
                                    src[0],
                                    src[1],
                                    src[2],
                                    src[3]
                                ),
                                None => {
                                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO NET PING NO REPLY")
                                }
                            }
                            // Vòng 64: DNS query A cho example.com qua slirp
                            // (10.0.2.3:53) — chứng minh UDP hai chiều.
                            let mut dns = [0u8; 32];
                            dns[0] = 0x12;
                            dns[1] = 0x34;
                            dns[2] = 0x01; // RD
                            dns[5] = 0x01; // QDCOUNT = 1
                            let qname = b"\x07example\x03com\x00";
                            dns[12..12 + qname.len()].copy_from_slice(qname);
                            let qoff = 12 + qname.len();
                            dns[qoff..qoff + 2].copy_from_slice(&[0x00, 0x01]); // QTYPE A
                            dns[qoff + 2..qoff + 4].copy_from_slice(&[0x00, 0x01]); // QCLASS IN
                            match vifast_virtio::net_send_udp([10, 0, 2, 3], 53, &dns[..qoff + 4]) {
                                Some(len) => vifast_kernel::klog!(
                                    RiscvPlatform,
                                    "VIRTIO NET DNS sent len={}",
                                    len
                                ),
                                None => vifast_kernel::klog!(RiscvPlatform, "VIRTIO NET DNS FAIL"),
                            }
                            match vifast_virtio::net_recv_dns_a([10, 0, 2, 3]) {
                                Some(ip) => vifast_kernel::klog!(
                                    RiscvPlatform,
                                    "VIRTIO NET DNS A {}.{}.{}.{}",
                                    ip[0],
                                    ip[1],
                                    ip[2],
                                    ip[3]
                                ),
                                None => {
                                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO NET DNS NO REPLY")
                                }
                            }
                        }
                        None => {
                            vifast_kernel::klog!(RiscvPlatform, "VIRTIO NET FAIL base={:#x}", base)
                        }
                    }
                }
            }
        }
        // Vòng 79: TCP echo server thường trú — chỉ khởi tạo trạng thái ở
        // đây, việc phục vụ chạy trong timer IRQ nên không chặn boot.
        tcp_init();
        // Vòng 75: virtio-rng (entropy source, device id 4).
        vifast_virtio::scan_devices(0x1000_1000, 0x1000_9000, 0x1000, |base, device| {
            if device == vifast_virtio::DEVICE_RNG {
                if vifast_virtio::rng_init(base) {
                    let mut a = [0u8; 8];
                    let mut b = [0u8; 8];
                    let na = vifast_virtio::rng_read(&mut a);
                    let nb = vifast_virtio::rng_read(&mut b);
                    if na == 8 && nb == 8 && a != b {
                        vifast_kernel::klog!(
                            RiscvPlatform,
                            "VIRTIO RNG OK {:02x}{:02x}{:02x}{:02x}",
                            a[0],
                            a[1],
                            a[2],
                            a[3]
                        );
                    } else {
                        vifast_kernel::klog!(RiscvPlatform, "VIRTIO RNG BAD");
                    }
                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO RNG READY base={:#x}", base);
                } else {
                    vifast_kernel::klog!(RiscvPlatform, "VIRTIO RNG INIT FAIL");
                }
            }
        });
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
        // Pha 3 (vòng 39-40): scan API sẵn sàng; chưa log từng slot vì MMIO
        // sau init cần xác minh thêm (treo khi gắn net). Giữ init_range blk.
    }
    fn init_kernel() -> &'static mut Kernel {
        unsafe {
            let kernel = &mut *core::ptr::addr_of_mut!(KERNEL_STORAGE);
            kernel.reset();
            kernel
        }
    }

    fn set_kernel(kernel: *mut Kernel) {
        // Safety: single boot-time install before secondary harts start.
        unsafe {
            *KERNEL_PTR.0.get() = kernel;
        }
    }

    fn kernel() -> &'static mut Kernel {
        // Safety: installed once at boot and never replaced.
        unsafe { &mut **KERNEL_PTR.0.get() }
    }

    fn boot_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BOOT_POOL) as *mut u8, 131072)
        }
    }

    fn data_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_DATA) as *mut u8, 311296)
        }
    }

    fn metadata_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(METADATA) as *mut u8, 81920)
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
            core::arch::asm!("csrs sie, {}", in(reg) (1u64 << 5) | (1u64 << 9));
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
        context.regs[17] = 0;
    }

    fn write_badge(context: &mut TrapFrame, badge: u64) {
        context.regs[17] = badge;
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

    fn block_flush() -> bool {
        vifast_virtio::flush()
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
            BSP_RATE_BASE.store(BSP_TIMER_TICKS.load(Ordering::Acquire), Ordering::Release);
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
    static __boot_handoff_from: u8;
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
    if vifast_arch::panic::is_panicking() {
        Riscv64::irq_disable();
        Riscv64::halt();
    }
    let frame = unsafe { &mut *frame };
    // Count every return to user, not only the first enter_user.
    if frame.sstatus & (1 << 8) == 0 {
        count_user_entry();
    }
    if frame.scause == 0x8000_0000_0000_0001 {
        let resched = MAINT_KIND[1].load(Ordering::Acquire) & MAINT_RESCHEDULE != 0;
        unsafe {
            core::arch::asm!("csrc sip, {}", in(reg) 1u64 << 1);
        }
        poll_maintenance();
        if resched {
            RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
        }
        return;
    }
    if frame.scause == 8 && frame.regs[17] == vifast_abi::SyscallId::GetTime as u64 {
        frame.regs[10] = 0;
        frame.regs[11] = RiscvPlatform::kernel().fast_time();
        frame.sepc = frame.sepc.wrapping_add(4);
        return;
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb();
    if frame.scause == 0x8000_0000_0000_0005 {
        let hart = RiscvPlatform::hart_id();
        let user = frame.sstatus & (1 << 8) == 0;
        if hart == 1 {
            SECONDARY_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
            if user {
                SECONDARY_USER_TICKS.fetch_add(1, Ordering::Relaxed);
            }
        } else if hart == 0 {
            BSP_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
            poll_virtio_input();
            tcp_pump();
        }
        let changed = vifast_runtime::timer::on_timer::<RiscvPlatform>(frame);
        if hart == 1 && user && changed {
            SECONDARY_PREEMPTS.fetch_add(1, Ordering::Relaxed);
        }
        drop(bkl);
        assert_bkl_released();
        return;
    }
    if frame.scause == 0x8000_0000_0000_0009 {
        let irq = plic_claim();
        if irq == PLIC_UART_IRQ {
            drain_uart_rx();
            let woke = RiscvPlatform::kernel().irq_raise(vifast_abi::IRQ_UART);
            if woke && RiscvPlatform::hart_id() == 0 && RiscvPlatform::kernel().has_ready_on(1) {
                RiscvPlatform::wake_hart(1);
            }
        }
        if irq != 0 {
            plic_complete(irq);
        }
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
    let waiting = vifast_runtime::syscall::dispatch::<RiscvPlatform>(frame, number, args);
    drop(bkl);
    if waiting {
        loop {
            RiscvPlatform::wait_for_work(1);
            let bkl = bkl_lock();
            assert_current_tcb();
            if vifast_runtime::syscall::resume_blocked::<RiscvPlatform>(frame) {
                drop(bkl);
                assert_bkl_released();
                return;
            }
            drop(bkl);
        }
    }
    assert_bkl_released();
}
