//! x86_64 UEFI platform: GDT/TSS/IDT, ring3, legacy PIC/PIT and user paging.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use vifast_abi::{KError, Rights};
use vifast_arch::bkl::BigKernelLock;
use vifast_arch::{Arch, CpuState, MAX_HARTS};
use vifast_kernel::{Kernel, PageTableOps};
use vifast_runtime::{Platform, PlatformInfo};

use vifast_arch_x86_64::X86_64;

pub struct X86Platform;

const USER_DATA_SEL: u64 = 0x18;
const USER_CODE_SEL: u64 = 0x20;
const TSS_SEL: u16 = 0x28;

/// r233 (T6, audit92): GDT cố định 7 entry: 5 selector + TSS (2 slot 8 byte).
/// Trampoline hiện chỉ dựng cho `AP_HART`, nên mọi hart khác sẽ cần GDT riêng.
/// Các assert dưới đây là ràng buộc TĨNH thật (không đọc lại giá trị vừa ghi
/// như 3 assert cũ trong `ap_entry` — chúng luôn đúng nên vô nghĩa, D11).
const GDT_ENTRIES: usize = 7;
const _: () = assert!(GDT_ENTRIES == 5 + 2);
const _: () = assert!(GDT_ENTRIES * 8 <= 0x1_0000);
const _: () = assert!(TSS_SEL as usize + 2 * 8 <= GDT_ENTRIES * 8);

/// Hart mà trampoline AP khởi động. x86_64 hiện chỉ có **một** AP: trampoline,
/// GDT và stack được dựng cố định cho hart 1 (audit #4 P1-7). Mọi chỗ dùng
/// index này thay vì literal `1` để giới hạn nằm ở một nơi và có assert.
const AP_HART: usize = 1;
const _: () = assert!(AP_HART < MAX_HARTS);
const _: () = assert!(AP_HART > 0);
const USER_DATA_RPL3: u64 = USER_DATA_SEL | 3;
const USER_CODE_RPL3: u64 = USER_CODE_SEL | 3;

const USER_CODE_VA: usize = vifast_abi::user_layout::CODE_VA;
const USER_DATA_VA: usize = vifast_abi::user_layout::DATA_VA;
const PAGE_PRESENT: u64 = 1;
const PAGE_WRITE: u64 = 2;
const PAGE_USER: u64 = 4;
const PAGE_LARGE: u64 = 0x80;
const PAGE_NX: u64 = 1 << 63;
const IA32_EFER: u32 = 0xC000_0080;
const IA32_GS_BASE: u32 = 0xC000_0101;
const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;
const EFER_NXE: u64 = 1 << 11;

core::arch::global_asm!(include_str!("trap.S"));

#[derive(Clone, Copy)]
#[allow(dead_code)]
#[repr(align(4096))]
struct Stack([u8; 32768]);

#[repr(align(4096))]
struct Table([u64; 512]);

static mut PML4: Table = Table([0; 512]);
static mut PDPT: Table = Table([0; 512]);
static mut PD_IDENTITY: Table = Table([0; 512]);
static mut PD_HIGH: Table = Table([0; 512]);
const LAPIC_DEFAULT: u64 = 0xFEE0_0000;
const PAGE_PCD: u64 = 1 << 4;
const PAGE_PWT: u64 = 1 << 3;

#[allow(dead_code)]
#[repr(align(4096))]
struct SplitTables([[u64; 512]; 8]);
static mut SPLIT_TABLES: SplitTables = SplitTables([[0; 512]; 8]);
static mut SPLIT_USED: usize = 0;
static mut NX_ENABLED: bool = false;
#[allow(dead_code)]
#[repr(align(4096))]
struct DataPool([u8; 393216]);
static mut USER_FRAME: DataPool = DataPool([0; 393216]);
#[allow(dead_code)]
#[repr(align(16))]
struct Metadata([u8; 81920]);
static mut METADATA: Metadata = Metadata([0; 81920]);
// Per-object user contexts indexed by `ObjRef::index`.  The kernel object
// table has MAX_OBJECTS slots, so the array must cover that full domain.
const _: () = assert!(vifast_kernel::MAX_TCBS <= u8::MAX as usize);
static mut CONTEXTS: [TrapFrame; vifast_kernel::MAX_TCBS] =
    [TrapFrame::empty(); vifast_kernel::MAX_TCBS];
static mut TRAP_STACKS: [Stack; MAX_HARTS] = [Stack([0; 32768]); MAX_HARTS];
static mut IST_STACKS: [Stack; MAX_HARTS] = [Stack([0; 32768]); MAX_HARTS];
static mut CPU_STATES: [CpuState; MAX_HARTS] = [CpuState::empty(); MAX_HARTS];
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
static mut SMAP_ENABLED: bool = false;
static mut PCID_ENABLED: bool = false;
static BKL: BigKernelLock = BigKernelLock::new();

fn bkl_lock() -> vifast_arch::bkl::BigKernelGuard {
    BKL.lock_with_hart(X86Platform::hart_id(), poll_maintenance)
}

fn assert_bkl_released() {
    let hart = X86Platform::hart_id();
    assert!(
        !BKL.held_by(hart),
        "BKL held on current hart before user return"
    );
}
static mut ACPI_CPUS: usize = 0;
static mut ACPI_BSP: u32 = 0;
static mut ACPI_LAPIC_BASE: u64 = 0;
static mut AP_TRAMPOLINE: u64 = 0;
static mut AP_APIC_ID: u32 = 0;
static mut AP_STARTED: bool = false;
static mut AP_LONG_ONLINE: bool = false;
static AP_ONLINE: AtomicBool = AtomicBool::new(false);
/// P0-2 (r289): hộp thư maintenance dùng chung (xem `vifast-maint`).
/// 289.3: MỘT hộp thư cho MỖI hart — hart phụ phải báo được hart chính, nên
/// không thể chỉ có hộp thư của AP như bản cũ.
static MAINT: [vifast_maint::Mailbox; MAX_HARTS] = [
    vifast_maint::Mailbox::new(),
    vifast_maint::Mailbox::new(),
    vifast_maint::Mailbox::new(),
    vifast_maint::Mailbox::new(),
];
/// APIC id của từng hart (0 = chưa biết). BSP tự đọc LAPIC của mình lúc init.
static APIC_IDS: [AtomicU32; MAX_HARTS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
/// 289.3: số yêu cầu shootdown đã gửi theo CẶP (nguồn, đích) — để log chẩn
/// đoán chứng minh chiều AP→BSP có thật, không chỉ BSP→AP.
static SHOOTDOWN_SENT: [[AtomicUsize; MAX_HARTS]; MAX_HARTS] = [
    [
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ],
    [
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ],
    [
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ],
    [
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    ],
];
static SHOOTDOWN_AP_TO_BSP: AtomicUsize = AtomicUsize::new(0);
static SHOOTDOWN_BSP_TO_AP: AtomicUsize = AtomicUsize::new(0);
const APIC_MAINT_VECTOR: u32 = 0x30;
const APIC_TIMER_VECTOR: u32 = 0x31;
const APIC_HALT_VECTOR: u32 = 0x32;
static AP_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static AP_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static AP_USER_TICKS: AtomicUsize = AtomicUsize::new(0);
static AP_PREEMPTS: AtomicUsize = AtomicUsize::new(0);
static AP_IPI_WAKES: AtomicUsize = AtomicUsize::new(0);
static AP_IPI_SEEN: AtomicUsize = AtomicUsize::new(0);
static AP_IPI_SENT: AtomicUsize = AtomicUsize::new(0);
static AP_IDLE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static AP_IDLE_WINDOW: AtomicBool = AtomicBool::new(false);
static USER_ENTRIES: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];
static RESCHED_SENT: AtomicUsize = AtomicUsize::new(0);
static RESCHED_WAKE: AtomicUsize = AtomicUsize::new(0);
static SECONDARY_WAIT_BLOCKS: AtomicUsize = AtomicUsize::new(0);
static POST_STARTED: AtomicBool = AtomicBool::new(false);
static POST_TIMER_TICKS: AtomicUsize = AtomicUsize::new(0);
static POST_TIMER_WAKES: AtomicUsize = AtomicUsize::new(0);
static POST_IDLE_ENTRIES: AtomicUsize = AtomicUsize::new(0);
static WAKE_SENT_THIS_WINDOW: AtomicBool = AtomicBool::new(false);
static AP_IDLE_GENERATION: AtomicUsize = AtomicUsize::new(0);
static WAKE_LAST_GENERATION: AtomicUsize = AtomicUsize::new(usize::MAX);
static AP_RESCHED_PENDING: AtomicBool = AtomicBool::new(false);
static TSC_HZ: AtomicU64 = AtomicU64::new(0);
static LAPIC_TIMER_HZ: AtomicU64 = AtomicU64::new(0);
static BSP_PIT_TICKS: AtomicUsize = AtomicUsize::new(0);
static BSP_RATE_BASE: AtomicUsize = AtomicUsize::new(0);
const AP_MAINT_TLB: u32 = 1;
const AP_MAINT_ICACHE: u32 = 2;
const AP_MAINT_TLB_VA: u32 = 4;
static TLB_VA_FLUSH: AtomicUsize = AtomicUsize::new(0);
static TLB_IPI_SENT: AtomicUsize = AtomicUsize::new(0);
static TLB_IPI_SKIPPED: AtomicUsize = AtomicUsize::new(0);
static AP_MAINT_TLB_ADDR: AtomicU64 = AtomicU64::new(0);
static AP_MAINT_TLB_PAGES: AtomicUsize = AtomicUsize::new(0);
static AP_MAINT_TLB_ASID: AtomicU32 = AtomicU32::new(0);
const TRAMPOLINE_STAGE_OFFSET: u64 = 0x802;
/// P1-7 (r261): ô 4 byte trong trang trampoline ghi hart mà AP sẽ trở thành.
/// Trước đây `ap_entry` dùng hằng `AP_HART`, nên cùng một trampoline không thể
/// khởi động hart nào khác; nay danh tính hart đi qua dữ liệu, không qua mã.
const TRAMPOLINE_HART_OFFSET: u64 = 0x204;

unsafe fn identity_u32(address: u64) -> Option<u32> {
    if address.saturating_add(4) > 0x4000_0000 {
        return None;
    }
    Some(core::ptr::read_unaligned(address as *const u32))
}

unsafe fn identity_u64(address: u64) -> Option<u64> {
    if address.saturating_add(8) > 0x4000_0000 {
        return None;
    }
    Some(core::ptr::read_unaligned(address as *const u64))
}

unsafe fn identity_table(address: u64, max: usize) -> Option<&'static [u8]> {
    if address < 0x1000 || address.saturating_add(8) > 0x4000_0000 {
        return None;
    }
    let length = identity_u32(address + 4)? as usize;
    if length < 36 || length > max || address.saturating_add(length as u64) > 0x4000_0000 {
        return None;
    }
    Some(core::slice::from_raw_parts(address as *const u8, length))
}

unsafe fn init_acpi(rsdp: u64) {
    if rsdp == 0 {
        X86_64::write_str("ACPI SKIP (no RSDP)\r\n");
        return;
    }
    if rsdp >= 0x4000_0000 {
        X86_64::write_str("ACPI SKIP (above identity map)\r\n");
        return;
    }
    let Some(revision) = (|| {
        let length = identity_u32(rsdp + 20)? as usize;
        if length < 36 || length > 4096 || rsdp.saturating_add(length as u64) > 0x4000_0000 {
            return None;
        }
        Some(core::slice::from_raw_parts(rsdp as *const u8, length)[15])
    })() else {
        X86_64::write_str("ACPI SKIP (bad RSDP length)\r\n");
        return;
    };
    if revision < 2 {
        X86_64::write_str("ACPI SKIP (RSDP v1)\r\n");
        return;
    }
    let Some(xsdt_address) = identity_u64(rsdp + 24) else {
        X86_64::write_str("ACPI SKIP (above identity map)\r\n");
        return;
    };
    let Some(rsdp_len) = identity_u32(rsdp + 20) else {
        return;
    };
    let rsdp_table = core::slice::from_raw_parts(rsdp as *const u8, rsdp_len as usize);
    let Some(xsdt) = identity_table(xsdt_address, 4096) else {
        X86_64::write_str("ACPI SKIP (bad XSDT)\r\n");
        return;
    };
    let mut madt_address = 0;
    let mut offset = 36;
    while offset + 8 <= xsdt.len() {
        let address = core::ptr::read_unaligned(xsdt.as_ptr().add(offset) as *const u64);
        if identity_u32(address).map(|value| value == u32::from_le_bytes(*b"APIC")) == Some(true) {
            madt_address = address;
            break;
        }
        offset += 8;
    }
    if madt_address == 0 {
        X86_64::write_str("ACPI SKIP (no MADT)\r\n");
        return;
    }
    let Some(madt) = identity_table(madt_address, 8192) else {
        X86_64::write_str("ACPI SKIP (bad MADT)\r\n");
        return;
    };
    match vifast_acpi::parse(rsdp_table, xsdt, madt, madt_address) {
        Ok(parsed) => {
            ACPI_CPUS = parsed.cpu_count;
            ACPI_BSP = parsed.bsp;
            ACPI_LAPIC_BASE = parsed.lapic_base;
            AP_APIC_ID = 0;
            // 289.3: lưu APIC id theo hart để shootdown đi được CẢ HAI chiều.
            APIC_IDS[0].store(parsed.bsp, Ordering::Release);
            for cpu in parsed.cpus[..parsed.cpu_count].iter() {
                if cpu.enabled && cpu.apic_id != parsed.bsp {
                    AP_APIC_ID = cpu.apic_id;
                    APIC_IDS[1].store(cpu.apic_id, Ordering::Release);
                    break;
                }
            }
            vifast_kernel::klog!(
                X86Platform,
                "ACPI MADT OK (cpus={}, bsp={}, lapic={:#x})",
                parsed.cpu_count,
                parsed.bsp,
                parsed.lapic_base
            );
        }
        Err(error) => {
            vifast_kernel::klog!(X86Platform, "ACPI SKIP ({:?})", error);
        }
    }
}

/// T33a (r294): đèn báo "hart này đang quay BKL quá lâu". Chỉ in MỘT lần cho
/// mỗi hart, KHÔNG phải marker của gate.
static BKL_SPIN_TICKS: [AtomicUsize; MAX_HARTS] = [const { AtomicUsize::new(0) }; MAX_HARTS];
static BKL_STUCK_REPORTED: [AtomicBool; MAX_HARTS] =
    [const { AtomicBool::new(false) }; MAX_HARTS];

fn note_bkl_spin() {
    #[cfg(not(feature = "diag-stall"))]
    {
        return;
    }
    #[cfg(feature = "diag-stall")]
    {
    let me = X86Platform::hart_id();
    if me >= MAX_HARTS {
        return;
    }
    let spins = BKL_SPIN_TICKS[me].fetch_add(1, Ordering::Relaxed);
    // ~2 triệu vòng là hàng trăm ms ở QEMU: gần như chắc chắn là kẹt thật.
    if spins < 2_000_000 || BKL_STUCK_REPORTED[me].swap(true, Ordering::Relaxed) {
        return;
    }
    let owner = BKL.owner_hart();
    vifast_kernel::klog!(
        X86Platform,
        "BKL STUCK (hart={}, owner={:?}, held={}, holders={}, spins={})",
        me,
        owner,
        BKL.is_held(),
        BKL.holders(),
        spins
    );
    }
}

fn poll_maintenance() {
    note_bkl_spin();
    unsafe {
        if read_msr(IA32_GS_BASE) != core::ptr::addr_of!(CPU_STATES[1]) as u64 {
            return;
        }
        process_maintenance();
    }
}

fn assert_irqs_disabled() {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq; pop {}", out(reg) flags);
    }
    assert!(flags & (1 << 9) == 0, "BKL entered with IF set");
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct DescriptorTablePtr {
    limit: u16,
    base: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    zero: u32,
}

impl IdtEntry {
    const fn zero() -> Self {
        IdtEntry {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            zero: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Tss {
    reserved0: u32,
    rsp0: u64,
    rsp1: u64,
    rsp2: u64,
    reserved1: u64,
    ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    iomap: u16,
}

impl Tss {
    const fn zero() -> Self {
        Tss {
            reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iomap: 0,
        }
    }
}

/// P1-7 (r261): GDT riêng cho TỪNG hart.
///
/// Trước đây BSP dùng `GDT` còn AP dùng `AP_GDT`; cả hai đều gắn cứng vào một
/// hart duy nhất, nên trampoline chỉ khởi động được đúng một AP và mọi hart
/// khác không có GDT/TSS hợp lệ. Nay mọi hart đi chung một đường
/// `setup_hart_gdt_tss_idt(hart)` và mỗi hart có GDT + GDTR + TSS riêng.
static mut GDTS: [[u64; GDT_ENTRIES]; MAX_HARTS] = [[0; GDT_ENTRIES]; MAX_HARTS];
static mut GDT_PTRS: [DescriptorTablePtr; MAX_HARTS] =
    [DescriptorTablePtr { limit: 0, base: 0 }; MAX_HARTS];
static mut IDT: [IdtEntry; 256] = [IdtEntry::zero(); 256];
/// Vùng ảnh kernel mà firmware đã nạp: (base, base + SizeOfImage) theo
/// `EFI_LOADED_IMAGE_PROTOCOL` (vòng 96, audit #8). Ảnh là DYNAMIC_BASE nên
/// không được hard-code 0x1400_0000.
static mut IMAGE_RANGE: (u64, u64) = (0, 0);

/// Ghi lại vùng ảnh ngay sau khi UEFI xác thực; dùng cho `kernel_image_end()`
/// và để kiểm tra W^X sau này.
pub fn set_image_range(base: u64, size: u64) {
    let end = base.saturating_add(size);
    unsafe {
        IMAGE_RANGE = (base, end);
    }
}

/// (base, end) của ảnh kernel trong bộ nhớ.
pub fn image_range() -> (u64, u64) {
    unsafe { IMAGE_RANGE }
}

static mut IDT_PTR: DescriptorTablePtr = DescriptorTablePtr { limit: 0, base: 0 };
static mut TSSS: [Tss; MAX_HARTS] = [Tss::zero(); MAX_HARTS];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rbp: u64,
    rdi: u64,
    rsi: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

// Vòng 92: chốt layout với trap.S để lệch một word là fail lúc build chứ
// không phải lúc chạy (audit #6 ca ngợi const assert kiểu này).
const _: () = assert!(core::mem::size_of::<TrapFrame>() == 160);
const _: () = assert!(core::mem::offset_of!(TrapFrame, r15) == 0);
const _: () = assert!(core::mem::offset_of!(TrapFrame, rax) == 112);
const _: () = assert!(core::mem::offset_of!(TrapFrame, rip) == 120);
const _: () = assert!(core::mem::offset_of!(TrapFrame, cs) == 128);
const _: () = assert!(core::mem::offset_of!(TrapFrame, ss) == 152);

impl TrapFrame {
    const fn empty() -> Self {
        TrapFrame {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rbp: 0,
            rdi: 0,
            rsi: 0,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            rip: 0,
            cs: 0,
            rflags: 0,
            rsp: 0,
            ss: 0,
        }
    }

    /// Dựng `TrapFrame` đầy đủ từ stack thô sau `SAVE_REGS` (vòng 92, audit
    /// #6): `exc_common` đẩy thêm vector+error trước rip nên rip ở word 17;
    /// `pf_entry` chỉ có error nên rip ở word 16. Phần 15 thanh ghi trùng
    /// layout với struct nên copy thẳng.
    unsafe fn from_raw(frame: *const u64, has_vector: bool) -> Self {
        let mut out = *(frame as *const TrapFrame);
        let tail = if has_vector { 17 } else { 16 };
        out.rip = *frame.add(tail);
        out.cs = *frame.add(tail + 1);
        out.rflags = *frame.add(tail + 2);
        out.rsp = *frame.add(tail + 3);
        out.ss = *frame.add(tail + 4);
        out
    }

    fn user(entry: usize, stack: usize) -> Self {
        let mut frame = Self::empty();
        frame.rip = entry as u64;
        frame.cs = USER_CODE_RPL3;
        frame.rflags = 0x202;
        frame.rsp = stack as u64;
        frame.ss = USER_DATA_RPL3;
        frame
    }
}

unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value);
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value);
    value
}

unsafe fn inw(port: u16) -> u16 {
    let value: u16;
    core::arch::asm!("in ax, dx", in("dx") port, out("ax") value);
    value
}

unsafe fn outw(port: u16, value: u16) {
    core::arch::asm!("out dx, ax", in("dx") port, in("ax") value);
}

unsafe fn outl(port: u16, value: u32) {
    core::arch::asm!("out dx, eax", in("dx") port, in("eax") value);
}

unsafe fn inl(port: u16) -> u32 {
    let value: u32;
    core::arch::asm!("in eax, dx", in("dx") port, out("eax") value);
    value
}

/// Đọc 32-bit từ PCI config space qua cơ chế 0xCF8/0xCFC (vòng 84).
unsafe fn pci_read32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = 0x8000_0000u32
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xfc);
    outl(0xcf8, address);
    inl(0xcfc)
}

/// Quét bus 0 để chứng minh truy cập config space hoạt động và đếm thiết bị.
/// Đây là nền tảng cho virtio-pci sau này (x86_64 hiện chưa có virtio).
fn pci_scan() -> (u32, u32, u32, u32) {
    let mut count = 0u32;
    let mut first_vendor = 0u32;
    let mut first_device = 0u32;
    let mut first_class = 0u32;
    for slot in 0..32u8 {
        let id = unsafe { pci_read32(0, slot, 0, 0) };
        let vendor = id & 0xffff;
        if vendor == 0xffff || vendor == 0 {
            continue;
        }
        let device = id >> 16;
        let class = unsafe { pci_read32(0, slot, 0, 8) } >> 16;
        if count == 0 {
            first_vendor = vendor;
            first_device = device;
            first_class = class;
        }
        count += 1;
    }
    (count, first_vendor, first_device, first_class)
}

const ATA_BASE: u16 = 0x1F0;
const ATA_CTRL: u16 = 0x3F6;

unsafe fn ata_wait_bsy() -> bool {
    let mut spins = 0u32;
    while inb(ATA_BASE + 7) & 0x80 != 0 {
        spins += 1;
        if spins > 10_000_000 {
            return false;
        }
        core::hint::spin_loop();
    }
    true
}

unsafe fn ata_wait_drq() -> bool {
    let mut spins = 0u32;
    loop {
        let status = inb(ATA_BASE + 7);
        if status & 0x80 == 0 && status & 0x08 != 0 {
            return true;
        }
        if status & 0x01 != 0 || spins > 10_000_000 {
            return false;
        }
        spins += 1;
        core::hint::spin_loop();
    }
}

unsafe fn ata_prepare(lba: u64) -> bool {
    if lba >= 1 << 28 || !ata_wait_bsy() {
        return false;
    }
    outb(ATA_BASE + 6, 0xE0 | ((lba >> 24) as u8 & 0x0f));
    // 400 ns delay: four alternate-status reads.
    for _ in 0..4 {
        let _ = inb(ATA_CTRL);
    }
    outb(ATA_BASE + 2, 1);
    outb(ATA_BASE + 3, lba as u8);
    outb(ATA_BASE + 4, (lba >> 8) as u8);
    outb(ATA_BASE + 5, (lba >> 16) as u8);
    true
}

unsafe fn ata_read(lba: u64, out: &mut [u8; 512]) -> bool {
    if !ata_prepare(lba) {
        return false;
    }
    outb(ATA_BASE + 7, 0x20);
    if !ata_wait_drq() {
        return false;
    }
    for i in 0..256 {
        let word = inw(ATA_BASE);
        out[i * 2] = word as u8;
        out[i * 2 + 1] = (word >> 8) as u8;
    }
    true
}

unsafe fn ata_write(lba: u64, input: &[u8; 512]) -> bool {
    if !ata_prepare(lba) {
        return false;
    }
    outb(ATA_BASE + 7, 0x30);
    if !ata_wait_drq() {
        return false;
    }
    for i in 0..256 {
        let word = input[i * 2] as u16 | ((input[i * 2 + 1] as u16) << 8);
        outw(ATA_BASE, word);
    }
    ata_flush()
}

unsafe fn ata_flush() -> bool {
    if !ata_wait_bsy() {
        return false;
    }
    outb(ATA_BASE + 7, 0xE7); // FLUSH CACHE
    ata_wait_bsy()
}

fn table_pa(table: *const Table) -> u64 {
    table as u64
}

unsafe fn zero_table(table: *mut Table) {
    for entry in (*table).0.iter_mut() {
        *entry = 0;
    }
}

unsafe fn enable_smep_smap() {
    let leaf7 = core::arch::x86_64::__cpuid(7);
    let mut cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4);
    if leaf7.ebx & (1 << 7) != 0 {
        cr4 |= 1 << 20; // SMEP
    }
    if leaf7.ebx & (1 << 20) != 0 {
        cr4 |= 1 << 21; // SMAP
        SMAP_ENABLED = true;
    }
    core::arch::asm!("mov cr4, {}", in(reg) cr4);
}

unsafe fn enable_pcid() {
    let leaf1 = core::arch::x86_64::__cpuid(1);
    if leaf1.ecx & (1 << 17) == 0 {
        return;
    }
    let mut cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4);
    cr4 |= 1 << 17;
    core::arch::asm!("mov cr4, {}", in(reg) cr4);
    PCID_ENABLED = true;
}

/// Tắt SSE/SSE2 cho cả ring 0 lẫn ring 3 (vòng 87, audit #3 P1-2).
/// Kernel và toàn bộ ELF user hiện không có lệnh SSE (kiểm bằng llvm-objdump),
/// nên thay vì phải fxsave/fxrstor 512 byte mỗi lần switch, ta chặn hẳn: một
/// ELF tương lai dùng SSE sẽ #UD rõ ràng thay vì đọc xmm còn sót của task khác.
unsafe fn disable_sse() {
    let mut cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4);
    cr4 &= !((1u64 << 9) | (1u64 << 10)); // OSFXSR | OSXMMEXCPT
    core::arch::asm!("mov cr4, {}", in(reg) cr4);
}

/// CPU có hỗ trợ NX không (đọc CPUID, không phụ thuộc cờ runtime).
fn cpu_has_nx() -> bool {
    let ext = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if ext < 0x8000_0001 {
        return false;
    }
    core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 20) != 0
}

unsafe fn enable_nx() -> bool {
    let ext = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if ext < 0x8000_0001 || core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 20) == 0 {
        return false;
    }
    let low: u32;
    let high: u32;
    core::arch::asm!("rdmsr", in("ecx") IA32_EFER, out("eax") low, out("edx") high);
    let efer = (((high as u64) << 32) | low as u64) | EFER_NXE;
    core::arch::asm!(
        "wrmsr",
        in("ecx") IA32_EFER,
        in("eax") efer as u32,
        in("edx") (efer >> 32) as u32,
    );
    true
}

/// Split one 2 MiB identity leaf into 4 KiB leaves so individual kernel data
/// pages can be marked non-executable without affecting the text page.
unsafe fn split_identity_2m(region: usize) -> *mut u64 {
    let identity = core::ptr::addr_of_mut!(PD_IDENTITY) as *mut Table;
    // Vòng 97 (audit #9): PD_IDENTITY chỉ có 512 entry (0..1 GiB). Base ưu
    // tiên của ảnh EFI là 0x1400_0000 (region 10240) nên thiếu check này là
    // ghi ngoài mảng 4 KiB, không panic.
    if region >= (*identity).0.len() {
        return core::ptr::null_mut();
    }
    let entry = &mut (*identity).0[region];
    if *entry & PAGE_LARGE == 0 {
        if *entry & PAGE_PRESENT == 0 {
            return core::ptr::null_mut();
        }
        return (*entry & !0xfff) as *mut u64;
    }
    let slot = SPLIT_USED;
    if slot >= 8 {
        return core::ptr::null_mut();
    }
    SPLIT_USED += 1;
    let table = core::ptr::addr_of_mut!(SPLIT_TABLES) as *mut u64;
    let table = table.add(slot * 512);
    let base = region as u64 * 0x20_0000;
    for index in 0..512 {
        *table.add(index) = (base + index as u64 * 4096) | PAGE_PRESENT | PAGE_WRITE;
    }
    *entry = table as u64 | PAGE_PRESENT | PAGE_WRITE;
    table
}

unsafe fn mark_nx_range(start: u64, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let end = match start.checked_add(len as u64 - 1) {
        Some(end) => end,
        None => return false,
    };
    let mut va = start & !0xfff;
    loop {
        let region = (va >> 21) as usize;
        let index = ((va >> 12) & 0x1ff) as usize;
        let table = split_identity_2m(region);
        if table.is_null() {
            return false;
        }
        *table.add(index) |= PAGE_NX;
        if va >= (end & !0xfff) {
            break;
        }
        va += 4096;
    }
    true
}

/// Quét section table của ảnh PE đang nạp và trả các dải **executable** đã
/// căn trang (tối đa 4). Trả 0 khi chưa biết vùng ảnh hoặc header không hợp lệ
/// — khi đó chính sách NX quay về danh sách tĩnh.
///
/// Vì sao đọc PE thay vì linker script: ảnh UEFI do lld-link (COFF) tạo và
/// `rust-lld -flavor link -T` báo lỗi (audit #8), nên chỉ có thể lấy dải text
/// từ chính header đã nạp.
fn kernel_text_ranges(out: &mut [(u64, u64); 8]) -> usize {
    let (base, end) = image_range();
    if base == 0 || end <= base.saturating_add(0x40) {
        return 0;
    }
    let limit = end - base;
    let peek16 =
        |off: u64| -> u16 { unsafe { core::ptr::read_unaligned((base + off) as *const u16) } };
    let peek32 =
        |off: u64| -> u32 { unsafe { core::ptr::read_unaligned((base + off) as *const u32) } };
    if peek16(0) != 0x5a4d {
        return 0; // "MZ"
    }
    let pe = u64::from(peek32(0x3c));
    if pe.saturating_add(0x18) > limit || peek32(pe) != 0x0000_4550 {
        return 0; // "PE\0\0"
    }
    let sections = u64::from(peek16(pe + 6));
    let optional = u64::from(peek16(pe + 0x14));
    let table = pe.saturating_add(0x18).saturating_add(optional);
    let mut count = 0;
    let mut index = 0;
    while index < sections.min(16) && count < out.len() {
        let entry = table.saturating_add(index * 40);
        if entry.saturating_add(40) > limit {
            break;
        }
        let characteristics = peek32(entry + 0x24);
        if characteristics & 0x2000_0000 != 0 {
            // IMAGE_SCN_MEM_EXECUTE; VirtualSize dùng được cho NOLOAD.
            let size = u64::from(peek32(entry + 0x8));
            let va = u64::from(peek32(entry + 0xc));
            if size != 0 {
                if count == out.len() {
                    // > số dải cho phép: fail-closed thay vì bỏ qua section exec
                    // (bỏ qua sẽ NX chính efi_main ⇒ triple fault trước khi có IDT).
                    return 0;
                }
                let start = base.saturating_add(va);
                let stop = start.saturating_add(size).min(end);
                if stop > start {
                    out[count] = (start, stop);
                    count += 1;
                } else {
                    return 0;
                }
            }
        }
        index += 1;
    }
    // Vòng 98 (audit #9): nếu ảnh được nạp thấp tới mức chứa cả trang
    // trampoline AP (0x9F000) thì trang đó PHẢI giữ executable — AP chạy
    // 16-bit real mode từ chính trang này.
    let tramp = unsafe { AP_TRAMPOLINE };
    if tramp != 0 && tramp >= base && tramp < end {
        if count == out.len() {
            return 0;
        }
        out[count] = (tramp, tramp.saturating_add(4096).min(end));
        count += 1;
    }
    count
}

/// Đánh NX cho **mọi trang của ảnh** trừ các dải executable (vòng 97).
/// Trước đây chỉ 20 vùng tĩnh được NX nên phần lớn ảnh (boot stack 192 KiB và
/// hàng chục static nhỏ) vẫn executable.
fn image_pages_nx() -> bool {
    let (base, end) = image_range();
    if base == 0 || end <= base {
        return false;
    }
    let mut text = [(0u64, 0u64); 8];
    let count = kernel_text_ranges(&mut text);
    if count == 0 {
        return false;
    }
    let mut va = base & !0xfff;
    while va < end {
        let page_end = va.saturating_add(4096);
        let is_text = text[..count]
            .iter()
            .any(|(start, stop)| va < *stop && page_end > *start);
        if !is_text && !unsafe { mark_nx_range(va, 4096) } {
            return false;
        }
        va = page_end;
    }
    // Vòng 97 (audit #9): x86 không tự đồng bộ TLB với PTE write; nếu không
    // flush, mapping 2 MiB cũ (executable) vẫn nằm trong TLB và NX chỉ là
    // "trên giấy" cho tới lần CR3 reload kế tiếp — đúng kiểu xanh giả.
    unsafe { reload_cr3() };
    true
}

fn kernel_data_ranges() -> [(u64, usize); 20] {
    [
        (
            core::ptr::addr_of!(TRAP_STACKS) as u64,
            core::mem::size_of::<[Stack; MAX_HARTS]>(),
        ),
        (
            core::ptr::addr_of!(IST_STACKS) as u64,
            core::mem::size_of::<[Stack; MAX_HARTS]>(),
        ),
        (
            core::ptr::addr_of!(CONTEXTS) as u64,
            core::mem::size_of::<[TrapFrame; vifast_kernel::MAX_TCBS]>(),
        ),
        (
            core::ptr::addr_of!(GDTS) as u64,
            core::mem::size_of::<[u64; 7]>(),
        ),
        (
            core::ptr::addr_of!(TSSS) as u64,
            core::mem::size_of::<[Tss; MAX_HARTS]>(),
        ),
        (
            core::ptr::addr_of!(IDT) as u64,
            core::mem::size_of::<[IdtEntry; 256]>(),
        ),
        (
            core::ptr::addr_of!(BOOT_POOL) as u64,
            core::mem::size_of::<BootPool>(),
        ),
        (
            core::ptr::addr_of!(USER_FRAME) as u64,
            core::mem::size_of::<DataPool>(),
        ),
        (
            core::ptr::addr_of!(METADATA) as u64,
            core::mem::size_of::<Metadata>(),
        ),
        (
            core::ptr::addr_of!(KERNEL_STORAGE) as u64,
            core::mem::size_of::<Kernel>(),
        ),
        (
            core::ptr::addr_of!(PML4) as u64,
            core::mem::size_of::<Table>(),
        ),
        (
            core::ptr::addr_of!(PDPT) as u64,
            core::mem::size_of::<Table>(),
        ),
        (
            core::ptr::addr_of!(PD_IDENTITY) as u64,
            core::mem::size_of::<Table>(),
        ),
        (
            core::ptr::addr_of!(SPLIT_TABLES) as u64,
            core::mem::size_of::<SplitTables>(),
        ),
        // Vòng 93 (audit #6): các static dưới đây từng bị bỏ khỏi danh sách
        // NX nên vẫn executable trong kernel space.
        (
            core::ptr::addr_of!(GDTS) as u64,
            core::mem::size_of::<[u64; 7]>(),
        ),
        (
            core::ptr::addr_of!(GDT_PTRS) as u64,
            core::mem::size_of::<DescriptorTablePtr>(),
        ),
        (
            core::ptr::addr_of!(IDT_PTR) as u64,
            core::mem::size_of::<DescriptorTablePtr>(),
        ),
        (
            core::ptr::addr_of!(GDT_PTRS) as u64,
            core::mem::size_of::<DescriptorTablePtr>(),
        ),
        (
            core::ptr::addr_of!(CPU_STATES) as u64,
            core::mem::size_of::<[CpuState; MAX_HARTS]>(),
        ),
        // Vòng 94 (audit #7): PD_HIGH là page table của LAPIC (2 MiB leaf);
        // thiếu nó thì trang chứa LAPIC vẫn executable dưới NX.
        (
            core::ptr::addr_of!(PD_HIGH) as u64,
            core::mem::size_of::<Table>(),
        ),
    ]
}

unsafe fn build_kernel_page_tables() {
    let pml4 = core::ptr::addr_of_mut!(PML4) as *mut Table;
    let pdpt = core::ptr::addr_of_mut!(PDPT) as *mut Table;
    let identity = core::ptr::addr_of_mut!(PD_IDENTITY) as *mut Table;
    let high = core::ptr::addr_of_mut!(PD_HIGH) as *mut Table;

    zero_table(pml4);
    zero_table(pdpt);
    zero_table(identity);
    zero_table(high);

    /* PML4[0] -> PDPT; PDPT[0] covers 0..1 GiB identity for kernel/UEFI
    code, stacks, GDT/TSS/IDT and page tables. */
    (*pml4).0[0] = table_pa(pdpt) | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
    (*pdpt).0[0] = table_pa(identity) | PAGE_PRESENT | PAGE_WRITE;
    // LAPIC lives in 3..4 GiB.  Share this supervisor-only 2 MiB MMIO
    // mapping through the same PDPT copied into every VSpace.
    let lapic_base = if ACPI_LAPIC_BASE == 0 {
        LAPIC_DEFAULT
    } else {
        ACPI_LAPIC_BASE
    };
    (*pdpt).0[3] = table_pa(high) | PAGE_PRESENT | PAGE_WRITE;
    let high_index = ((lapic_base >> 21) & 0x1ff) as usize;
    (*high).0[high_index] = (lapic_base & !0x1f_ffff)
        | PAGE_PRESENT
        | PAGE_WRITE
        | PAGE_LARGE
        | PAGE_PCD
        | PAGE_PWT
        | PAGE_NX;
    for i in 0..512usize {
        (*identity).0[i] = (i as u64 * 0x20_0000) | PAGE_PRESENT | PAGE_WRITE | PAGE_LARGE;
    }
    NX_ENABLED = enable_nx();
    if NX_ENABLED {
        for (start, len) in kernel_data_ranges() {
            // Vòng 94 (audit #7): trước đây hết slot split thì NX_ENABLED bị
            // đặt false và `kernel_data_nx` trả true (vì !NX_ENABLED) — tức
            // toàn bộ data kernel thành executable mà self-check vẫn xanh.
            // Nay thiếu slot là lỗi chết người, không im lặng.
            if !mark_nx_range(start, len) {
                vifast_kernel::klog!(
                    X86Platform,
                    "NX FAIL: không đủ slot split cho static tại {:#x} (len={})",
                    start,
                    len
                );
                X86_64::exit(1);
            }
        }
    }
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

unsafe fn write_msr(msr: u32, value: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") value as u32,
        in("edx") (value >> 32) as u32,
    );
}

unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!("rdmsr", in("ecx") msr, out("eax") low, out("edx") high);
    ((high as u64) << 32) | low as u64
}

fn cpu_state() -> *mut CpuState {
    unsafe { read_msr(IA32_GS_BASE) as *mut CpuState }
}

fn publish_current_tcb(tcb: Option<usize>) {
    let state = cpu_state();
    unsafe {
        if !state.is_null() {
            (*state).current_tcb = tcb.unwrap_or(usize::MAX);
        }
    }
}

fn active_root() -> u64 {
    let state = cpu_state();
    unsafe {
        if state.is_null() {
            0
        } else {
            (*state).active_root
        }
    }
}

fn active_asid() -> u16 {
    let state = cpu_state();
    unsafe {
        if state.is_null() {
            0
        } else {
            (*state).active_asid
        }
    }
}

fn set_active_root(root: u64, asid: u16) {
    let state = cpu_state();
    unsafe {
        if !state.is_null() {
            (*state).active_root = root;
            (*state).active_asid = asid;
        }
    }
}

fn assert_current_tcb(where_: &str) {
    let state = cpu_state();
    if state.is_null() {
        return;
    }
    let hart = unsafe { (*state).hart_id };
    let expected = X86Platform::kernel()
        .current_on(hart)
        .map(|tcb| tcb.index as usize)
        .unwrap_or(usize::MAX);
    let actual = unsafe { (*state).current_tcb };
    if actual != expected {
        vifast_kernel::klog!(
            X86Platform,
            "CURRENT MISMATCH where={} hart={} actual={} expected={}",
            where_,
            unsafe { (*state).hart_id },
            actual,
            expected
        );
        X86Platform::exit(1);
    }
}

static mut LAPIC_READY: bool = false;

unsafe fn configure_lapic() -> u32 {
    let base = if ACPI_LAPIC_BASE == 0 {
        LAPIC_DEFAULT
    } else {
        ACPI_LAPIC_BASE
    };
    let svr = (base + 0xF0) as *mut u32;
    let tpr = (base + 0x80) as *mut u32;
    let id = core::ptr::read_volatile((base + 0x20) as *const u32) >> 24;
    core::ptr::write_volatile(tpr, 0);
    core::ptr::write_volatile(svr, 0x100 | 0xFF);
    id
}

unsafe fn init_lapic() {
    if LAPIC_READY {
        return;
    }
    let id = configure_lapic();
    let base = lapic_base();
    core::ptr::write_volatile((base + 0x350) as *mut u32, 0x700);
    let lint0 = core::ptr::read_volatile((base + 0x350) as *const u32);
    let lint1 = core::ptr::read_volatile((base + 0x360) as *const u32);
    LAPIC_READY = true;
    vifast_kernel::klog!(X86Platform, "LAPIC OK (id={})", id);
    vifast_kernel::klog!(X86Platform, "BSP LINT0={:#x} LINT1={:#x}", lint0, lint1);
}

unsafe fn lapic_base() -> u64 {
    if ACPI_LAPIC_BASE == 0 {
        LAPIC_DEFAULT
    } else {
        ACPI_LAPIC_BASE
    }
}

fn count_user_entry() {
    let hart = X86Platform::hart_id();
    if let Some(counter) = USER_ENTRIES.get(hart) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe fn lapic_eoi() {
    core::ptr::write_volatile((lapic_base() + 0xB0) as *mut u32, 0);
}

unsafe fn reload_cr3() {
    let cr3: u64;
    core::arch::asm!("mov {}, cr3", out(reg) cr3);
    core::arch::asm!("mov cr3, {}", in(reg) cr3);
}

unsafe fn flush_tagged_va(asid: u16, va: u64, pages: usize) {
    if active_asid() == asid {
        for page in 0..pages {
            core::arch::asm!("invlpg [{}]", in(reg) va + (page as u64) * 4096);
        }
    } else {
        // The CPU may keep tagged entries for a non-active PCID.  A CR3
        // reload is the portable fallback when INVPCID is unavailable.
        reload_cr3();
    }
}

unsafe fn process_maintenance() {
    let me = X86Platform::hart_id();
    let Some((seq, kind, scope)) = MAINT[me].take() else {
        return;
    };
    if kind & (AP_MAINT_TLB | AP_MAINT_TLB_VA) != 0 {
        match scope {
            vifast_maint::Scope::All => reload_cr3(),
            vifast_maint::Scope::Asid(_) => reload_cr3(),
            vifast_maint::Scope::Range { va, pages, asid } => {
                flush_tagged_va(asid, va, pages);
            }
            vifast_maint::Scope::Free => {}
        }
    }
    if kind & AP_MAINT_ICACHE != 0 {
        core::arch::asm!("wbinvd");
    }
    MAINT[me].ack(seq);
}

/// 289.3: gửi maintenance tới MỘT hart bất kỳ (kể cả hart chính khi ta là hart
/// phụ). Trả `seq` để chờ ACK, 0 nếu không gửi được.
unsafe fn send_maintenance_to(target: usize, kind: u32, scope: vifast_maint::Scope) -> usize {
    let me = X86Platform::hart_id();
    if target >= MAX_HARTS || target == me {
        return 0;
    }
    if target != 0 && !unsafe { AP_STARTED } {
        return 0;
    }
    let apic = APIC_IDS[target].load(Ordering::Acquire);
    if apic == 0 {
        return 0;
    }
    let base = lapic_base();
    let seq = MAINT[target].publish(kind, scope);
    core::sync::atomic::fence(Ordering::SeqCst);
    AP_IPI_SENT.fetch_add(1, Ordering::AcqRel);
    SHOOTDOWN_SENT[me][target].fetch_add(1, Ordering::Relaxed);
    if me == 1 && target == 0 {
        SHOOTDOWN_AP_TO_BSP.fetch_add(1, Ordering::Relaxed);
    } else if me == 0 && target == 1 {
        SHOOTDOWN_BSP_TO_AP.fetch_add(1, Ordering::Relaxed);
    }
    core::ptr::write_volatile((base + 0x310) as *mut u32, apic << 24);
    send_icr(base, APIC_MAINT_VECTOR);
    seq
}

unsafe fn wait_maintenance(target: usize, seq: usize) -> bool {
    if seq == 0 || target >= MAX_HARTS {
        return false;
    }
    for _ in 0..5_000_000 {
        if MAINT[target].acked(seq) {
            return true;
        }
        core::hint::spin_loop();
    }
    MAINT[target].acked(seq)
}

#[cfg_attr(feature = "test-skip-tlb-ipi", allow(dead_code))]
/// Publish an AP TLB-VA shootdown and return its sequence number without
/// waiting (issue #4 — same publish/wait split as RISC-V/AArch64).  On x86
/// there is a single AP, so the split mainly documents the contract and
/// keeps the wait-polling (`pause` loop) reusable; returns 0 when no AP.
unsafe fn publish_tlb_va_ap(asid: u16, va: u64, pages: usize) -> usize {
    unsafe {
        send_maintenance_to(
            1,
            AP_MAINT_TLB_VA,
            vifast_maint::Scope::Range { va, pages, asid },
        )
    }
}

/// Wait until the AP acknowledges `seq`.  Safe without the BKL: publication
/// is ordered by Release stores + LAPIC ICR.
unsafe fn wait_ap(seq: usize) -> bool {
    if seq == 0 {
        return false;
    }
    for _ in 0..100_000 {
        if MAINT[1].acked(seq) {
            return true;
        }
        core::arch::asm!("pause");
    }
    false
}

unsafe fn send_tlb_va_ap(asid: u16, va: u64, pages: usize) -> bool {
    let seq = unsafe { publish_tlb_va_ap(asid, va, pages) };
    if seq == 0 {
        return false;
    }
    unsafe { wait_ap(seq) }
}

/// Publish a full-AP TLB flush without waiting (issue #4 — pairs with
/// [`wait_ap`]).
unsafe fn publish_full_tlb_ap() -> usize {
    unsafe { send_maintenance_to(1, AP_MAINT_TLB, vifast_maint::Scope::All) }
}

unsafe fn send_full_tlb_ap() -> bool {
    let seq = unsafe { publish_full_tlb_ap() };
    if seq == 0 {
        return false;
    }
    unsafe { wait_ap(seq) }
}

unsafe fn rdtsc() -> u64 {
    let low: u32;
    let high: u32;
    core::arch::asm!("rdtsc", out("eax") low, out("edx") high);
    ((high as u64) << 32) | low as u64
}

unsafe fn calibrate_tsc_pit() {
    // PIT channel 2, mode 0, 10 ms one-shot; gate enabled through port 0x61.
    let divisor: u16 = 11932;
    let control = (inb(0x61) & !0x02) | 0x01;
    outb(0x61, control);
    outb(0x43, 0xB0);
    outb(0x42, divisor as u8);
    outb(0x42, (divisor >> 8) as u8);
    let start = rdtsc();
    let mut spins = 0u64;
    while inb(0x61) & 0x20 == 0 && spins < 100_000_000 {
        core::arch::asm!("pause");
        spins += 1;
    }
    let end = rdtsc();
    let elapsed = end.wrapping_sub(start);
    let hz = if spins >= 100_000_000 || elapsed == 0 {
        2_500_000_000
    } else {
        ((elapsed as u128 * 1_193_182u128) / divisor as u128) as u64
    };
    TSC_HZ.store(hz, Ordering::Release);
    vifast_kernel::klog!(X86Platform, "TSC HZ={}", hz);
}

unsafe fn tsc_delay_us(micros: u64) {
    let hz = TSC_HZ.load(Ordering::Acquire);
    let cycles = if hz == 0 {
        micros.saturating_mul(2_500)
    } else {
        (hz as u128 * micros as u128 / 1_000_000) as u64
    };
    tsc_delay(cycles);
}

unsafe fn calibrate_lapic_timer() {
    let base = lapic_base();
    let tsc = TSC_HZ.load(Ordering::Acquire).max(1);
    // Mask the timer and run a one-shot with the largest count for 10 ms.
    core::ptr::write_volatile((base + 0x320) as *mut u32, 1 << 16);
    core::ptr::write_volatile((base + 0x3E0) as *mut u32, 0xB);
    core::ptr::write_volatile((base + 0x380) as *mut u32, 0xFFFF_FFFF);
    let start = rdtsc();
    let target = tsc / 100;
    while rdtsc().wrapping_sub(start) < target {
        core::arch::asm!("pause");
    }
    let elapsed = rdtsc().wrapping_sub(start).max(1);
    let current = core::ptr::read_volatile((base + 0x390) as *const u32);
    let delta = 0xFFFF_FFFFu64.saturating_sub(current as u64);
    let hz = if delta == 0 {
        100_000_000
    } else {
        ((delta as u128 * tsc as u128) / elapsed as u128) as u64
    };
    let hz = hz.max(1_000);
    LAPIC_TIMER_HZ.store(hz, Ordering::Release);
    vifast_kernel::klog!(X86Platform, "LAPIC TIMER HZ={}", hz);
    let initial = (hz / 100).max(1_000) as u32;
    core::ptr::write_volatile((base + 0x3E0) as *mut u32, 0xB);
    core::ptr::write_volatile((base + 0x380) as *mut u32, 0);
    core::ptr::write_volatile((base + 0x320) as *mut u32, (1 << 17) | APIC_TIMER_VECTOR);
    core::ptr::write_volatile((base + 0x380) as *mut u32, initial);
}

unsafe fn ap_idle_loop() -> ! {
    calibrate_lapic_timer();
    loop {
        let (picked, ready) = {
            let bkl = bkl_lock();
            let picked = X86Platform::kernel().pick_on(1);
            let ready = X86Platform::kernel().has_ready_on(1);
            drop(bkl);
            (picked, ready)
        };
        if let Some(tcb) = picked {
            publish_current_tcb(Some(tcb.index as usize));
            if vifast_runtime::ensure_current_root::<X86Platform>().is_err() {
                X86Platform::exit(1);
            }
            let slot = X86Platform::kernel().context_slot(tcb).unwrap_or(0) as usize;
            let context = X86Platform::contexts()[slot];
            <X86Platform as Platform>::enter_user(&context);
        }
        if ready {
            continue;
        }
        AP_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        let generation = AP_IDLE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = generation;
        WAKE_SENT_THIS_WINDOW.store(false, Ordering::SeqCst);
        if POST_STARTED.load(Ordering::Relaxed) {
            POST_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
        }
        // Vòng 89 (audit #4 P1-5): đọc queue (1094) và set cờ (1120) không
        // nguyên tử — task được make_ready ở giữa sẽ không có IPI. Kiểm lại
        // sau khi set cờ; nếu đã có việc thì không ngủ.
        AP_IDLE_WINDOW.store(true, Ordering::SeqCst);
        let raced = {
            let bkl = bkl_lock();
            let work = X86Platform::kernel().has_ready_on(1);
            drop(bkl);
            work
        };
        if raced {
            AP_IDLE_WINDOW.store(false, Ordering::SeqCst);
            continue;
        }
        core::arch::asm!("sti", "hlt", "cli", options(nomem, nostack));
        AP_IDLE_WINDOW.store(false, Ordering::SeqCst);
    }
}

unsafe fn install_real_trampoline() {
    let base = AP_TRAMPOLINE;
    if base == 0 || base >= 0xA0000 || base & 0xfff != 0 {
        return;
    }
    let page = base as *mut u8;
    core::ptr::write_bytes(page, 0, 4096);
    // 16-bit real mode: magic, GDT, protected mode, then far jump to 32-bit.
    let vector_byte = (base >> 12) as u8;
    let real = [
        0xFA, // cli
        0xB8,
        0x00,
        vector_byte, // mov ax, vector << 8
        0x8E,
        0xD8, // mov ds, ax
        0xC7,
        0x06,
        0x00,
        0x08,
        0x5A,
        0xA5, // mov word [0800], A55A
        0x0F,
        0x01,
        0x16,
        0x40,
        0x03, // lgdt [0340]
        0x0F,
        0x20,
        0xC0, // mov eax, cr0
        0x0C,
        0x01, // or al, 1
        0x0F,
        0x22,
        0xC0, // mov cr0, eax
        0x66,
        0xEA,
        0,
        0,
        0,
        0,
        0x08,
        0x00, // ljmp 08:page+0x30
    ];
    let far_start = real.len() - 8;
    core::ptr::copy_nonoverlapping(real.as_ptr(), page, far_start);
    let real_stage = [0xC6, 0x06, 0x02, 0x08, 0x01]; // mov byte [0802], 1
    core::ptr::copy_nonoverlapping(real_stage.as_ptr(), page.add(far_start), real_stage.len());
    core::ptr::copy_nonoverlapping(
        real[far_start..].as_ptr(),
        page.add(far_start + real_stage.len()),
        8,
    );
    core::ptr::write_unaligned(
        page.add(far_start + real_stage.len() + 2) as *mut u32,
        (base as u32).wrapping_add(0x30),
    );
    // 32-bit protected mode: flat segments, CR3, PAE, EFER.LME|NXE, paging.
    let pm = [
        0x66, 0xB8, 0x10, 0x00, // mov ax, 0x10
        0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xE0, 0x8E, 0xE8, 0x8E, 0xD0, 0x8B, 0x1D, 0, 0, 0,
        0, // mov ebx, [page+0x200]
        0x0F, 0x22, 0xDB, // mov cr3, ebx
        0x0F, 0x20, 0xE0, // mov eax, cr4
        0x0D, 0x20, 0x00, 0x00, 0x00, // or eax, 0x20
        0x0F, 0x22, 0xE0, // mov cr4, eax
        0xB9, 0x80, 0x00, 0x00, 0xC0, // mov ecx, 0xC0000080
        0x0F, 0x32, // rdmsr
        0x0D, 0x00, 0x09, 0x00, 0x00, // or eax, 0x900
        0x0F, 0x30, // wrmsr
        0x0F, 0x20, 0xC0, // mov eax, cr0
        0x0D, 0x01, 0x00, 0x00, 0x80, // or eax, 0x80000001
        0x0F, 0x22, 0xC0, // mov cr0, eax
        0xEA, 0, 0, 0, 0, 0x18, 0x00, // ljmp 18:page+0x80
    ];
    let pm_offset = 0x30;
    let pm_far = pm.len() - 7;
    core::ptr::copy_nonoverlapping(pm.as_ptr(), page.add(pm_offset), pm_far);
    let mut pm_stage = [
        0xC6, 0x05, 0, 0, 0, 0, 0x02, // mov byte [abs], 2
    ];
    core::ptr::write_unaligned(
        pm_stage.as_mut_ptr().add(2) as *mut u32,
        (base as u32).wrapping_add(0x802),
    );
    core::ptr::copy_nonoverlapping(
        pm_stage.as_ptr(),
        page.add(pm_offset + pm_far),
        pm_stage.len(),
    );
    core::ptr::copy_nonoverlapping(
        pm[pm_far..].as_ptr(),
        page.add(pm_offset + pm_far + pm_stage.len()),
        7,
    );
    core::ptr::write_unaligned(
        page.add(pm_offset + 0x10) as *mut u32,
        (base as u32).wrapping_add(0x200),
    );
    core::ptr::write_unaligned(
        page.add(pm_offset + pm_far + pm_stage.len() + 1) as *mut u32,
        (base as u32).wrapping_add(0x80),
    );
    #[cfg(feature = "test-ap-stall")]
    core::ptr::copy_nonoverlapping(
        [0xEB, 0xFE].as_ptr(),
        page.add(pm_offset + pm_far + pm_stage.len()),
        2,
    );
    // 64-bit entry: load stack and AP entry address.
    let mut lm = [
        0x66, 0xB8, 0x10, 0x00, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xE0, 0x8E, 0xE8, 0x8E, 0xD0, 0x48,
        0xBC, 0, 0, 0, 0, 0, 0, 0, 0, // mov rsp, imm64
        0x48, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, // mov rax, imm64
        0xFF, 0xE0,
    ];
    let lm_offset = 0x80;
    core::ptr::copy_nonoverlapping(lm.as_mut_ptr(), page.add(lm_offset), lm.len());
    let stack = core::ptr::addr_of!(TRAP_STACKS[AP_HART]) as u64 + 0x4000;
    let entry = ap_entry as *const () as u64;
    core::ptr::write_unaligned(page.add(lm_offset + 16) as *mut u64, stack);
    core::ptr::write_unaligned(page.add(lm_offset + 26) as *mut u64, entry);
    // Data: CR3 at 0x200, GDT at 0x300/0x340, stack/entry patched in code.
    let pml4 = core::ptr::addr_of!(PML4) as u32;
    core::ptr::write_unaligned(page.add(0x200) as *mut u32, pml4);
    core::ptr::write_unaligned(page.add(0x204) as *mut u32, AP_HART as u32);
    let gdt = [
        0u64,
        0x00CF_9A00_0000_FFFFu64,
        0x00CF_9200_0000_FFFFu64,
        0x00AF_9A00_0000_FFFFu64,
        0x00AF_9200_0000_FFFFu64,
    ];
    core::ptr::copy_nonoverlapping(
        gdt.as_ptr() as *const u8,
        page.add(0x300),
        core::mem::size_of_val(&gdt),
    );
    let gdt_desc = [0x27u8, 0, 0, 0, 0, 0, 0, 0];
    core::ptr::copy_nonoverlapping(gdt_desc.as_ptr(), page.add(0x340), 8);
    core::ptr::write_unaligned(
        page.add(0x342) as *mut u32,
        (base as u32).wrapping_add(0x300),
    );
}

/// Hart mà trampoline đang dựng (đọc từ dữ liệu, không phải hằng số).
unsafe fn hart_from_trampoline() -> usize {
    let hart = core::ptr::read_volatile((AP_TRAMPOLINE + TRAMPOLINE_HART_OFFSET) as *const u32);
    core::cmp::min(hart as usize, MAX_HARTS - 1)
}

#[allow(asm_sub_register)]
#[no_mangle]
pub extern "C" fn ap_entry() -> ! {
    unsafe {
        core::arch::asm!("cli");
        let stage_ptr = (AP_TRAMPOLINE + TRAMPOLINE_STAGE_OFFSET) as *mut u8;
        core::ptr::write_volatile(stage_ptr, 3);
        let hart = hart_from_trampoline();
        setup_hart_gdt_tss_idt(hart);
        init_cpu(hart);
        // r233 (T6, audit92): ba assert cũ đều VACUOUS — chúng đọc lại đúng giá
        // trị vừa ghi (`hart_id()` tra index cố định, `str` trả hằng 0x28,
        // GS_BASE so với chính giá trị vừa set). Chúng không thể fail ⇒ false-green.
        // Thay bằng hai kiểm tra có thể fail thật:
        //  (a) APIC ID đọc từ phần cứng phải đúng hart mà trampoline nhắm tới;
        //  (b) GS_BASE phải nằm TRONG mảng CPU_STATES (kiểm biên, không so giá trị).
        let hw_apic_id =
            (core::ptr::read_volatile((lapic_base() + 0x20) as *const u32) >> 24) & 0xff;
        assert_eq!(
            hw_apic_id, AP_APIC_ID,
            "AP APIC id đọc từ LAPIC không khớp hart trampoline nhắm tới"
        );
        let gs = read_msr(IA32_GS_BASE);
        let states_lo = core::ptr::addr_of!(CPU_STATES) as u64;
        let states_hi = states_lo + core::mem::size_of::<[CpuState; MAX_HARTS]>() as u64;
        assert!(
            gs >= states_lo && gs < states_hi,
            "AP GS base nằm ngoài CPU_STATES"
        );
        let _ = configure_lapic();
        let base = lapic_base();
        core::ptr::write_volatile((base + 0x350) as *mut u32, 1 << 16);
        core::ptr::write_volatile((base + 0x360) as *mut u32, 1 << 16);
        enable_smep_smap();

        disable_sse();

        vifast_kernel::klog!(X86Platform, "AP SSE OFF");
        core::ptr::write_volatile(stage_ptr, 4);
        AP_ONLINE.store(true, Ordering::SeqCst);
        AP_LONG_ONLINE = true;
        ap_idle_loop()
    }
}

pub fn panic_halt_others() {
    unsafe {
        let base = lapic_base();
        let target = if X86Platform::hart_id() == 0 {
            if !AP_ONLINE.load(Ordering::SeqCst) {
                return;
            }
            AP_APIC_ID
        } else {
            0
        };
        core::ptr::write_volatile((base + 0x310) as *mut u32, target << 24);
        send_icr(base, APIC_HALT_VECTOR);
    }
}

unsafe fn trampoline_stage() -> u8 {
    if AP_TRAMPOLINE == 0 {
        0
    } else {
        core::ptr::read_volatile((AP_TRAMPOLINE + TRAMPOLINE_STAGE_OFFSET) as *const u8)
    }
}

unsafe fn wait_stage(target: u8) -> u8 {
    let mut stage = trampoline_stage();
    for _ in 0..40 {
        if stage >= target {
            return stage;
        }
        tsc_delay_us(500);
        stage = trampoline_stage();
    }
    stage
}

unsafe fn wait_icr_idle(base: u64) -> bool {
    for _ in 0..1_000_000 {
        if core::ptr::read_volatile((base + 0x300) as *const u32) & (1 << 12) == 0 {
            return true;
        }
        core::arch::asm!("pause");
    }
    false
}

unsafe fn send_icr(base: u64, low: u32) {
    // Destination was written to ICR high (0x310) before this low write.
    core::ptr::write_volatile((base + 0x300) as *mut u32, low);
}

unsafe fn tsc_delay(cycles: u64) {
    let low: u32;
    let high: u32;
    core::arch::asm!("rdtsc", out("eax") low, out("edx") high);
    let start = ((high as u64) << 32) | low as u64;
    let mut now = start;
    while now.wrapping_sub(start) < cycles {
        core::arch::asm!("pause");
        let low: u32;
        let high: u32;
        core::arch::asm!("rdtsc", out("eax") low, out("edx") high);
        now = ((high as u64) << 32) | low as u64;
    }
}

unsafe fn start_ap_real() {
    if AP_STARTED || AP_TRAMPOLINE == 0 || AP_APIC_ID == 0 {
        return;
    }
    AP_STARTED = true;
    let base = if ACPI_LAPIC_BASE == 0 {
        LAPIC_DEFAULT
    } else {
        ACPI_LAPIC_BASE
    };
    let vector = (AP_TRAMPOLINE >> 12) as u32;
    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
    send_icr(base, 0x000C_4500);
    if !wait_icr_idle(base) {
        X86_64::write_str("SMP FAIL (ap timeout stage=real)\r\n");
        return;
    }
    tsc_delay_us(10_000);
    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
    send_icr(base, 0x000C_4600 | vector);
    if !wait_icr_idle(base) {
        X86_64::write_str("SMP FAIL (ap timeout stage=real)\r\n");
        return;
    }
    tsc_delay_us(200);
    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
    send_icr(base, 0x000C_4600 | vector);
    let _ = wait_icr_idle(base);
    let stage = wait_stage(1);
    if stage < 1 {
        vifast_kernel::klog!(X86Platform, "SMP FAIL (ap timeout stage={})", stage);
        #[cfg(feature = "test-ap-stall")]
        X86Platform::exit(1);
        #[cfg(not(feature = "test-ap-stall"))]
        return;
    }
    let apic = AP_APIC_ID;
    vifast_kernel::klog!(X86Platform, "AP REAL MODE OK (apic={})", apic);
    let stage = wait_stage(3);
    if stage < 3 {
        vifast_kernel::klog!(X86Platform, "SMP FAIL (ap timeout stage={})", stage);
        #[cfg(feature = "test-ap-stall")]
        X86Platform::exit(1);
        #[cfg(not(feature = "test-ap-stall"))]
        return;
    }
    vifast_kernel::klog!(X86Platform, "AP LONG MODE OK (cpu=1)");
    let seq = MAINT[1].publish(AP_MAINT_TLB | AP_MAINT_ICACHE, vifast_maint::Scope::All);
    core::sync::atomic::fence(Ordering::SeqCst);
    AP_IPI_SENT.fetch_add(1, Ordering::AcqRel);
    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
    send_icr(base, APIC_MAINT_VECTOR);
    for _ in 0..100_000 {
        if MAINT[1].acked(seq) && AP_IPI_SEEN.load(Ordering::Acquire) > 0 {
            BSP_RATE_BASE.store(BSP_PIT_TICKS.load(Ordering::Acquire), Ordering::Release);
            vifast_kernel::klog!(X86Platform, "SMP OK (harts=2, ipi=1)");
            return;
        }
        core::arch::asm!("pause");
    }
    vifast_kernel::klog!(
        X86Platform,
        "SMP FAIL (ap timeout stage={})",
        trampoline_stage()
    );
}

unsafe fn init_cpu(hartid: usize) {
    let index = core::cmp::min(hartid, MAX_HARTS - 1);
    CPU_STATES[index].hart_id = hartid;
    CPU_STATES[index].current_tcb = 0;
    CPU_STATES[index].trap_stack_top = core::ptr::addr_of!(TRAP_STACKS[index]) as u64 + 16384;
    write_msr(IA32_GS_BASE, core::ptr::addr_of!(CPU_STATES[index]) as u64);
    write_msr(IA32_KERNEL_GS_BASE, 0);
}
unsafe fn setup_hart_gdt_tss_idt(hart: usize) {
    let hart = core::cmp::min(hart, MAX_HARTS - 1);
    let tss = core::ptr::addr_of_mut!(TSSS[hart]);
    (*tss).rsp0 = core::ptr::addr_of!(TRAP_STACKS[hart]) as u64 + 16384;
    (*tss).ist[0] = core::ptr::addr_of!(IST_STACKS[hart]) as u64 + 32768;
    (*tss).iomap = core::mem::size_of::<Tss>() as u16;
    let tss_base = tss as u64;
    let tss_limit = core::mem::size_of::<Tss>() as u64 - 1;
    let tss_low = (tss_limit & 0xffff)
        | ((tss_base & 0xffffff) << 16)
        | (0x89u64 << 40)
        | (((tss_limit >> 16) & 0xf) << 48)
        | (((tss_base >> 24) & 0xff) << 56);
    let tss_high = tss_base >> 32;
    let gdt = core::ptr::addr_of_mut!(GDTS[hart]);
    (*gdt)[0] = 0;
    (*gdt)[1] = 0x00AF9A000000FFFF;
    (*gdt)[2] = 0x00AF92000000FFFF;
    (*gdt)[3] = 0x00AFF2000000FFFF;
    (*gdt)[4] = 0x00AFFA000000FFFF;
    (*gdt)[5] = tss_low;
    (*gdt)[6] = tss_high;
    let gdt_base = core::ptr::addr_of!(GDTS[hart]) as u64;
    core::ptr::write_volatile(
        core::ptr::addr_of_mut!(GDT_PTRS[hart]),
        DescriptorTablePtr {
            limit: GDT_ENTRIES as u16 * 8 - 1,
            base: gdt_base,
        },
    );
    core::arch::asm!("lgdt [{}]", in(reg) core::ptr::addr_of!(GDT_PTRS[hart]));
    x86_reload_segments();
    core::arch::asm!("ltr ax", in("ax") TSS_SEL);
    let idt = core::ptr::addr_of_mut!(IDT);
    for entry in (*idt).iter_mut() {
        *entry = IdtEntry::zero();
    }
    let handler = int80_entry as *const () as u64;
    (*idt)[0x80] = IdtEntry {
        offset_low: handler as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0xEE,
        offset_mid: (handler >> 16) as u16,
        offset_high: (handler >> 32) as u32,
        zero: 0,
    };
    let spurious = spurious_entry as *const () as u64;
    (*idt)[0xFF] = IdtEntry {
        offset_low: spurious as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0xEE,
        offset_mid: (spurious >> 16) as u16,
        offset_high: (spurious >> 32) as u32,
        zero: 0,
    };
    for (vector, handler) in x86_exception_table.iter().enumerate() {
        if vector == 0x0E {
            continue;
        }
        let handler = *handler;
        (*idt)[vector] = IdtEntry {
            offset_low: handler as u16,
            selector: 0x08,
            ist: if matches!(vector, 2 | 8 | 18) { 1 } else { 0 },
            type_attr: 0x8E,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        };
    }
    let pf = pf_entry as *const () as u64;
    (*idt)[0x0E] = IdtEntry {
        offset_low: pf as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0x8E,
        offset_mid: (pf >> 16) as u16,
        offset_high: (pf >> 32) as u32,
        zero: 0,
    };
    let timer = irq0_entry as *const () as u64;
    (*idt)[0x20] = IdtEntry {
        offset_low: timer as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0x8E,
        offset_mid: (timer >> 16) as u16,
        offset_high: (timer >> 32) as u32,
        zero: 0,
    };
    // IRQ1: bàn phím PS/2 (vòng 98, x86 parity với virtio-input).
    let keyboard = irq1_entry as *const () as u64;
    (*idt)[0x21] = IdtEntry {
        offset_low: keyboard as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0x8E,
        offset_mid: (keyboard >> 16) as u16,
        offset_high: (keyboard >> 32) as u32,
        zero: 0,
    };
    let uart = irq4_entry as *const () as u64;
    (*idt)[0x24] = IdtEntry {
        offset_low: uart as u16,
        selector: 0x08,
        ist: 0,
        type_attr: 0x8E,
        offset_mid: (uart >> 16) as u16,
        offset_high: (uart >> 32) as u32,
        zero: 0,
    };
    for (vector, handler) in [
        (APIC_MAINT_VECTOR, ap_ipi_entry as *const () as u64),
        (APIC_TIMER_VECTOR, ap_timer_entry as *const () as u64),
        (APIC_HALT_VECTOR, ap_halt_entry as *const () as u64),
    ] {
        (*idt)[vector as usize] = IdtEntry {
            offset_low: handler as u16,
            selector: 0x08,
            ist: 0,
            type_attr: 0x8E,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            zero: 0,
        };
    }
    core::ptr::write_volatile(
        core::ptr::addr_of_mut!(IDT_PTR),
        DescriptorTablePtr {
            limit: 256 * 16 - 1,
            base: idt as u64,
        },
    );
    core::arch::asm!("lidt [{}]", in(reg) core::ptr::addr_of!(IDT_PTR));
}

unsafe fn setup_pic_pit() {
    outb(0x20, 0x11);
    outb(0xA0, 0x11);
    outb(0x21, 0x20);
    outb(0xA1, 0x28);
    outb(0x21, 0x04);
    outb(0xA1, 0x02);
    outb(0x21, 0x01);
    outb(0xA1, 0x01);
    outb(0x21, 0xEC);
    outb(0xA1, 0xFF);
    outb(0x3F9, 0x01); // COM1 RX interrupt
                       // i8042 (vòng 98, gia cố theo audit #10): OVMF để controller ở trạng thái
                       // tuỳ ý. Thứ tự an toàn:
                       //   AD (disable port 1) -> xả OBF -> đọc CCB (đã xả) -> sửa CCB
                       //   -> ghi CCB -> AE -> xả -> F4 và chờ ACK -> cuối cùng mới unmask IRQ1.
    unsafe fn wait_write() {
        let mut spins = 0u32;
        while spins < 100_000 && inb(0x64) & 0x02 != 0 {
            core::hint::spin_loop();
            spins += 1;
        }
    }
    unsafe fn wait_read() -> bool {
        let mut spins = 0u32;
        while spins < 100_000 && inb(0x64) & 0x01 == 0 {
            core::hint::spin_loop();
            spins += 1;
        }
        inb(0x64) & 0x01 != 0
    }
    unsafe fn flush_out() {
        let mut drain = 0;
        while drain < 32 && inb(0x64) & 0x01 != 0 {
            let _ = inb(0x60);
            drain += 1;
        }
    }
    wait_write();
    outb(0x64, 0xAD); // disable first port
    flush_out();
    wait_write();
    outb(0x64, 0x20); // read CCB
    let mut config = 0u8;
    if wait_read() {
        config = inb(0x60);
    }
    // bit0 IRQ1 enable; bit6 translation (set 2 -> set 1); xoá bit1/bit3/bit4
    // (clock inhibit) để keyboard không bị "điếc" mà không báo lỗi.
    config = (config & !0x02 & !0x08 & !0x10) | 0x01 | 0x40;
    wait_write();
    outb(0x64, 0x60);
    wait_write();
    outb(0x60, config);
    wait_write();
    outb(0x64, 0xAE); // enable first port
    flush_out();
    wait_write();
    outb(0x60, 0xF4); // enable scanning
    if wait_read() {
        let _ = inb(0x60); // ACK 0xFA
    }
    flush_out();
    // Unmask IRQ1 SAU cùng: trước đó IF=0 nên chưa có ngắt nào tới.
    outb(0x21, 0xEC);
    let divisor: u16 = 11932;
    outb(0x43, 0x36);
    outb(0x40, divisor as u8);
    outb(0x40, (divisor >> 8) as u8);
}

impl Arch for X86Platform {
    const NAME: &'static str = X86_64::NAME;
    fn init() {
        X86_64::init();
    }
    fn putc(byte: u8) {
        X86_64::putc(byte);
    }
    fn getc() -> Option<u8> {
        X86_64::getc()
    }
    fn stash_rx(byte: u8) {
        X86_64::stash_rx(byte);
    }
    fn now() -> u64 {
        X86_64::now()
    }
    fn irq_disable() {
        X86_64::irq_disable();
    }
    fn irq_enable() {
        X86_64::irq_enable();
    }
    fn send_ipi(hart: usize) {
        // Vòng 88: LAPIC ICR tới AP (hart 1). Hart 0 là BSP, không tự gửi.
        if hart != 1 || !AP_ONLINE.load(Ordering::SeqCst) {
            return;
        }
        unsafe {
            let base = lapic_base();
            core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
            send_icr(base, APIC_MAINT_VECTOR);
        }
    }
    fn halt() -> ! {
        X86_64::halt()
    }
    fn exit(code: u8) -> ! {
        X86_64::exit(code)
    }
}

impl vifast_kernel::console::Console for X86Platform {
    const NAME: &'static str = X86_64::NAME;

    fn write_byte(byte: u8) {
        X86_64::putc(byte);
    }
}

struct PageOps;

impl vifast_kernel::PageTableOps for PageOps {
    fn levels(&self) -> usize {
        vifast_pte::x86_64::LEVELS
    }

    fn index(&self, va: u64, level: usize) -> usize {
        vifast_pte::x86_64::index(va, level)
    }

    fn read_pte(&self, table_pa: u64, idx: usize) -> u64 {
        unsafe { *((table_pa as *const u64).add(idx)) }
    }

    fn write_pte(&self, table_pa: u64, idx: usize, pte: u64) {
        unsafe {
            *((table_pa as *mut u64).add(idx)) = pte;
        }
    }

    fn encode_leaf(&self, pa: u64, rights: Rights) -> u64 {
        vifast_pte::x86_64::encode_leaf(pa, rights)
    }

    fn encode_table(&self, pa: u64) -> u64 {
        vifast_pte::x86_64::encode_table(pa)
    }

    fn decode(&self, entry: u64) -> vifast_kernel::Pte {
        vifast_pte::x86_64::decode(entry)
    }

    fn decode_at(&self, entry: u64, level: usize) -> vifast_kernel::Pte {
        vifast_pte::x86_64::decode_at(entry, level)
    }

    fn vspace_pages(&self) -> usize {
        2
    }

    fn install_kernel(&self, root_pa: u64, aux_pa: u64) {
        unsafe {
            let root = root_pa as *mut u64;
            let aux = aux_pa as *mut u64;
            let shared_pdpt = core::ptr::addr_of!(PDPT) as *const u64;
            *root = aux_pa | PAGE_PRESENT | PAGE_WRITE | PAGE_USER;
            *aux = *shared_pdpt;
            // PDPT[3] carries the supervisor-only LAPIC MMIO window.
            *aux.add(3) = *shared_pdpt.add(3);
        }
    }

    fn kernel_accessible_range(&self) -> (u64, u64) {
        (0, 0x4000_0000)
    }

    fn user_va_range(&self) -> (u64, u64) {
        (
            vifast_pte::x86_64::USER_VA_START,
            vifast_pte::x86_64::USER_VA_END,
        )
    }

    fn flush_va(&self, va: u64) {
        unsafe {
            core::arch::asm!("invlpg [{}]", in(reg) va);
        }
    }

    fn flush_all_user(&self) {
        unsafe {
            reload_cr3();
        }
    }

    fn flush_va_asid(&self, asid: u16, va: u64, pages: usize, targets: u32) {
        TLB_VA_FLUSH.fetch_add(1, Ordering::Relaxed);
        unsafe {
            flush_tagged_va(asid, va, pages);
        }
        // 289.4: chỉ shootdown khi CHÍNH hart này giữ BKL.
        if !BKL.held_by(X86Platform::hart_id()) {
            return;
        }
        let me = X86Platform::hart_id();
        #[cfg(not(feature = "test-skip-tlb-ipi"))]
        for target in 0..MAX_HARTS {
            if target == me {
                continue;
            }
            // 289.3: đối xứng — hart phụ cũng phải báo được hart chính.
            // Hart 0 luôn online; hart > 0 cần AP đã lên.
            if target != 0 && !unsafe { AP_STARTED } {
                continue;
            }
            if APIC_IDS[target].load(Ordering::Acquire) == 0 {
                continue;
            }
            if targets & (1 << target) == 0 {
                TLB_IPI_SKIPPED.fetch_add(1, Ordering::Relaxed);
            } else {
                let seq = unsafe {
                    send_maintenance_to(
                        target,
                        AP_MAINT_TLB_VA,
                        vifast_maint::Scope::Range { va, pages, asid },
                    )
                };
                if seq != 0 && unsafe { wait_maintenance(target, seq) } {
                    TLB_IPI_SENT.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        #[cfg(feature = "test-skip-tlb-ipi")]
        let _ = (me, targets);
    }

    fn flush_all_asid(&self, _asid: u16, targets: u32) {
        TLB_VA_FLUSH.fetch_add(1, Ordering::Relaxed);
        unsafe {
            reload_cr3();
        }
        // 289.4: chỉ shootdown khi CHÍNH hart này giữ BKL.
        if !BKL.held_by(X86Platform::hart_id()) {
            return;
        }
        let me = X86Platform::hart_id();
        // 289.3: đối xứng như `flush_va_asid`.
        for target in 0..MAX_HARTS {
            if target == me {
                continue;
            }
            if target != 0 && !unsafe { AP_STARTED } {
                continue;
            }
            if APIC_IDS[target].load(Ordering::Acquire) == 0 {
                continue;
            }
            if targets & (1 << target) == 0 {
                TLB_IPI_SKIPPED.fetch_add(1, Ordering::Relaxed);
            } else {
                let seq =
                    unsafe { send_maintenance_to(target, AP_MAINT_TLB, vifast_maint::Scope::All) };
                if seq != 0 && unsafe { wait_maintenance(target, seq) } {
                    TLB_IPI_SENT.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    fn sync_icache(&self, _pa: u64, _size: usize) {
        // x86 has coherent instruction and data caches.
    }

    fn leaf_is_user(&self, pte: u64) -> bool {
        vifast_pte::x86_64::is_user(pte)
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
    for level in 0..vifast_pte::x86_64::LEVELS {
        let idx = vifast_pte::x86_64::index(va, level);
        let pte = PageOps.read_pte(table, idx);
        match PageOps.decode_at(pte, level) {
            vifast_kernel::Pte::Table(next) => {
                if level + 1 == vifast_pte::x86_64::LEVELS {
                    return false;
                }
                table = next;
            }
            vifast_kernel::Pte::Leaf(_, rights) => {
                return rights.contains(Rights::READ)
                    && !vifast_pte::x86_64::is_user(pte)
                    && (!need_write || pte & vifast_pte::x86_64::W != 0)
                    && (!need_exec || pte & vifast_pte::x86_64::NX == 0);
            }
            vifast_kernel::Pte::Invalid => return false,
        }
    }
    false
}

fn kernel_leaf_nx(root_pa: u64, va: u64) -> bool {
    let mut table = root_pa;
    for level in 0..vifast_pte::x86_64::LEVELS {
        let idx = vifast_pte::x86_64::index(va, level);
        let pte = PageOps.read_pte(table, idx);
        match PageOps.decode_at(pte, level) {
            vifast_kernel::Pte::Table(next) => {
                if level + 1 == vifast_pte::x86_64::LEVELS {
                    return false;
                }
                table = next;
            }
            vifast_kernel::Pte::Leaf(_, _) => return pte & PAGE_NX != 0,
            vifast_kernel::Pte::Invalid => return false,
        }
    }
    false
}

fn kernel_range_nx(root_pa: u64, start: u64, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let end = match start.checked_add(len as u64 - 1) {
        Some(end) => end,
        None => return false,
    };
    let mut va = start & !0xfff;
    loop {
        if !kernel_leaf_nx(root_pa, va) {
            return false;
        }
        if va >= (end & !0xfff) {
            break;
        }
        va += 4096;
    }
    true
}

/// W^X tự kiểm tra độc lập với danh sách tĩnh (vòng 97, audit #8): duyệt từng
/// trang của ảnh kernel trong page table và fail nếu một trang **không thuộc
/// dải text** mà leaf vẫn executable. Nhờ đọc từ chính page table, check này
/// không thể "xanh giả" khi `image_pages_nx()` bỏ sót.
fn kernel_wx_ok(root_pa: u64) -> bool {
    if unsafe { !NX_ENABLED } {
        return !cpu_has_nx();
    }
    let (base, end) = image_range();
    if base == 0 || end <= base {
        return false;
    }
    let mut text = [(0u64, 0u64); 8];
    let count = kernel_text_ranges(&mut text);
    if count == 0 {
        return false;
    }
    let mut va = base & !0xfff;
    while va < end {
        let page_end = va.saturating_add(4096);
        let is_text = text[..count]
            .iter()
            .any(|(start, stop)| va < *stop && page_end > *start);
        if !is_text {
            let mut table = root_pa;
            let mut pte = 0u64;
            let mut found = false;
            for level in 0..vifast_pte::x86_64::LEVELS {
                let idx = vifast_pte::x86_64::index(va, level);
                let entry = PageOps.read_pte(table, idx);
                match PageOps.decode_at(entry, level) {
                    vifast_kernel::Pte::Table(next) => table = next,
                    vifast_kernel::Pte::Leaf(_, _) => {
                        pte = entry;
                        found = true;
                        break;
                    }
                    vifast_kernel::Pte::Invalid => break,
                }
            }
            // Audit #9: phải kiểm cả hai chiều. Non-text còn exec = lỗi cần
            // bắt; text bị NX = sẽ #PF ngay lệnh đầu (đặc biệt efi_main).
            let exec = pte & PAGE_NX == 0;
            let supervisor = !vifast_pte::x86_64::is_user(pte);
            if !found || !supervisor {
                return false;
            }
            if is_text {
                if !exec {
                    return false;
                }
            } else if exec {
                return false;
            }
        }
        va = page_end;
    }
    true
}

fn kernel_data_nx(root_pa: u64) -> bool {
    // NX tắt chỉ hợp lệ khi CPU không hỗ trợ (enable_nx trả false). Nếu CPU có
    // NX mà cờ bị tắt thì đó là lỗi, không được coi là "pass".
    if unsafe { !NX_ENABLED } {
        return !cpu_has_nx();
    }
    kernel_data_ranges()
        .into_iter()
        .all(|(start, len)| kernel_range_nx(root_pa, start, len))
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

// r229 (T4, D11): canary cho vùng đệm ngay sau `KERNEL_STORAGE`.
// Nếu Kernel phình ra ghi đè vùng này thì phát hiện được thay vì âm thầm phá
// static kế tiếp (AArch64 từng chỉ còn 8 byte trước BKL).
#[link_section = ".data.kernel_storage_guard"]
static mut KSTORE_GUARD: [u8; 4096] = [0x5A; 4096];
const KSTORE_CANARY: u64 = 0x4B53_544F_5245_4F4B;

fn kstore_canary_write() {
    unsafe {
        let p = core::ptr::addr_of_mut!(KSTORE_GUARD) as *mut u8;
        core::ptr::write_volatile(p as *mut u64, KSTORE_CANARY);
    }
}

fn kstore_canary_ok() -> bool {
    unsafe {
        let p = core::ptr::addr_of!(KSTORE_GUARD) as *const u8;
        core::ptr::read_volatile(p as *const u64) == KSTORE_CANARY
            && core::ptr::read_volatile(p.add(4095)) == 0x5A
    }
}

impl Platform for X86Platform {
    /// Vòng 161: bật thu hồi trang staging. Lần thử trước in `SYS REJECT BAD`
    /// nhưng nguyên nhân thật là phép thử dùng slot 63 (đã dùng hợp lệ) — sửa
    /// phép thử xong thì bật lại an toàn.
    fn staging_release_safe() -> bool {
        true
    }

    type Context = TrapFrame;

    const USER_CODE_VA: usize = USER_CODE_VA;
    const USER_DATA_VA: usize = USER_DATA_VA;
    const ELF_MACHINE: u16 = vifast_pte::x86_64::ELF_MACHINE;

    fn init_platform(info: PlatformInfo) {
        kstore_canary_write();
        unsafe {
            init_acpi(info.acpi_rsdp);
            AP_TRAMPOLINE = info.ap_trampoline;
            build_kernel_page_tables();
            install_real_trampoline();
            init_cpu(info.hartid);
            calibrate_tsc_pit();
            // APs start before prepare_user(); install the shared IDT first so
            // their D4 idle window never runs with an empty IDT.
            setup_hart_gdt_tss_idt(0);
            // Vòng 98 (audit #9): áp NX toàn ảnh SAU khi IDT đã sẵn sàng — nếu có lỗi
            // mapping thì fault còn in được thay vì triple fault trước khi có IDT.
            // Vòng 97 (audit #8): NX toàn bộ ảnh trừ dải text đọc từ PE section
            // table — thay vì chỉ 20 vùng tĩnh, nhờ đó boot stack và các static
            // nhỏ không còn executable.
            if !image_pages_nx() {
                vifast_kernel::klog!(
                    X86Platform,
                    "NX FAIL: không đánh NX được toàn ảnh (image={:#x}..{:#x})",
                    image_range().0,
                    image_range().1
                );
                X86_64::exit(1);
            }
            {
                let mut text = [(0u64, 0u64); 8];
                let count = kernel_text_ranges(&mut text);
                vifast_kernel::klog!(
                    X86Platform,
                    "IMAGE TEXT NX OK ranges={} image={:#x}..{:#x}",
                    count,
                    image_range().0,
                    image_range().1
                );
            }

            let trampoline = AP_TRAMPOLINE;
            if trampoline != 0 {
                vifast_kernel::klog!(X86Platform, "AP TRAMPOLINE OK (page={:#x})", trampoline);
            } else {
                vifast_kernel::klog!(X86Platform, "SMP FAIL (trampoline alloc)");
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

    fn init_kernel() -> &'static mut Kernel {
        unsafe {
            let kernel = &mut *core::ptr::addr_of_mut!(KERNEL_STORAGE);
            kernel.reset();
            kernel
        }
    }

    fn fault_frame_complete() -> bool {
        true
    }

    fn prepare_user() {
        unsafe {
            // The GDT/TSS/IDT was installed in init_platform before the AP
            // started; do not rebuild it while the AP may be idle with IF=1.
            init_cpu(0);
            enable_smep_smap();
            enable_pcid();
            // Vòng 87: CR4 là per-hart nên BSP cũng phải tắt SSE cho user.
            disable_sse();
            vifast_kernel::klog!(X86Platform, "SSE USER OFF");
            if NX_ENABLED {
                vifast_kernel::klog!(X86Platform, "KERNEL DATA NX OK");
            }
            if PCID_ENABLED {
                vifast_kernel::klog!(X86Platform, "PCID ENABLED");
            }
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
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(USER_FRAME) as *mut u8, 393216)
        }
    }

    fn metadata_page() -> &'static mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(METADATA) as *mut u8, 81920)
        }
    }

    fn vspace_root() -> u64 {
        core::ptr::addr_of!(PML4) as u64
    }

    fn switch_root(root_pa: u64, asid: u16) -> bool {
        unsafe {
            if root_pa == 0 || (root_pa == active_root() && asid == active_asid()) {
                return false;
            }
            set_active_root(root_pa, asid);
            let cr3 = if PCID_ENABLED {
                root_pa | (((asid as u64) & 0x0fff) << 12)
            } else {
                root_pa
            };
            // A normal CR3 write invalidates the target PCID.  Other address
            // spaces keep their tagged TLB entries.
            core::arch::asm!("mov cr3, {}", in(reg) cr3);
            init_lapic();
            start_ap_real();
        }
        true
    }

    fn page_table_ops() -> &'static dyn vifast_kernel::PageTableOps {
        &PAGE_OPS
    }

    fn kernel_image_end() -> u64 {
        // Vòng 96 (audit #8): trước đây trả 0 nên check "KERNEL IMAGE OUTSIDE"
        // trong runtime không bao giờ chạy trên x86.
        image_range().1
    }

    fn check_kernel_mappings(root_pa: u64) -> bool {
        kernel_range_ok(root_pa)
            && kernel_wx_ok(root_pa)
            && kernel_leaf_ok(
                root_pa,
                if unsafe { ACPI_LAPIC_BASE == 0 } {
                    LAPIC_DEFAULT
                } else {
                    unsafe { ACPI_LAPIC_BASE }
                },
                true,
                false,
            )
            && kernel_leaf_ok(root_pa, unsafe { x86_exception_table[0] }, false, true)
            && kernel_data_nx(root_pa)
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(TRAP_STACKS) as u64,
                true,
                false,
            )
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(IST_STACKS) as u64, true, false)
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(GDTS) as u64, true, false)
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(TSSS) as u64, true, false)
            && kernel_leaf_ok(root_pa, core::ptr::addr_of!(IDT) as u64, true, false)
            && kernel_leaf_ok(
                root_pa,
                core::ptr::addr_of!(KERNEL_STORAGE) as u64,
                true,
                false,
            )
    }

    fn init_block() {
        // Vòng 84: quét PCI bus 0 (port I/O 0xCF8/0xCFC) — bằng chứng x86_64
        // truy cập được config space, nền tảng cho virtio-pci sau này.
        let (devices, vendor, device, class) = pci_scan();
        vifast_kernel::klog!(
            X86Platform,
            "PCI SCAN OK devices={} first={:04x}:{:04x} class={:04x}",
            devices,
            vendor,
            device,
            class
        );
    }

    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if !vifast_abi::user_layout::copy_window_ok(ptr, out.len() as usize) {
            return Err(KError::InvalidArg);
        }
        unsafe {
            if SMAP_ENABLED {
                core::arch::asm!("stac");
            }
            core::ptr::copy_nonoverlapping(ptr as *const u8, out.as_mut_ptr(), out.len());
            if SMAP_ENABLED {
                core::arch::asm!("clac");
            }
        }
        Ok(())
    }

    fn write_user(ptr: u64, input: &[u8]) -> Result<(), KError> {
        if !active_root_matches_current() {
            return Err(KError::IllegalOperation);
        }
        if !vifast_abi::user_layout::copy_window_ok(ptr, input.len() as usize) {
            return Err(KError::InvalidArg);
        }
        unsafe {
            if SMAP_ENABLED {
                core::arch::asm!("stac");
            }
            core::ptr::copy_nonoverlapping(input.as_ptr(), ptr as *mut u8, input.len());
            if SMAP_ENABLED {
                core::arch::asm!("clac");
            }
        }
        Ok(())
    }

    fn context_for_user(entry: usize, stack: usize) -> TrapFrame {
        TrapFrame::user(entry, stack)
    }

    fn set_context_arg0(context: &mut TrapFrame, value: u64) {
        context.rdi = value;
    }

    fn set_context_arg1(context: &mut TrapFrame, value: u64) {
        context.rsi = value;
    }
    fn contexts() -> &'static mut [TrapFrame] {
        unsafe { &mut *core::ptr::addr_of_mut!(CONTEXTS) }
    }

    fn enter_user(context: &TrapFrame) -> ! {
        let hart: usize;
        let trap_top: u64;
        unsafe {
            core::arch::asm!("mov {}, gs:[0]", out(reg) hart);
            core::arch::asm!("mov {}, gs:[16]", out(reg) trap_top);
        }
        assert!(hart < 2, "unexpected x86 per-CPU hart");
        assert!(trap_top != 0, "x86 per-CPU trap stack missing");
        assert_bkl_released();
        count_user_entry();
        // Entering ring 3: restore the user GS base and keep CpuState in
        // IA32_KERNEL_GS_BASE (standard swapgs convention).
        unsafe {
            core::arch::asm!("swapgs", options(nostack));
        }
        let base = context as *const TrapFrame as usize;
        unsafe {
            core::arch::asm!(
                "push {ss}", "push {rsp}", "push {rflags}", "push {cs}", "push {rip}",
                "mov r10, {base}",
                "mov r15, [r10 + {off_r15}]",
                "mov r14, [r10 + {off_r14}]",
                "mov r13, [r10 + {off_r13}]",
                "mov r12, [r10 + {off_r12}]",
                "mov r11, [r10 + {off_r11}]",
                "mov r9, [r10 + {off_r9}]",
                "mov r8, [r10 + {off_r8}]",
                "mov rbp, [r10 + {off_rbp}]",
                "mov rdi, [r10 + {off_rdi}]",
                "mov rsi, [r10 + {off_rsi}]",
                "mov rdx, [r10 + {off_rdx}]",
                "mov rcx, [r10 + {off_rcx}]",
                "mov rbx, [r10 + {off_rbx}]",
                "mov rax, [r10 + {off_rax}]",
                "mov r10, [r10 + {off_r10}]",
                "iretq",
                ss = in(reg) context.ss,
                rsp = in(reg) context.rsp,
                rflags = in(reg) context.rflags,
                cs = in(reg) context.cs,
                rip = in(reg) context.rip,
                base = in(reg) base,
                off_r15 = const core::mem::offset_of!(TrapFrame, r15),
                off_r14 = const core::mem::offset_of!(TrapFrame, r14),
                off_r13 = const core::mem::offset_of!(TrapFrame, r13),
                off_r12 = const core::mem::offset_of!(TrapFrame, r12),
                off_r11 = const core::mem::offset_of!(TrapFrame, r11),
                off_r10 = const core::mem::offset_of!(TrapFrame, r10),
                off_r9 = const core::mem::offset_of!(TrapFrame, r9),
                off_r8 = const core::mem::offset_of!(TrapFrame, r8),
                off_rbp = const core::mem::offset_of!(TrapFrame, rbp),
                off_rdi = const core::mem::offset_of!(TrapFrame, rdi),
                off_rsi = const core::mem::offset_of!(TrapFrame, rsi),
                off_rdx = const core::mem::offset_of!(TrapFrame, rdx),
                off_rcx = const core::mem::offset_of!(TrapFrame, rcx),
                off_rbx = const core::mem::offset_of!(TrapFrame, rbx),
                off_rax = const core::mem::offset_of!(TrapFrame, rax),
                options(noreturn),
            );
        }
    }

    fn write_result(
        context: &mut TrapFrame,
        error: KError,
        value: u64,
        label: u64,
        data: [u64; 4],
    ) {
        context.rax = error as u64;
        context.rdi = value;
        context.rsi = label;
        context.rdx = data[0];
        context.rcx = data[1];
        context.r8 = data[2];
        context.r9 = data[3];
        context.r10 = 0;
    }

    fn write_badge(context: &mut TrapFrame, badge: u64) {
        context.r10 = badge;
    }

    fn start_timer() {
        unsafe {
            setup_pic_pit();
        }
    }
    fn reload_timer() {}
    fn ack_timer() {
        // Vòng 89 (audit #4 P1-2): PIC thuộc BSP. Hart 1 chạy LAPIC timer và
        // đã EOI LAPIC trong x86_ap_irq_handler; nếu nó cũng ghi 0x20 thì cờ
        // in-service của BSP bị xoá -> mất tick PIT hoặc spurious IRQ7.
        if Self::hart_id() != 0 {
            return;
        }
        unsafe {
            outb(0x20, 0x20);
        }
    }

    fn block_read(lba: u64, out: &mut [u8; 512]) -> bool {
        unsafe { ata_read(lba, out) }
    }

    fn block_write(lba: u64, input: &[u8; 512]) -> bool {
        unsafe { ata_write(lba, input) }
    }

    fn block_flush() -> bool {
        unsafe { ata_flush() }
    }

    fn secondary_wakeups() -> (usize, usize) {
        (
            AP_TIMER_WAKES.load(Ordering::Relaxed),
            AP_IPI_WAKES.load(Ordering::Relaxed),
        )
    }

    fn secondary_timer_ticks() -> usize {
        AP_TIMER_TICKS.load(Ordering::Relaxed)
    }

    fn secondary_user_ticks() -> usize {
        AP_USER_TICKS.load(Ordering::Relaxed)
    }

    fn secondary_preempts() -> usize {
        AP_PREEMPTS.load(Ordering::Relaxed)
    }

    fn secondary_idle_entries() -> usize {
        AP_IDLE_ENTRIES.load(Ordering::Relaxed)
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

    /// T33 (r275): chẩn đoán — hart phụ còn sống/đang ngủ/đã nhận IPI chưa.
    fn stall_detail() -> Option<[usize; 6]> {
        Some([
            AP_ONLINE.load(Ordering::SeqCst) as usize,
            AP_IDLE_WINDOW.load(Ordering::SeqCst) as usize,
            AP_IDLE_GENERATION.load(Ordering::Relaxed),
            WAKE_LAST_GENERATION.load(Ordering::Relaxed),
            AP_IPI_SENT.load(Ordering::Relaxed),
            RESCHED_SENT.load(Ordering::Relaxed),
        ])
    }

    fn post_idle_ready() -> bool {
        if !POST_STARTED.load(Ordering::SeqCst) {
            return false;
        }
        // Vòng 150: chờ TƯỜNG MINH thay vì so tốc độ với số tick.
        // Ý nghĩa giữ nguyên: sau khi pha POST bắt đầu, hart phụ phải THỰC SỰ
        // vào idle (nếu nó quay vòng bận thì POST_IDLE_ENTRIES không tăng).
        // Điều kiện cũ `idle >= ticks - 2` đòi hart phụ vào idle gần như MỌI
        // tick; chỉ cần một worker chạy lâu hơn một chút là trượt vĩnh viễn —
        // đã làm đỏ gate oan ở vòng 116/141/149 dù hệ thống hoàn toàn khoẻ.
        const POST_IDLE_MIN: usize = 5;
        POST_IDLE_ENTRIES.load(Ordering::Relaxed) >= POST_IDLE_MIN
            && POST_TIMER_WAKES.load(Ordering::Relaxed) >= POST_IDLE_MIN
            && POST_TIMER_TICKS.load(Ordering::Relaxed) >= POST_IDLE_MIN
    }

    fn wake_hart(hart: usize) {
        if hart == 1 && AP_ONLINE.load(Ordering::SeqCst) && AP_IDLE_WINDOW.load(Ordering::SeqCst) {
            let generation = AP_IDLE_GENERATION.load(Ordering::SeqCst);
            if WAKE_LAST_GENERATION.swap(generation, Ordering::AcqRel) != generation {
                unsafe {
                    let base = lapic_base();
                    AP_IPI_SENT.fetch_add(1, Ordering::AcqRel);
                    RESCHED_SENT.fetch_add(1, Ordering::Relaxed);
                    AP_RESCHED_PENDING.store(true, Ordering::Release);
                    core::ptr::write_volatile((base + 0x310) as *mut u32, AP_APIC_ID << 24);
                    send_icr(base, APIC_MAINT_VECTOR);
                }
            }
        }
    }

    fn sched_harts() -> u32 {
        0b11
    }

    fn can_wait_for_work() -> bool {
        true
    }

    fn wait_for_work(hart: usize) {
        loop {
            let ready = {
                let bkl = bkl_lock();
                let ready = X86Platform::kernel().has_ready_on(hart);
                drop(bkl);
                ready
            };
            if ready {
                return;
            }
            if hart == 1 {
                AP_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
                AP_IDLE_GENERATION.fetch_add(1, Ordering::SeqCst);
                WAKE_SENT_THIS_WINDOW.store(false, Ordering::SeqCst);
                if POST_STARTED.load(Ordering::Relaxed) {
                    POST_IDLE_ENTRIES.fetch_add(1, Ordering::Relaxed);
                }
                AP_IDLE_WINDOW.store(true, Ordering::SeqCst);
                unsafe {
                    core::arch::asm!("sti", "hlt", "cli", options(nomem, nostack));
                }
                AP_IDLE_WINDOW.store(false, Ordering::SeqCst);
            } else {
                core::hint::spin_loop();
            }
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
        AP_IDLE_ENTRIES.load(Ordering::Relaxed)
    }

    fn timer_rate() -> Option<(usize, usize)> {
        let base = BSP_RATE_BASE.load(Ordering::Acquire);
        Some((
            AP_TIMER_TICKS.load(Ordering::Relaxed),
            BSP_PIT_TICKS.load(Ordering::Relaxed).saturating_sub(base),
        ))
    }

    fn ipi_count() -> Option<(usize, usize)> {
        Some((
            AP_IPI_SENT.load(Ordering::Relaxed),
            AP_IPI_SEEN.load(Ordering::Relaxed),
        ))
    }

    fn tlb_stats() -> Option<(usize, usize, usize)> {
        Some((
            TLB_VA_FLUSH.load(Ordering::Relaxed),
            TLB_IPI_SENT.load(Ordering::Relaxed),
            TLB_IPI_SKIPPED.load(Ordering::Relaxed),
        ))
    }

    fn kernel_storage_canary() -> Option<bool> {
        Some(kstore_canary_ok())
    }

    fn boot_stack_usage() -> Option<(usize, usize)> {
        extern "C" {
            static __vifast_boot_stack_bottom: u8;
            static __vifast_boot_stack_top: u8;
        }
        let bottom = core::ptr::addr_of!(__vifast_boot_stack_bottom) as usize;
        let top = core::ptr::addr_of!(__vifast_boot_stack_top) as usize;
        let mut cursor = bottom;
        while cursor < top {
            if unsafe { core::ptr::read_volatile(cursor as *const u8) } != 0xA5 {
                return Some((top - cursor, top - bottom));
            }
            cursor += 1;
        }
        Some((0, top - bottom))
    }

    fn start_smp() {
        // The AP is started from switch_root after the kernel page tables and
        // LAPIC are active.  D2: AP kernel IF=0, idle/poll via pause.
    }
}

// UEFI x86 uses the Win64 ABI: Rust entry points are `efiapi`, and trap.S
// passes the first three arguments in rcx/rdx/r8.
extern "efiapi" {
    fn spurious_entry();
    fn int80_entry();
    fn irq0_entry();
    fn irq1_entry();
    fn irq4_entry();
    fn ap_ipi_entry();
    fn ap_timer_entry();
    fn ap_halt_entry();
    fn pf_entry();
    static x86_exception_table: [u64; 32];
    fn x86_reload_segments();
}

#[no_mangle]
extern "efiapi" fn x86_exception_handler(frame: *const u64, vector: u64, error: u64) -> ! {
    if X86Platform::hart_id() != 0 {
        vifast_kernel::klog!(
            X86Platform,
            "AP EXCEPTION vector={} err={:#x} rip={:#x} cs={:#x} rsp={:#x}",
            vector,
            error,
            unsafe { *frame.add(17) },
            unsafe { *frame.add(18) },
            unsafe { *frame.add(20) }
        );
        X86Platform::exit(1);
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("exception");
    if matches!(vector, 2 | 8 | 18) {
        vifast_kernel::klog!(
            X86Platform,
            "FATAL EXCEPTION vector={} err={:#x}",
            vector,
            error
        );
        X86Platform::exit(1);
    }
    let cs = unsafe { *frame.add(18) };
    if cs & 3 == 0 {
        vifast_kernel::klog!(
            X86Platform,
            "KERNEL EXCEPTION vector={} err={:#x} cs={:#x}",
            vector,
            error,
            cs
        );
        X86Platform::exit(1);
    }
    // Vòng 90: frame thật đã thử nhưng trap.S đẩy thêm vector/error trước
    // SAVE_REGS nên cast thẳng rsp làm hỏng đường trả về user (GP vector=13).
    // Giữ frame rỗng; runtime vẫn lưu frame vào slot của TCB fault nên không
    // ảnh hưởng task khác.
    // vòng 92: frame thật (audit #6) — pager cứu được task và nó chạy lại
    // đúng instruction đã fault.
    let mut context = unsafe { TrapFrame::from_raw(frame, true) };
    vifast_runtime::fault::suspend::<X86Platform>(&mut context, vector, 0);
    drop(bkl);
    assert_bkl_released();
    <X86Platform as Platform>::enter_user(&context);
}

#[no_mangle]
extern "efiapi" fn x86_page_fault(_frame: *const u64, error: u64) -> ! {
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("pagefault");
    let cr2: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2);
    }
    /* error bit 2 is U/S: 1 means user mode, 0 means kernel mode. */
    if error & 0x4 == 0 {
        vifast_kernel::klog!(
            X86Platform,
            "KERNEL PAGE FAULT cr2={:#x} err={:#x}",
            cr2,
            error
        );
        X86_64::exit(1);
    }
    let mut context = unsafe { TrapFrame::from_raw(_frame, false) };
    vifast_runtime::fault::suspend::<X86Platform>(&mut context, error, cr2);
    drop(bkl);
    assert_bkl_released();
    <X86Platform as Platform>::enter_user(&context);
}

#[no_mangle]
extern "efiapi" fn x86_trap_handler(frame: *mut TrapFrame) {
    if vifast_arch::panic::is_panicking() {
        X86_64::halt();
    }
    assert_irqs_disabled();
    let frame = unsafe { &mut *frame };
    if frame.rax == vifast_abi::SyscallId::GetTime as u64 {
        frame.rax = 0;
        frame.rdi = X86Platform::kernel().fast_time();
        return;
    }
    let bkl = bkl_lock();
    assert_current_tcb("trap");
    let number = frame.rax;
    let args = [
        frame.rdi, frame.rsi, frame.rdx, frame.rcx, frame.r8, frame.r9,
    ];
    let waiting = vifast_runtime::syscall::dispatch::<X86Platform>(frame, number, args);
    drop(bkl);
    if waiting && X86Platform::hart_id() == 1 {
        loop {
            <X86Platform as Platform>::wait_for_work(1);
            let bkl = bkl_lock();
            if vifast_runtime::syscall::resume_blocked::<X86Platform>(frame) {
                drop(bkl);
                assert_bkl_released();
                count_user_entry();
                return;
            }
            drop(bkl);
        }
    }
    assert_bkl_released();
    count_user_entry();
}

#[no_mangle]
extern "efiapi" fn x86_uart_irq_handler(_frame: *mut TrapFrame) {
    if vifast_arch::panic::is_panicking() {
        X86_64::halt();
    }
    if X86Platform::hart_id() != 0 {
        panic!("AP COM1 IRQ");
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("irq4");
    unsafe {
        while inb(0x3FD) & 0x01 != 0 {
            let byte = inb(0x3F8);
            X86_64::stash_rx(byte);
        }
        outb(0x20, 0x20);
    }
    let woke = X86Platform::kernel().irq_raise(vifast_abi::IRQ_UART);
    if woke && X86Platform::kernel().has_ready_on(1) {
        X86Platform::wake_hart(1);
    }
    drop(bkl);
    assert_bkl_released();
}

/// Bảng scancode set 1 (bit 7 = phím nhả) → ASCII, không tính shift cho
/// các phím đã hỗ trợ; trả None cho phím không in được.
fn ps2_scancode_ascii(code: u8, shift: bool) -> Option<u8> {
    let byte = match code {
        0x02 => b'1',
        0x03 => b'2',
        0x04 => b'3',
        0x05 => b'4',
        0x06 => b'5',
        0x07 => b'6',
        0x08 => b'7',
        0x09 => b'8',
        0x0A => b'9',
        0x0B => b'0',
        0x0C => b'-',
        0x0D => b'=',
        0x10 => b'q',
        0x11 => b'w',
        0x12 => b'e',
        0x13 => b'r',
        0x14 => b't',
        0x15 => b'y',
        0x16 => b'u',
        0x17 => b'i',
        0x18 => b'o',
        0x19 => b'p',
        0x1A => b'[',
        0x1B => b']',
        0x1E => b'a',
        0x1F => b's',
        0x20 => b'd',
        0x21 => b'f',
        0x22 => b'g',
        0x23 => b'h',
        0x24 => b'j',
        0x25 => b'k',
        0x26 => b'l',
        0x27 => b';',
        0x28 => b'\'',
        0x29 => b'`',
        0x2B => b'\\',
        0x2C => b'z',
        0x2D => b'x',
        0x2E => b'c',
        0x2F => b'v',
        0x30 => b'b',
        0x31 => b'n',
        0x32 => b'm',
        0x33 => b',',
        0x34 => b'.',
        0x35 => b'/',
        0x39 => b' ',
        0x1C => b'\r',
        0x0E => 8, // backspace
        _ => return None,
    };
    if shift && byte.is_ascii_lowercase() {
        return Some(byte.to_ascii_uppercase());
    }
    Some(byte)
}

#[no_mangle]
extern "efiapi" fn x86_keyboard_irq_handler(_frame: *mut TrapFrame) {
    if vifast_arch::panic::is_panicking() {
        X86_64::halt();
    }
    if X86Platform::hart_id() != 0 {
        // Bàn phím chỉ phục vụ trên BSP; AP không đọc port 0x60.
        return;
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("irq1");
    static mut SHIFT: bool = false;
    static mut KEY_LOGGED: u32 = 0;
    static mut DOWN: u128 = 0;
    static mut EXTENDED: bool = false;
    let mut queued = false;
    unsafe {
        // 8259 là edge-triggered: một IRQ có thể mang nhiều scancode đang chờ,
        // nên phải đọc hết (bounded) trong một lần.
        let mut processed = 0;
        while processed < 16 && inb(0x64) & 0x01 != 0 {
            processed += 1;
            let scancode = inb(0x60);
            // 0xE0 mở đầu phím mở rộng (keypad, mũi tên); bỏ cả scancode kế.
            if scancode == 0xE0 {
                EXTENDED = true;
                continue;
            }
            if EXTENDED {
                EXTENDED = false;
                continue;
            }
            let released = scancode & 0x80 != 0;
            let code = (scancode & 0x7f) as usize;
            let bit = 1u128 << code;
            if released {
                DOWN &= !bit;
                if code == 0x2A || code == 0x36 {
                    SHIFT = false;
                }
                continue;
            }
            let first_press = DOWN & bit == 0;
            DOWN |= bit;
            if code == 0x2A || code == 0x36 {
                SHIFT = true;
                continue;
            }
            // Auto-repeat (set 1 không có cờ như virtio): chỉ nhận lần nhấn đầu.
            if !first_press {
                continue;
            }
            if KEY_LOGGED < 16 {
                KEY_LOGGED += 1;
                vifast_kernel::klog!(X86Platform, "PS2 KEY {}", scancode & 0x7f);
            }
            if let Some(byte) = ps2_scancode_ascii(scancode & 0x7f, SHIFT) {
                X86_64::stash_rx(byte);
                queued = true;
            }
        }
        outb(0x20, 0x20);
    }
    if queued {
        let woke = X86Platform::kernel().irq_raise(vifast_abi::IRQ_UART);
        if woke && X86Platform::kernel().has_ready_on(1) {
            X86Platform::wake_hart(1);
        }
    }
    drop(bkl);
    assert_bkl_released();
}

#[no_mangle]
extern "efiapi" fn x86_irq_handler(frame: *mut TrapFrame) {
    if vifast_arch::panic::is_panicking() {
        X86_64::halt();
    }
    if X86Platform::hart_id() != 0 {
        vifast_kernel::klog!(X86Platform, "AP PIC IRQ");
        panic!("AP PIC IRQ");
    }
    assert_irqs_disabled();
    let bkl = bkl_lock();
    assert_current_tcb("irq0");
    if cpu_state() == unsafe { core::ptr::addr_of!(CPU_STATES[0]) as *mut _ } {
        BSP_PIT_TICKS.fetch_add(1, Ordering::Relaxed);
    }
    let frame = unsafe { &mut *frame };
    vifast_runtime::timer::on_timer::<X86Platform>(frame);
    drop(bkl);
    assert_bkl_released();
}

#[no_mangle]
extern "efiapi" fn x86_ap_irq_handler(frame: *mut TrapFrame, vector: u64) {
    if vifast_arch::panic::is_panicking() {
        X86_64::halt();
    }
    assert_irqs_disabled();
    let cs = unsafe { (*frame).cs };
    if cs & 3 == 0 {
        // D4: the only legal ring-0 interrupt path is the idle window.
        if !AP_IDLE_WINDOW.load(Ordering::SeqCst) {
            vifast_kernel::klog!(X86Platform, "KERNEL IRQ OUTSIDE IDLE");
            X86Platform::exit(1);
        }
        if vector == APIC_TIMER_VECTOR as u64 {
            AP_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
            AP_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
            if POST_STARTED.load(Ordering::Relaxed) {
                POST_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
                POST_TIMER_WAKES.fetch_add(1, Ordering::Relaxed);
            }
        } else if vector == APIC_MAINT_VECTOR as u64 {
            AP_IPI_SEEN.fetch_add(1, Ordering::Relaxed);
            AP_IPI_WAKES.fetch_add(1, Ordering::Relaxed);
            if AP_RESCHED_PENDING.swap(false, Ordering::AcqRel) {
                RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
            }
            unsafe { process_maintenance() };
        }
        unsafe { lapic_eoi() };
        return;
    }
    // Ring-3 path keeps the existing preemption contract.
    let bkl = bkl_lock();
    assert_current_tcb("ap-user-irq");
    if vector == APIC_TIMER_VECTOR as u64 {
        unsafe { lapic_eoi() };
        AP_TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
        AP_USER_TICKS.fetch_add(1, Ordering::Relaxed);
        let changed = vifast_runtime::timer::on_timer::<X86Platform>(unsafe { &mut *frame });
        if changed {
            AP_PREEMPTS.fetch_add(1, Ordering::Relaxed);
        }
    } else if vector == APIC_MAINT_VECTOR as u64 {
        AP_IPI_SEEN.fetch_add(1, Ordering::Relaxed);
        if AP_RESCHED_PENDING.swap(false, Ordering::AcqRel) {
            RESCHED_WAKE.fetch_add(1, Ordering::Relaxed);
        }
        unsafe {
            process_maintenance();
            lapic_eoi();
        }
    }
    count_user_entry();
    drop(bkl);
    assert_bkl_released();
}
