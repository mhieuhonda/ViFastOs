//! RISC-V QEMU `virt` platform: Sv39 MMU, SBI timer, trap entry and SMP.

use core::sync::atomic::{AtomicUsize, Ordering};

use vifast_abi::{KError, Rights};
use vifast_arch::Arch;
use vifast_kernel::Kernel;
use vifast_runtime::{Platform, PlatformInfo};

use vifast_arch_riscv64::Riscv64;

pub struct RiscvPlatform;

pub const USER_CODE_VA: usize = vifast_abi::user_layout::CODE_VA;
pub const USER_STACK_A_TOP: usize = vifast_abi::user_layout::STACK_A_TOP;
pub const USER_STACK_B_TOP: usize = vifast_abi::user_layout::STACK_B_TOP;
pub const USER_DATA_VA: usize = vifast_abi::user_layout::DATA_VA;
pub const USER_STACK_BASE: usize = vifast_abi::user_layout::STACK_BASE;
pub const USER_STACK_LEN: usize = vifast_abi::user_layout::STACK_LEN;
const USER_SSTATUS: u64 = 1 << 5;
const TIMER_INTERVAL: u64 = 100_000;
const PTE_V: u64 = 1;
const PTE_R: u64 = 2;
const PTE_W: u64 = 4;
const PTE_X: u64 = 8;
const PTE_U: u64 = 16;

core::arch::global_asm!(include_str!("trap.S"));
core::arch::global_asm!(include_str!("smp.S"));

#[repr(align(4096))]
struct Table([u64; 512]);

#[allow(dead_code)]
#[repr(align(4096))]
struct Page([u8; 4096]);

#[allow(dead_code)]
#[repr(align(4096))]
struct UserStack([u8; 32768]);

static mut ROOT: Table = Table([0; 512]);
static mut KERNEL_L1: Table = Table([0; 512]);
static mut DEVICE_L1: Table = Table([0; 512]);
static mut USER_L1: Table = Table([0; 512]);
static mut USER_L0: Table = Table([0; 512]);
static mut USER_CODE: Page = Page([0; 4096]);
static mut USER_STACK: UserStack = UserStack([0; 32768]);
static mut USER_DATA: Page = Page([0; 4096]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 65536]);
static mut METADATA: Metadata = Metadata([0; 65536]);
static mut CONTEXTS: [TrapFrame; 32] = [TrapFrame::empty(); 32];
static mut KERNEL_PTR: *mut Kernel = core::ptr::null_mut();
#[allow(dead_code)]
#[repr(align(16))]
struct TrapStack([u8; 32768]);
static mut TRAP_STACK: TrapStack = TrapStack([0; 32768]);
static SECONDARY_ONLINE: AtomicUsize = AtomicUsize::new(0);

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
pub extern "C" fn rust_secondary(_hartid: usize) -> ! {
    SECONDARY_ONLINE.fetch_add(1, Ordering::SeqCst);
    loop {
        unsafe {
            core::arch::asm!("wfi");
        }
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

impl Platform for RiscvPlatform {
    type Context = TrapFrame;

    const USER_CODE_VA: usize = USER_CODE_VA;
    const USER_DATA_VA: usize = USER_DATA_VA;

    fn stack_a_top() -> usize {
        USER_STACK_A_TOP
    }
    fn stack_b_top() -> usize {
        USER_STACK_B_TOP
    }

    fn init_platform(_info: PlatformInfo) {}

    fn set_kernel(kernel: *mut Kernel) {
        unsafe {
            KERNEL_PTR = kernel;
        }
    }

    fn kernel() -> &'static mut Kernel {
        unsafe { &mut *KERNEL_PTR }
    }

    fn code_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_CODE) as *mut u8, 4096)
        }
    }

    fn data_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_DATA) as *mut u8, 4096)
        }
    }

    fn metadata_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(METADATA) as *mut u8, 65536)
        }
    }

    fn build_page_tables() {
        unsafe {
            let root = core::ptr::addr_of_mut!(ROOT) as *mut Table;
            let kernel_l1 = core::ptr::addr_of_mut!(KERNEL_L1) as *mut Table;
            let device_l1 = core::ptr::addr_of_mut!(DEVICE_L1) as *mut Table;
            let user_l1 = core::ptr::addr_of_mut!(USER_L1) as *mut Table;
            let user_l0 = core::ptr::addr_of_mut!(USER_L0) as *mut Table;

            (*root).0[0] = pte(table_pa(device_l1), PTE_V);
            /* 64 GiB shared user window. */
            (*root).0[64] = pte(table_pa(user_l1), PTE_V);
            (*root).0[2] = pte(table_pa(kernel_l1), PTE_V);
            (*kernel_l1).0[0] = pte(0x8000_0000, PTE_V | PTE_R | PTE_W | PTE_X);
            (*kernel_l1).0[1] = pte(0x8020_0000, PTE_V | PTE_R | PTE_W | PTE_X);
            (*device_l1).0[128] = pte(0x1000_0000, PTE_V | PTE_R | PTE_W);
            (*user_l1).0[0] = pte(table_pa(user_l0), PTE_V);

            let code_pa = core::ptr::addr_of!(USER_CODE) as u64;
            let stack_pa = core::ptr::addr_of!(USER_STACK) as u64;
            /* W^X: code is RX; stack is RW+NX. */
            (*user_l0).0[0] = pte(code_pa, PTE_V | PTE_R | PTE_X | PTE_U);
            for i in 0..8usize {
                (*user_l0).0[1 + i] =
                    pte(stack_pa + (i as u64) * 4096, PTE_V | PTE_R | PTE_W | PTE_U);
            }
        }
    }

    fn vspace_root() -> u64 {
        table_pa(core::ptr::addr_of!(ROOT))
    }

    fn map_user_page(va: u64, pa: u64, rights: Rights) -> Result<(), KError> {
        if va < USER_CODE_VA as u64 || va >= USER_CODE_VA as u64 + 0x20_0000 {
            return Err(KError::InvalidArg);
        }
        if pa % 4096 != 0 {
            return Err(KError::Alignment);
        }
        if rights.contains(Rights::WRITE) && rights.contains(Rights::EXEC) {
            return Err(KError::NoRights);
        }
        let mut flags = PTE_V | PTE_U;
        if rights.contains(Rights::READ) || rights.contains(Rights::WRITE) {
            flags |= PTE_R;
        }
        if rights.contains(Rights::WRITE) {
            flags |= PTE_W;
        }
        if rights.contains(Rights::EXEC) {
            flags |= PTE_X;
        }
        unsafe {
            let l0 = core::ptr::addr_of_mut!(USER_L0) as *mut Table;
            let index = ((va >> 12) & 0x1ff) as usize;
            (*l0).0[index] = pte(pa, flags);
            core::arch::asm!("sfence.vma {}, zero", in(reg) va, options(nostack));
        }
        Ok(())
    }

    fn unmap_user_page(va: u64) -> Result<(), KError> {
        if va < USER_CODE_VA as u64 || va >= USER_CODE_VA as u64 + 0x20_0000 {
            return Err(KError::InvalidArg);
        }
        unsafe {
            let l0 = core::ptr::addr_of_mut!(USER_L0) as *mut Table;
            let index = ((va >> 12) & 0x1ff) as usize;
            (*l0).0[index] = 0;
            core::arch::asm!("sfence.vma {}, zero", in(reg) va, options(nostack));
        }
        Ok(())
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if page_in_range(ptr, out.len(), USER_CODE_VA, 4096)
            || page_in_range(ptr, out.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, out.len(), USER_DATA_VA, 4096)
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
        if page_in_range(ptr, input.len(), USER_STACK_BASE, USER_STACK_LEN)
            || page_in_range(ptr, input.len(), USER_DATA_VA, 4096)
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

    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        let satp = (8u64 << 60) | (table_pa(core::ptr::addr_of!(ROOT)) >> 12);
        unsafe {
            core::arch::asm!("csrw satp, {}", in(reg) satp);
            core::arch::asm!("sfence.vma");
            core::arch::asm!("csrw sstatus, {}", in(reg) context.sstatus);
            core::arch::asm!("csrw sepc, {}", in(reg) context.sepc);
            core::arch::asm!("csrw stvec, {}", in(reg) trap_entry as *const () as usize);
            let trap_top = core::ptr::addr_of!(TRAP_STACK) as u64 + 32768;
            core::arch::asm!("csrw sscratch, {}", in(reg) trap_top);
            core::arch::asm!("csrs sie, {}", in(reg) 1u64 << 5);
            core::arch::asm!("mv sp, {}", in(reg) context.regs[2]);
            core::arch::asm!("sret");
        }
        unreachable!()
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
            vifast_kernel::klog!(Riscv64, "SMP OK (harts=2)");
        } else {
            vifast_kernel::klog!(Riscv64, "SMP SKIP (hsm_start={:#x})", started);
        }
    }
}

#[no_mangle]
extern "C" fn riscv_kernel_trap() -> ! {
    let cause: u64;
    unsafe {
        core::arch::asm!("csrr {}, scause", out(reg) cause);
    }
    vifast_kernel::klog!(Riscv64, "KERNEL TRAP cause={:#x}", cause);
    Riscv64::exit(1)
}

extern "C" {
    fn trap_entry();
    fn secondary_entry();
}

#[no_mangle]
extern "C" fn riscv_trap_handler(frame: *mut TrapFrame) {
    let frame = unsafe { &mut *frame };
    if frame.scause == 0x8000_0000_0000_0005 {
        vifast_runtime::timer::on_timer::<RiscvPlatform>(frame);
        return;
    }
    if frame.scause != 8 {
        let stval: u64;
        unsafe {
            core::arch::asm!("csrr {}, stval", out(reg) stval);
        }
        vifast_runtime::fault::suspend::<RiscvPlatform>(frame, frame.scause, stval);
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
}
