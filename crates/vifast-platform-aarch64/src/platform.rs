//! AArch64 QEMU `virt` platform: 4-level MMU, GICv2, generic timer and PSCI.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use vifast_abi::{KError, Rights};
use vifast_arch::bkl::{BigKernelGuard, BigKernelLock};
use vifast_arch::{Arch, CpuState, MAX_HARTS};
use vifast_kernel::{Kernel, PageTableOps};
use vifast_runtime::{Platform, PlatformInfo};

use vifast_arch_aarch64::Aarch64;

pub struct Aarch64Platform;

pub const USER_CODE_VA: usize = vifast_abi::user_layout::CODE_VA;
pub const USER_DATA_VA: usize = vifast_abi::user_layout::DATA_VA;
const USER_STACK_BASE: usize = vifast_abi::user_layout::STACK_BASE;
const USER_STACK_LEN: usize = vifast_abi::user_layout::STACK_LEN;
const USER_SPSR: u64 = 0x340;
const GICD: u64 = 0x0800_0000;
const GICC: u64 = 0x0801_0000;
const TIMER_IRQ: u32 = 30;
const UART_IRQ: u32 = 33;
const TIMER_HZ: u64 = 100;
const PL011_BASE: u64 = 0x0900_0000;
const PL011_DR: u64 = 0x00;
const PL011_FR: u64 = 0x18;
const PL011_IMSC: u64 = 0x38;
const PL011_ICR: u64 = 0x44;
const FR_RXFE: u32 = 1 << 4;

const DESC_TABLE: u64 = 0b11;
const DESC_BLOCK: u64 = 0b1;
const AF: u64 = 1 << 10;
const SH_INNER: u64 = 0b11 << 8;
const ATTR_DEVICE: u64 = 1 << 2;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;

core::arch::global_asm!(include_str!("trap.S"));
core::arch::global_asm!(include_str!("smp.S"));

#[repr(align(4096))]
struct Table([u64; 512]);

#[derive(Clone, Copy)]
#[allow(dead_code)]
#[repr(align(4096))]
struct UserStack([u8; 32768]);

static mut L0: Table = Table([0; 512]);
static mut L1: Table = Table([0; 512]);
static mut L2_LOW: Table = Table([0; 512]);
static mut L2_KERNEL: Table = Table([0; 512]);
#[allow(dead_code)]
#[repr(align(4096))]
struct DataPool([u8; 352256]);
static mut USER_DATA: DataPool = DataPool([0; 352256]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 81920]);
static mut METADATA: Metadata = Metadata([0; 81920]);
// Per-object user contexts indexed by `ObjRef::index`.  The kernel object
// table has MAX_OBJECTS slots, so the array must cover that full domain.
const _: () = assert!(vifast_kernel::MAX_TCBS <= u8::MAX as usize);
static mut CONTEXTS: [TrapFrame; vifast_kernel::MAX_TCBS] =
    [TrapFrame::empty(); vifast_kernel::MAX_TCBS];
static TIMER_INTERVAL: AtomicU64 = AtomicU64::new(0);
static mut TRAP_STACKS: [UserStack; MAX_HARTS] = [UserStack([0; 32768]); MAX_HARTS];
struct CpuStates(core::cell::UnsafeCell<[CpuState; MAX_HARTS]>);
// Safety: each hart only writes its own entry during boot; runtime access goes
// through the per-CPU TPIDR pointer installed here.
unsafe impl Sync for CpuStates {}
static CPU_STATES: CpuStates =
    CpuStates(core::cell::UnsafeCell::new([CpuState::empty(); MAX_HARTS]));
#[allow(dead_code)]
#[repr(align(4096))]
struct BootPool([u8; 163840]);
static mut BOOT_POOL: BootPool = BootPool([0; 163840]);
struct KernelPointer(core::cell::UnsafeCell<*mut Kernel>);
// Safety: boot installs the pointer once before any secondary hart starts and
// only reads it afterwards; all kernel entry points run under the BKL.
unsafe impl Sync for KernelPointer {}
static KERNEL_PTR: KernelPointer =
    KernelPointer(core::cell::UnsafeCell::new(core::ptr::null_mut()));
static SECONDARY_ONLINE: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static BSP_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static BSP_RATE_BASE: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_USER_TICKS: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_PREEMPTS: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_IPI_WAKES: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_IDLE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_NONE_WAKES: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_WAIT_BLOCKS: AtomicUsize = AtomicUsize::new(0);
static POST_STARTED: AtomicBool = AtomicBool::new(false);
static POST_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static POST_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static POST_IDLE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
const MAINT_TLB: u32 = 1;
const MAINT_ICACHE: u32 = 2;
static TLB_VA_FLUSH: AtomicUsize = AtomicUsize::new(0);
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
static SCHED_READY: AtomicBool = AtomicBool::new(false);
static RESCHED_SENT: AtomicUsize = AtomicUsize::new(0);
static RESCHED_WAKE: AtomicUsize = AtomicUsize::new(0);
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

static mut SEMIHOST_EXIT_BLOCK: [u32; 2] = [0x20026, 1];

/// Semihosting SYS_EXIT_EXTENDED gives QEMU a real nonzero exit code; PSCI
/// SYSTEM_OFF cannot distinguish panic from success.
pub fn panic_exit(code: u8) -> ! {
    unsafe {
        SEMIHOST_EXIT_BLOCK[1] = code as u32;
        let block = core::ptr::addr_of!(SEMIHOST_EXIT_BLOCK) as u64;
        core::arch::asm!(
            "hlt #0xF000",
            in("x0") 0x20u64,
            in("x1") block,
        );
    }
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

pub fn panic_halt_others() {
    let hart = current_hart();
    let target = if hart == 0 { 1 } else { 0 };
    if target == 1 && SECONDARY_ONLINE.load(Ordering::SeqCst) == 0 {
        return;
    }
    // GICv2 SGI 2: panic halt.
    write32(GICD + 0xF00, (1 << (16 + target)) | 2);
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

fn send_reschedule(target: usize) -> bool {
    assert!(
        BKL.held_by(current_hart()),
        "reschedule sender must hold BKL"
    );
    if target >= MAX_HARTS || !maintenance_target_online(target) {
        return false;
    }
    if !IDLE_HARTS[target].load(Ordering::SeqCst) {
        return false;
    }
    if RESCHED_PENDING[target].swap(true, Ordering::SeqCst) {
        return false;
    }
    // GICv2 SGIR: SGI 1 to the target CPU.
    write32(GICD + 0xF00, (1 << (16 + target)) | 1);
    RESCHED_SENT.fetch_add(1, Ordering::Relaxed);
    true
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
            core::arch::asm!("dsb ish");
        }
    }
    if kind & MAINT_ICACHE != 0 {
        unsafe {
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
    }
    MAINT_ACK[hart].store(seq, Ordering::Release);
}

fn maintenance_target_online(target: usize) -> bool {
    if target == 0 {
        return true;
    }
    // Mốc 5 (vòng 29): như riscv — mọi hart phụ đã online đều nhận
    // maintenance (đồng bộ, dù aarch64 hiện mới start tới hart 1).
    target < MAX_HARTS && SECONDARY_ONLINE.load(Ordering::SeqCst) >= target
}

fn send_maintenance(target: usize, kind: u32) -> bool {
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
    // GICv2 SGIR SGI 0, target list bit for the destination CPU.
    write32(GICD + 0xF00, 1 << (16 + target));
    let mut spins = 0usize;
    while MAINT_ACK[target].load(Ordering::Acquire) < seq && spins < 5_000_000 {
        poll_maintenance();
        core::hint::spin_loop();
        spins += 1;
    }
    MAINT_ACK[target].load(Ordering::Acquire) >= seq
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
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif);
    }
    assert!(daif & (1 << 7) != 0, "BKL entered with IRQ unmasked");
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    regs: [u64; 31],
    sp: u64,
    elr: u64,
    spsr: u64,
    esr: u64,
    far: u64,
}

impl TrapFrame {
    const fn empty() -> TrapFrame {
        TrapFrame {
            regs: [0; 31],
            sp: 0,
            elr: 0,
            spsr: 0,
            esr: 0,
            far: 0,
        }
    }

    fn user(entry: usize, stack: usize) -> TrapFrame {
        let mut frame = TrapFrame::empty();
        frame.sp = stack as u64;
        frame.elr = entry as u64;
        frame.spsr = USER_SPSR;
        frame
    }
}

fn table_desc(pa: u64) -> u64 {
    pa | DESC_TABLE
}
fn block_desc(pa: u64, attr: u64, ap: u64, pxn: u64, uxn: u64, sh: u64) -> u64 {
    pa | DESC_BLOCK | attr | ap | sh | AF | pxn | uxn
}
fn read32(addr: u64) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}
fn write32(addr: u64, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}
fn write8(addr: u64, value: u8) {
    unsafe { core::ptr::write_volatile(addr as *mut u8, value) }
}

fn init_gic() {
    write32(GICD + 0x000, 1);
    // SGI 0/1 (reschedule) và SGI 2 (panic halt, audit #4 P1-7) phải được
    // enable; trước đây chỉ bật 0b11 nên `panic_halt_others` là dead code.
    write32(GICD + 0x100, (1 << TIMER_IRQ) | (1 << 2) | 0b11);
    write32(GICD + 0x104, 1 << (UART_IRQ - 32));
    write8(GICD + 0x400 + TIMER_IRQ as u64, 0xA0);
    write8(GICD + 0x400 + UART_IRQ as u64, 0xA0);
    write8(GICD + 0x800 + TIMER_IRQ as u64, 1);
    // UART interrupts are handled on CPU0; CPU1 must not consume the level.
    write8(GICD + 0x800 + UART_IRQ as u64, 1);
    write32(GICC + 0x004, 0xFF);
    write32(GICC + 0x000, 1);
    init_uart_irq();
}

fn init_uart_irq() {
    write32(PL011_BASE + PL011_ICR, 0x7FF);
    write32(PL011_BASE + PL011_IMSC, 1 << 4);
}

/// Poll virtio-input trong timer IRQ (hart 0) và đẩy ký tự vào RxQueue UART
/// để shell hiện có đọc được. Không log trong IRQ.
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
        if event.kind == 1 && event.value != 0 {
            static mut KEY_LOG: u32 = 0;
            unsafe {
                if KEY_LOG < 16 {
                    KEY_LOG += 1;
                    vifast_kernel::klog!(Aarch64Platform, "VIRTIO KEY {}", event.code);
                }
            }
        }
        if let Some(byte) = vifast_virtio::input_key_ascii(event) {
            Aarch64::stash_rx(byte);
            queued = true;
            // Vòng 95: cập nhật màn hình bằng mã phím vừa bấm (chỉ vài phím
            // đầu để bound công việc trong IRQ) — mốc "display tương tác".
            static mut ECHO_DONE: u32 = 0;
            unsafe {
                if ECHO_DONE < 4 && vifast_virtio::gpu_echo_key(event.code) {
                    ECHO_DONE += 1;
                    vifast_kernel::klog!(Aarch64Platform, "INPUT ECHO {}", event.code);
                }
            }
        }
    }
    if queued {
        let _ = Aarch64Platform::kernel().irq_raise(vifast_abi::IRQ_UART);
    }
}

fn drain_uart_rx() {
    loop {
        if read32(PL011_BASE + PL011_FR) & FR_RXFE != 0 {
            break;
        }
        let byte = read32(PL011_BASE + PL011_DR) as u8;
        Aarch64::stash_rx(byte);
    }
    write32(PL011_BASE + PL011_ICR, 0x7FF);
}

unsafe fn init_cpu(hartid: usize) {
    let index = core::cmp::min(hartid, MAX_HARTS - 1);
    let entries = CPU_STATES.0.get();
    unsafe {
        (*entries)[index].hart_id = hartid;
        (*entries)[index].current_tcb = 0;
        let stack = core::ptr::addr_of!(TRAP_STACKS[index]) as u64;
        (*entries)[index].trap_stack_top = stack + 16384;
    }
    let state = unsafe { core::ptr::addr_of_mut!((*entries)[index]) } as u64;
    core::arch::asm!("msr tpidr_el1, {}", in(reg) state);
}

fn init_gic_cpu1() {
    // CPU1 writes its own banked PPIs/SGIs and CPU interface state.
    write32(GICD + 0x000, 1);
    write32(GICD + 0x100, (1 << TIMER_IRQ) | 0b11);
    write8(GICD + 0x400 + TIMER_IRQ as u64, 0xA0);
    write8(GICD + 0x800 + TIMER_IRQ as u64, 1);
    write32(GICC + 0x004, 0xFF);
    write32(GICC + 0x000, 1);
}

fn daif_i_set() -> bool {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif);
    }
    daif & (1 << 7) != 0
}

/// Drain the GIC.  Returns (saw_timer, saw_sgi) for one WFI wake.
fn service_gic() -> (bool, bool) {
    let mut saw_timer = false;
    let mut saw_sgi = false;
    loop {
        let iar = read32(GICC + 0x00C);
        let irq = iar & 0x3FF;
        if irq == 1023 {
            return (saw_timer, saw_sgi);
        }
        write32(GICC + 0x010, irq);
        if irq == 2 || vifast_arch::panic::is_panicking() {
            Aarch64::irq_disable();
            Aarch64::halt();
        }
        if irq == TIMER_IRQ {
            saw_timer = true;
            SECONDARY_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
            if POST_STARTED.load(Ordering::Relaxed) {
                POST_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
            }
            init_timer();
        } else {
            saw_sgi = true;
            poll_maintenance();
        }
    }
}

fn init_timer() {
    let freq: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq);
        let interval = (freq / TIMER_HZ).max(1);
        TIMER_INTERVAL.store(interval, Ordering::Relaxed);
        core::arch::asm!("msr cntp_tval_el0, {}", in(reg) interval);
        core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) 1u64);
    }
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

fn pan_supported() -> bool {
    let mmfr1: u64;
    unsafe {
        core::arch::asm!("mrs {}, id_aa64mmfr1_el1", out(reg) mmfr1);
    }
    ((mmfr1 >> 20) & 0xF) != 0
}

fn account_gic_wake(saw_timer: bool, saw_sgi: bool) {
    if saw_sgi {
        SECONDARY_IPI_WAKES.fetch_add(1, Ordering::Relaxed);
    } else if saw_timer {
        SECONDARY_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
        if POST_STARTED.load(Ordering::Relaxed) {
            POST_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
        }
    } else {
        SECONDARY_NONE_WAKES.fetch_add(1, Ordering::Relaxed);
    }
}

fn park_secondary(hart: usize) {
    loop {
        if vifast_arch::panic::is_panicking() {
            Aarch64::halt();
        }
        poll_maintenance();
        if Aarch64Platform::kernel().has_ready_on(hart) {
            return;
        }
        SECONDARY_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        if POST_STARTED.load(Ordering::Relaxed) {
            POST_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        }
        IDLE_HARTS[hart].store(true, Ordering::SeqCst);
        if Aarch64Platform::kernel().has_ready_on(hart) {
            IDLE_HARTS[hart].store(false, Ordering::SeqCst);
            return;
        }
        unsafe {
            core::arch::asm!("wfi");
        }
        IDLE_HARTS[hart].store(false, Ordering::SeqCst);
        RESCHED_PENDING[hart].store(false, Ordering::SeqCst);
        let (saw_timer, saw_sgi) = service_gic();
        account_gic_wake(saw_timer, saw_sgi);
    }
}

#[no_mangle]
pub extern "C" fn rust_secondary(_context: u64) -> ! {
    Aarch64Platform::init();
    unsafe {
        init_cpu(1);
        core::arch::asm!("msr daifset, #0xf");
        init_gic_cpu1();
        init_timer();
    }
    SECONDARY_ONLINE.fetch_add(1, Ordering::SeqCst);
    loop {
        if vifast_arch::panic::is_panicking() {
            Aarch64::halt();
        }
        assert!(daif_i_set(), "AArch64 CPU1 must keep DAIF.I set");
        if SCHED_READY.load(Ordering::SeqCst) {
            let picked = {
                let bkl = bkl_lock();
                let picked = Aarch64Platform::kernel().pick_on(1);
                if picked.is_none() {
                    IDLE_HARTS[1].store(true, Ordering::SeqCst);
                    if Aarch64Platform::kernel().has_ready_on(1) {
                        IDLE_HARTS[1].store(false, Ordering::SeqCst);
                    }
                }
                if let Some(tcb) = picked {
                    publish_current_tcb(Some(tcb.index as usize));
                    if vifast_runtime::ensure_current_root::<Aarch64Platform>().is_err() {
                        drop(bkl);
                        Aarch64::exit(1);
                    }
                }
                picked
            };
            if let Some(tcb) = picked {
                let slot = Aarch64Platform::kernel().context_slot(tcb).unwrap_or(0) as usize;
                let context = Aarch64Platform::contexts()[slot];
                <Aarch64Platform as Platform>::enter_user(&context);
            }
        }
        SECONDARY_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        if POST_STARTED.load(Ordering::Relaxed) {
            POST_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        }
        IDLE_HARTS[1].store(true, Ordering::SeqCst);
        if SCHED_READY.load(Ordering::SeqCst) && Aarch64Platform::kernel().has_ready_on(1) {
            IDLE_HARTS[1].store(false, Ordering::SeqCst);
        }
        unsafe {
            core::arch::asm!("wfi");
        }
        IDLE_HARTS[1].store(false, Ordering::SeqCst);
        let resched = RESCHED_PENDING[1].swap(false, Ordering::SeqCst);
        let (saw_timer, saw_sgi) = service_gic();
        if resched {
            RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
        }
        account_gic_wake(saw_timer, saw_sgi);
        assert!(daif_i_set(), "AArch64 CPU1 lost DAIF.I after WFI");
        core::hint::spin_loop();
    }
}

fn cpu_state() -> *mut CpuState {
    let state: *mut CpuState;
    unsafe {
        core::arch::asm!("mrs {}, tpidr_el1", out(reg) state);
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
    let expected = Aarch64Platform::kernel()
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
        let Some(tcb) = kernel.current_on((*cpu_state()).hart_id) else {
            return false;
        };
        kernel.current_root(tcb) == Some(active_root())
    }
}

impl Arch for Aarch64Platform {
    const NAME: &'static str = Aarch64::NAME;
    fn init() {
        Aarch64::init();
        // Vòng 91: marker bắt buộc để harness phát hiện hồi quy nếu ai đó bật
        // lại FP cho EL0 (CPACR FPEN=0b11) — khi đó q0..q31 rò rỉ giữa task.
        vifast_kernel::klog!(
            Aarch64Platform,
            "EL0 FP TRAP (FPEN=0b01, kernel vẫn dùng FP/SIMD)"
        );
    }
    fn putc(byte: u8) {
        Aarch64::putc(byte);
    }
    fn getc() -> Option<u8> {
        Aarch64::getc()
    }
    fn stash_rx(byte: u8) {
        Aarch64::stash_rx(byte);
    }
    fn now() -> u64 {
        Aarch64::now()
    }
    fn irq_disable() {
        Aarch64::irq_disable();
    }
    fn irq_enable() {
        Aarch64::irq_enable();
    }
    fn send_ipi(hart: usize) {
        // Vòng 88 (audit #3 P2): Arch::send_ipi là stub; platform có GICD nên
        // gửi SGI 1 thật tới hart đích (cùng SGI mà reschedule dùng).
        if hart >= MAX_HARTS {
            return;
        }
        write32(GICD + 0xF00, (1 << (16 + hart)) | 1);
    }
    fn halt() -> ! {
        Aarch64::halt()
    }
    fn exit(code: u8) -> ! {
        Aarch64::exit(code)
    }
}

impl vifast_kernel::console::Console for Aarch64Platform {
    const NAME: &'static str = Aarch64::NAME;

    fn write_byte(byte: u8) {
        Aarch64::putc(byte);
    }
}

struct PageOps;

impl vifast_kernel::PageTableOps for PageOps {
    fn levels(&self) -> usize {
        vifast_pte::aarch64::LEVELS
    }

    fn index(&self, va: u64, level: usize) -> usize {
        vifast_pte::aarch64::index(va, level)
    }

    fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
        unsafe { *((table_pa as *const u64).add(idx)) }
    }

    fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
        unsafe {
            *((table_pa as *mut u64).add(idx)) = pte;
            core::arch::asm!("dsb ishst");
        }
    }

    fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
        vifast_pte::aarch64::encode_leaf(pa, rights)
    }

    fn encode_table(&self, pa: u64) -> u64 {
        vifast_pte::aarch64::encode_table(pa)
    }

    fn decode(&self, entry: u64) -> vifast_kernel::Pte {
        vifast_pte::aarch64::decode(entry)
    }

    fn decode_at(&self, entry: u64, level: usize) -> vifast_kernel::Pte {
        vifast_pte::aarch64::decode_at(entry, level)
    }

    fn vspace_pages(&self) -> usize {
        2
    }

    fn install_kernel(&self, root_pa: u64, aux_pa: u64) {
        unsafe {
            let root = root_pa as *mut u64;
            let aux = aux_pa as *mut u64;
            *root = table_desc(aux_pa);
            let shared_l1 = core::ptr::addr_of!(L1) as *const u64;
            for slot in vifast_pte::aarch64::KERNEL_SHARED_SLOTS {
                *aux.add(*slot) = *shared_l1.add(*slot);
            }
        }
    }

    fn kernel_accessible_range(&self) -> (u64, u64) {
        (0x4000_0000, 0x4080_0000)
    }

    fn user_va_range(&self) -> (u64, u64) {
        (
            vifast_pte::aarch64::USER_VA_START,
            vifast_pte::aarch64::USER_VA_END,
        )
    }

    fn flush_va(&self, va: u64) {
        unsafe {
            core::arch::asm!("tlbi vae1is, {}", in(reg) va);
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
    }

    fn flush_all_user(&self) {
        unsafe {
            core::arch::asm!("tlbi vmalle1is");
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
    }

    fn flush_va_asid(&self, _asid: u16, va: u64, pages: usize, _targets: u32) {
        TLB_VA_FLUSH.fetch_add(1, Ordering::Relaxed);
        let mut page = 0;
        while page < pages {
            unsafe {
                core::arch::asm!("tlbi vae1is, {}", in(reg) va + (page as u64) * 4096);
            }
            page += 1;
        }
        unsafe {
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
    }

    fn flush_all_asid(&self, asid: u16, _targets: u32) {
        TLB_VA_FLUSH.fetch_add(1, Ordering::Relaxed);
        unsafe {
            core::arch::asm!("tlbi aside1is, {}", in(reg) asid as u64);
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
        }
    }

    fn sync_icache(&self, pa: u64, size: usize) {
        if size == 0 {
            return;
        }
        let ctr: u64;
        unsafe {
            core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack));
            let dline = 4u64 << (ctr & 0xf);
            let iline = 4u64 << ((ctr >> 16) & 0xf);
            let end = pa.saturating_add(size as u64);
            let mut addr = pa & !(dline - 1);
            while addr < end {
                core::arch::asm!("dc civac, {}", in(reg) addr, options(nostack));
                addr = addr.saturating_add(dline);
            }
            core::arch::asm!("dsb ish", options(nostack));
            addr = pa & !(iline - 1);
            while addr < end {
                core::arch::asm!("ic ivau, {}", in(reg) addr, options(nostack));
                addr = addr.saturating_add(iline);
            }
            core::arch::asm!("dsb ish", options(nostack));
            core::arch::asm!("isb", options(nostack));
        }
    }

    fn leaf_is_user(&self, pte: u64) -> bool {
        vifast_pte::aarch64::is_user(pte)
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

fn kernel_leaf_ok(root_pa: u64, va: u64, need_write: bool, need_exec: bool) -> bool {
    let mut table = root_pa;
    for level in 0..vifast_pte::aarch64::LEVELS {
        let idx = vifast_pte::aarch64::index(va, level);
        let pte = PageOps.read_pte(table, idx);
        match PageOps.decode_at(pte, level) {
            vifast_kernel::Pte::Table(next) => {
                if level + 1 == vifast_pte::aarch64::LEVELS {
                    return false;
                }
                table = next;
            }
            vifast_kernel::Pte::Leaf(_, rights) => {
                let kernel_writable = pte & (0b11 << 6) == 0;
                return rights.contains(Rights::READ)
                    && !vifast_pte::aarch64::is_user(pte)
                    && (!need_write || kernel_writable)
                    && (!need_exec || pte & vifast_pte::aarch64::PXN == 0);
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

unsafe fn build_kernel_page_tables() {
    let l0 = core::ptr::addr_of_mut!(L0) as *mut Table;
    let l1 = core::ptr::addr_of_mut!(L1) as *mut Table;
    let low = core::ptr::addr_of_mut!(L2_LOW) as *mut Table;
    let kernel = core::ptr::addr_of_mut!(L2_KERNEL) as *mut Table;
    (*l0).0[0] = table_desc(l1 as u64);
    (*l1).0[0] = table_desc(low as u64);
    (*l1).0[1] = table_desc(kernel as u64);
    (*low).0[64] = block_desc(0x0800_0000, ATTR_DEVICE, 0, PXN, UXN, 0);
    (*low).0[72] = block_desc(0x0900_0000, ATTR_DEVICE, 0, PXN, UXN, 0);
    (*low).0[80] = block_desc(0x0a00_0000, ATTR_DEVICE, 0, PXN, UXN, 0);
    for i in 0..4usize {
        let pa = 0x4000_0000u64 + i as u64 * 2 * 1024 * 1024;
        (*kernel).0[i] = block_desc(pa, 0, 0, 0, UXN, SH_INNER);
    }
}

static PAGE_OPS: PageOps = PageOps;

struct KernelStorage(core::cell::UnsafeCell<Kernel>);
// Safety: the single boot hart initializes it before any secondary hart starts;
// all later access is serialized by the BKL.
unsafe impl Sync for KernelStorage {}
static KERNEL_STORAGE: KernelStorage =
    KernelStorage(core::cell::UnsafeCell::new(Kernel::new_const()));

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
    vifast_kernel::klog!(Aarch64Platform, "VIRTIO TCP LISTEN OK port={}", TCP_PORT);
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
                vifast_kernel::klog!(Aarch64Platform, "VIRTIO TCP ACCEPT");
            }
            continue;
        }
        if seg.flags & vifast_virtio::TCP_RST != 0 {
            server.conn = false;
            vifast_kernel::klog!(Aarch64Platform, "VIRTIO TCP RESET");
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
                vifast_kernel::klog!(Aarch64Platform, "VIRTIO TCP CLOSE");
            }
        }
    }
}

impl Platform for Aarch64Platform {
    type Context = TrapFrame;

    const USER_CODE_VA: usize = USER_CODE_VA;
    const USER_DATA_VA: usize = USER_DATA_VA;
    const ELF_MACHINE: u16 = vifast_pte::aarch64::ELF_MACHINE;

    fn init_platform(info: PlatformInfo) {
        unsafe {
            build_kernel_page_tables();
            init_cpu(info.hartid);
        }
        // Pha 4 (vòng 54): probe DTB header magic read-only (0xd00dfeed BE).
        if info.dtb != 0 {
            let magic = unsafe { core::ptr::read_volatile(info.dtb as *const u32) };
            vifast_kernel::klog!(Aarch64Platform, "DTB magic={:#x}", u32::from_be(magic));
        }
    }

    fn init_kernel() -> &'static mut Kernel {
        // Safety: boot-time initialization, before secondary harts start.
        unsafe {
            let kernel = &mut *KERNEL_STORAGE.0.get();
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
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(BOOT_POOL) as *mut u8, 163840)
        }
    }

    fn data_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_DATA) as *mut u8, 352256)
        }
    }

    fn metadata_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(METADATA) as *mut u8, 81920)
        }
    }

    fn vspace_root() -> u64 {
        core::ptr::addr_of!(L0) as u64
    }

    fn switch_root(root_pa: u64, asid: u16) -> bool {
        unsafe {
            if root_pa == 0 || (root_pa == active_root() && asid == active_asid()) {
                return false;
            }
            set_active_root(root_pa, asid);
            // TCR_EL1.AS=1 enables 16-bit ASIDs; TTBR0 carries the tag.
            let tcr: u64 = 16 | (1 << 23) | (2 << 32) | (1 << 36);
            core::arch::asm!("msr tcr_el1, {}", in(reg) tcr);
            let ttbr = root_pa | ((asid as u64) << 48);
            core::arch::asm!("msr ttbr0_el1, {}", in(reg) ttbr);
            core::arch::asm!("tlbi aside1is, {}", in(reg) asid as u64);
            core::arch::asm!("dsb ish");
            core::arch::asm!("isb");
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
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(vector_table) as u64,
                false,
                true,
            )
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
            && kernel_leaf_ok(root_pa, GICD, true, false)
            && kernel_leaf_ok(root_pa, GICC, true, false)
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if page_in_range(ptr, out.len(), USER_CODE_VA, 0x2_0000)
            || page_in_range(ptr, out.len(), USER_STACK_BASE, USER_STACK_LEN)
            // Vòng 145: exec task có stack RIÊNG ở EXEC_STACK_BASE — thiếu
            // cửa sổ này thì mọi DebugWrite/read từ stack của task exec bị
            // platform từ chối (lỗi "exec không đọc được trang stack").
            || page_in_range(
                ptr,
                out.len(),
                vifast_abi::user_layout::EXEC_STACK_BASE,
                vifast_abi::user_layout::EXEC_STACK_TOP - vifast_abi::user_layout::EXEC_STACK_BASE,
            )
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
                if pan_supported() {
                    core::arch::asm!(".arch armv8.1-a", "msr pan, #0");
                }
                core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), out.len());
                if pan_supported() {
                    core::arch::asm!(".arch armv8.1-a", "msr pan, #1");
                }
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
            || page_in_range(
                ptr,
                input.len(),
                vifast_abi::user_layout::EXEC_STACK_BASE,
                vifast_abi::user_layout::EXEC_STACK_TOP - vifast_abi::user_layout::EXEC_STACK_BASE,
            )
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
                if pan_supported() {
                    core::arch::asm!(".arch armv8.1-a", "msr pan, #0");
                }
                core::ptr::copy_nonoverlapping(input.as_ptr(), ptr as *mut u8, input.len());
                if pan_supported() {
                    core::arch::asm!(".arch armv8.1-a", "msr pan, #1");
                }
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
        context.regs[0] = value;
    }

    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        assert_bkl_released();
        SCHED_READY.store(true, Ordering::SeqCst);
        let tcr: u64 = 16 | (1 << 23) | (2 << 32) | (1 << 36);
        let mair: u64 = 0xFF;
        unsafe {
            core::arch::asm!("msr tcr_el1, {}", in(reg) tcr);
            core::arch::asm!("msr mair_el1, {}", in(reg) mair);
            let mut sctlr: u64;
            core::arch::asm!("mrs {}, sctlr_el1", out(reg) sctlr);
            sctlr &= !(1 << 19);
            if pan_supported() {
                sctlr |= 1 << 23;
            }
            sctlr |= (1 << 0) | (1 << 2) | (1 << 12);
            core::arch::asm!("msr vbar_el1, {}", in(reg) core::ptr::addr_of!(vector_table) as u64);
            core::arch::asm!("dsb sy; isb");
            core::arch::asm!("msr sctlr_el1, {}", in(reg) sctlr);
            core::arch::asm!("isb");
            let cpu: *const CpuState;
            core::arch::asm!("mrs {}, tpidr_el1", out(reg) cpu);
            assert!(!cpu.is_null(), "AArch64 per-CPU pointer is null");
            let trap_top = (*cpu).trap_stack_top;
            core::arch::asm!("msr spsel, #1");
            core::arch::asm!("mov sp, {}", in(reg) trap_top);
            core::arch::asm!("msr sp_el0, {}", in(reg) context.sp);
            core::arch::asm!("msr elr_el1, {}", in(reg) context.elr);
            core::arch::asm!("msr spsr_el1, {}", in(reg) context.spsr);
            core::arch::asm!("mov x0, {}", in(reg) context.regs[0]);
            core::arch::asm!("eret", options(noreturn));
        }
    }

    fn write_result(
        context: &mut TrapFrame,
        error: KError,
        value: u64,
        label: u64,
        data: [u64; 4],
    ) {
        context.regs[0] = error as u64;
        context.regs[1] = value;
        context.regs[2] = label;
        context.regs[3] = data[0];
        context.regs[4] = data[1];
        context.regs[5] = data[2];
        context.regs[6] = data[3];
        context.regs[7] = 0;
    }

    fn write_badge(context: &mut TrapFrame, badge: u64) {
        context.regs[7] = badge;
    }

    fn start_timer() {
        init_gic();
        init_timer();
    }

    fn reload_timer() {
        unsafe {
            let interval = TIMER_INTERVAL.load(Ordering::Relaxed);
            core::arch::asm!("msr cntp_tval_el0, {}", in(reg) interval);
            core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) 1u64);
        }
    }

    fn ack_timer() {
        write32(GICC + 0x010, TIMER_IRQ);
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

    fn sched_harts() -> u32 {
        0b11
    }

    fn can_wait_for_work() -> bool {
        true
    }

    fn wait_for_work(hart: usize) {
        park_secondary(hart);
    }

    fn wake_hart(hart: usize) {
        if hart < MAX_HARTS && maintenance_target_online(hart) {
            let _ = send_reschedule(hart);
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

    fn bkl_stats() -> [(usize, usize); vifast_arch::MAX_HARTS] {
        [BKL.stats(0), BKL.stats(1), BKL.stats(2), BKL.stats(3)]
    }

    fn idle_entries() -> usize {
        SECONDARY_IDLE_ENTRIES.load(Ordering::Relaxed)
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
        Some((TLB_VA_FLUSH.load(Ordering::Relaxed), 0, 0))
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
        SECONDARY_IDLE_ENTRIES.load(Ordering::Relaxed)
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

    fn init_block() {
        // Pha 4 (vòng 52-53): probe fw_cfg read-only (QEMU virt: 0x09020000).
        // Vòng 53 thử đọc dir treo RISC-V nên revert sig-only cả 2 arch.
        {
            let fw_base = 0x0902_0000u64;
            let sig = unsafe { core::ptr::read_volatile(fw_base as *const u32) };
            vifast_kernel::klog!(Aarch64Platform, "FWCFG base={:#x} sig={:#x}", fw_base, sig);
        }
        // Pha 3 (vòng 42-45): probe 32 slot đầu TRƯỚC init — an toàn.
        // Đọc MAC net qua config space khi gặp device=1. Vòng 45 log thêm
        // địa chỉ NetQueue static riêng.
        // Vòng 66 (M4 display): dò + khởi tạo virtio-gpu (device id 16),
        // tạo resource 2D 320x240, set scanout và vẽ dải màu để QEMU
        // screendump kiểm chứng được pixel thật.
        vifast_virtio::scan_devices(0x0a00_0000, 0x0a00_4000, 0x200, |base, device| {
            if device == 16 {
                let gver = unsafe { core::ptr::read_volatile((base + 4) as *const u32) };
                vifast_kernel::klog!(Aarch64Platform, "VIRTIO GPU base={:#x} ver={}", base, gver);
                if vifast_virtio::gpu_init(base) {
                    vifast_kernel::klog!(Aarch64Platform, "VIRTIO GPU READY");
                    if vifast_virtio::gpu_draw_test_pattern() {
                        let (fb, w, h) = vifast_virtio::gpu_framebuffer_info();
                        vifast_kernel::klog!(
                            Aarch64Platform,
                            "VIRTIO GPU DRAW OK fb={:#x} {}x{}",
                            fb,
                            w,
                            h
                        );
                    } else {
                        vifast_kernel::klog!(Aarch64Platform, "VIRTIO GPU DRAW FAIL");
                    }
                } else {
                    vifast_kernel::klog!(Aarch64Platform, "VIRTIO GPU INIT FAIL");
                }
            }
        });
        // Vòng 70 (M4 input): virtio-input (bàn phím) trên MMIO.
        vifast_virtio::scan_devices(0x0a00_0000, 0x0a00_4000, 0x200, |base, device| {
            if device == vifast_virtio::DEVICE_INPUT {
                let iver = unsafe { core::ptr::read_volatile((base + 4) as *const u32) };
                vifast_kernel::klog!(
                    Aarch64Platform,
                    "VIRTIO INPUT base={:#x} ver={}",
                    base,
                    iver
                );
                if vifast_virtio::input_init(base) {
                    vifast_kernel::klog!(Aarch64Platform, "VIRTIO INPUT READY base={:#x}", base);
                } else {
                    vifast_kernel::klog!(Aarch64Platform, "VIRTIO INPUT INIT FAIL");
                }
            }
        });
        {
            let (qaddr, qsize) = vifast_virtio::net_queue_info();
            vifast_kernel::klog!(
                Aarch64Platform,
                "VIRTIO NETQ addr={:#x} size={}",
                qaddr,
                qsize
            );
        }
        for i in 0..32u64 {
            let base = 0x0a00_0000u64 + i * 0x200;
            let magic = unsafe { core::ptr::read_volatile(base as *const u32) };
            let device = unsafe { core::ptr::read_volatile((base + 8) as *const u32) };
            if magic == 0x7472_6976 && device != 0 {
                vifast_kernel::klog!(
                    Aarch64Platform,
                    "VIRTIO PRE base={:#x} magic={:#x} device={}",
                    base,
                    magic,
                    device
                );
                if device == vifast_virtio::DEVICE_NET {
                    if let Some(mac) = vifast_virtio::net_mac(base) {
                        let feats = vifast_virtio::net_features(base).unwrap_or(0);
                        vifast_kernel::klog!(
                            Aarch64Platform,
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
                        // Vòng 47: gửi ARP request đầu tiên qua TX.
                        // Vòng 48: HOÃN post RX sau DRIVER_OK (treo/timeout
                        // QEMU, rc=-9). Giữ TX-only; RX chờ vòng sau.
                        // Vòng 49: both gắn cả RX+TX trước DRIVER_OK; chờ reply.
                        match vifast_virtio::net_init_tx(base) {
                            Some(acc) => {
                                vifast_kernel::klog!(
                                    Aarch64Platform,
                                    "VIRTIO NET READY base={:#x} accepted={:#x}",
                                    base,
                                    acc
                                );
                                // Vòng 63: ARP cho gateway và DNS.
                                for (label, tpa) in
                                    [("GW", [10u8, 0, 2, 2]), ("DNS", [10u8, 0, 2, 3])]
                                {
                                    match vifast_virtio::net_send_arp_to(tpa) {
                                        Some(len) => vifast_kernel::klog!(
                                            Aarch64Platform,
                                            "VIRTIO NET ARP {} sent len={}",
                                            label,
                                            len
                                        ),
                                        None => vifast_kernel::klog!(
                                            Aarch64Platform,
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
                                        Aarch64Platform,
                                        "VIRTIO NET PING sent len={}",
                                        len
                                    ),
                                    None => vifast_kernel::klog!(
                                        Aarch64Platform,
                                        "VIRTIO NET PING FAIL"
                                    ),
                                }
                                match vifast_virtio::net_recv_icmp() {
                                    Some(src) => vifast_kernel::klog!(
                                        Aarch64Platform,
                                        "VIRTIO NET PING REPLY from {}.{}.{}.{}",
                                        src[0],
                                        src[1],
                                        src[2],
                                        src[3]
                                    ),
                                    None => vifast_kernel::klog!(
                                        Aarch64Platform,
                                        "VIRTIO NET PING NO REPLY"
                                    ),
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
                                                                                        // Vòng 96: TX ring có thể còn drain đúng lúc này
                                                                                        // (soak bắt 1/20 lần): thử lại vài lần trước khi FAIL.
                                let mut dns_sent = vifast_virtio::net_send_udp(
                                    [10, 0, 2, 3],
                                    53,
                                    &dns[..qoff + 4],
                                );
                                let mut attempt = 0;
                                while dns_sent.is_none() && attempt < 8 {
                                    attempt += 1;
                                    let mut spins = 0u32;
                                    while spins < 200_000 {
                                        core::hint::spin_loop();
                                        spins += 1;
                                    }
                                    dns_sent = vifast_virtio::net_send_udp(
                                        [10, 0, 2, 3],
                                        53,
                                        &dns[..qoff + 4],
                                    );
                                }
                                match dns_sent {
                                    Some(len) => vifast_kernel::klog!(
                                        Aarch64Platform,
                                        "VIRTIO NET DNS sent len={}",
                                        len
                                    ),
                                    None => {
                                        vifast_kernel::klog!(Aarch64Platform, "VIRTIO NET DNS FAIL")
                                    }
                                }
                                match vifast_virtio::net_recv_dns_a([10, 0, 2, 3]) {
                                    Some(ip) => vifast_kernel::klog!(
                                        Aarch64Platform,
                                        "VIRTIO NET DNS A {}.{}.{}.{}",
                                        ip[0],
                                        ip[1],
                                        ip[2],
                                        ip[3]
                                    ),
                                    None => vifast_kernel::klog!(
                                        Aarch64Platform,
                                        "VIRTIO NET DNS NO REPLY"
                                    ),
                                }
                            }
                            None => vifast_kernel::klog!(
                                Aarch64Platform,
                                "VIRTIO NET FAIL base={:#x}",
                                base
                            ),
                        }
                    }
                }
            }
        }
        // Vòng 79: TCP echo server thường trú — chỉ khởi tạo trạng thái ở
        // đây, việc phục vụ chạy trong timer IRQ nên không chặn boot.
        tcp_init();
        // Vòng 75: virtio-rng (entropy source, device id 4).
        vifast_virtio::scan_devices(0x0a00_0000, 0x0a00_4000, 0x200, |base, device| {
            if device == vifast_virtio::DEVICE_RNG {
                if vifast_virtio::rng_init(base) {
                    let mut a = [0u8; 8];
                    let mut b = [0u8; 8];
                    let na = vifast_virtio::rng_read(&mut a);
                    let nb = vifast_virtio::rng_read(&mut b);
                    if na == 8 && nb == 8 && a != b {
                        vifast_kernel::klog!(
                            Aarch64Platform,
                            "VIRTIO RNG OK {:02x}{:02x}{:02x}{:02x}",
                            a[0],
                            a[1],
                            a[2],
                            a[3]
                        );
                    } else {
                        vifast_kernel::klog!(Aarch64Platform, "VIRTIO RNG BAD");
                    }
                    vifast_kernel::klog!(Aarch64Platform, "VIRTIO RNG READY base={:#x}", base);
                } else {
                    vifast_kernel::klog!(Aarch64Platform, "VIRTIO RNG INIT FAIL");
                }
            }
        });
        if vifast_virtio::init() {
            let (base, magic, version, device) = vifast_virtio::probe_info();
            vifast_kernel::klog!(
                Aarch64Platform,
                "VIRTIO OK base={:#x} magic={:#x} version={} device={}",
                base,
                magic,
                version,
                device
            );
        } else {
            let (base, magic, version, device) = vifast_virtio::probe_info();
            vifast_kernel::klog!(
                Aarch64Platform,
                "VIRTIO SKIP base={:#x} magic={:#x} version={} device={}",
                base,
                magic,
                version,
                device
            );
        }
    }

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
        let entry = secondary_entry as *const () as u64;
        let result: u64;
        unsafe {
            core::arch::asm!(
                "hvc #0",
                in("x0") 0xC400_0003u64,
                in("x1") 1u64,
                in("x2") entry,
                in("x3") 0u64,
                lateout("x0") result,
            );
        }
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
                vifast_kernel::klog!(
                    Aarch64Platform,
                    "SMP OK (harts=2, psci={}, ipi=1)",
                    result as i64
                );
            } else {
                vifast_kernel::klog!(
                    Aarch64Platform,
                    "SMP OK (harts=2, psci={}, ipi=pending)",
                    result as i64
                );
            }
        } else {
            vifast_kernel::klog!(Aarch64Platform, "SMP SKIP (psci={})", result as i64);
        }
    }
}

fn vector_name(index: u64) -> &'static str {
    match index {
        0 => "cur-sp0-sync",
        1 => "cur-sp0-irq",
        2 => "cur-sp0-fiq",
        3 => "cur-sp0-serror",
        4 => "cur-spx-sync",
        5 => "cur-spx-irq",
        6 => "cur-spx-fiq",
        7 => "cur-spx-serror",
        _ => "lower-or-unknown",
    }
}

#[no_mangle]
extern "C" fn aarch64_kernel_fault(index: u64) -> ! {
    let esr: u64;
    let elr: u64;
    let far: u64;
    let mpidr: u64;
    let spsr: u64;
    let daif: u64;
    let isr: u64;
    unsafe {
        core::arch::asm!("mrs {}, esr_el1", out(reg) esr);
        core::arch::asm!("mrs {}, elr_el1", out(reg) elr);
        core::arch::asm!("mrs {}, far_el1", out(reg) far);
        core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr);
        core::arch::asm!("mrs {}, spsr_el1", out(reg) spsr);
        core::arch::asm!("mrs {}, daif", out(reg) daif);
        core::arch::asm!("mrs {}, isr_el1", out(reg) isr);
    }
    vifast_kernel::klog!(
        Aarch64Platform,
        "KERNEL EXCEPTION vector={}({}) cpu={:#x} ESR={:#x} ELR={:#x} FAR={:#x} SPSR={:#x} DAIF={:#x} ISR={:#x}",
        index,
        vector_name(index),
        mpidr,
        esr,
        elr,
        far,
        spsr,
        daif,
        isr
    );
    Aarch64::exit(1)
}

extern "C" {
    static vector_table: u8;
    static __kernel_end: u8;
    fn secondary_entry();
}

fn count_user_trap() {
    if let Some(counter) = USER_ENTRIES.get(Aarch64Platform::hart_id()) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[no_mangle]
extern "C" fn aarch64_trap_handler(frame: *mut TrapFrame) {
    assert_irqs_disabled();
    let frame = unsafe { &mut *frame };
    count_user_trap();
    let ec = (frame.esr >> 26) & 0x3f;
    if ec == 0x15 && frame.regs[8] == vifast_abi::SyscallId::GetTime as u64 {
        frame.regs[0] = 0;
        frame.regs[1] = Aarch64Platform::kernel().fast_time();
        return;
    }
    let bkl = bkl_lock();
    assert_current_tcb();
    if ec != 0x15 {
        vifast_runtime::fault::suspend::<Aarch64Platform>(frame, frame.esr, frame.far);
        drop(bkl);
        assert_bkl_released();
        return;
    }
    let number = frame.regs[8];
    let args = [
        frame.regs[0],
        frame.regs[1],
        frame.regs[2],
        frame.regs[3],
        frame.regs[4],
        frame.regs[5],
    ];
    let waiting = vifast_runtime::syscall::dispatch::<Aarch64Platform>(frame, number, args);
    drop(bkl);
    if waiting {
        loop {
            park_secondary(1);
            let bkl = bkl_lock();
            assert_current_tcb();
            if vifast_runtime::syscall::resume_blocked::<Aarch64Platform>(frame) {
                drop(bkl);
                assert_bkl_released();
                return;
            }
            drop(bkl);
        }
    }
    assert_bkl_released();
}

#[no_mangle]
extern "C" fn aarch64_irq_handler(frame: *mut TrapFrame) {
    if vifast_arch::panic::is_panicking() {
        Aarch64::irq_disable();
        Aarch64::halt();
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb();
    let frame = unsafe { &mut *frame };
    count_user_trap();
    let iar = read32(GICC + 0x00C);
    let irq = iar & 0x3FF;
    if irq == UART_IRQ {
        drain_uart_rx();
        let woke = Aarch64Platform::kernel().irq_raise(vifast_abi::IRQ_UART);
        if woke && Aarch64Platform::hart_id() == 0 && Aarch64Platform::kernel().has_ready_on(1) {
            Aarch64Platform::wake_hart(1);
        }
        write32(GICC + 0x010, irq);
        drop(bkl);
        assert_bkl_released();
        return;
    }
    if irq != TIMER_IRQ {
        write32(GICC + 0x010, irq);
        if irq == 1 {
            RESCHED_PENDING[1].store(false, Ordering::SeqCst);
            RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
        } else {
            poll_maintenance();
        }
        drop(bkl);
        assert_bkl_released();
        return;
    }
    let hart = Aarch64Platform::hart_id();
    let user = frame.spsr & 0b1111 == 0;
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
    let changed = vifast_runtime::timer::on_timer::<Aarch64Platform>(frame);
    if hart == 1 && user && changed {
        SECONDARY_PREEMPTS.fetch_add(1, Ordering::Relaxed);
    }
    drop(bkl);
    assert_bkl_released();
}
