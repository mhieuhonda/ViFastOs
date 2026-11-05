//! Reusable boot, syscall-dispatch and timer runtime for all ViFastOs ports.
//!
//! This crate contains no architecture instructions and no MMIO.  A concrete
//! port implements [`Platform`] in its `vifast-arch-*` crate; the kernel binary
//! only supplies the boot entry, linker script and user image.

#![no_std]
#![forbid(unsafe_code)]

use vifast_abi::{
    BootInfo, KError, Message, ObjType, Rights, SyscallId, BOOTINFO_MAGIC, BOOTINFO_VERSION,
    LABEL_FAULT,
};
use vifast_arch::Arch;
use vifast_kernel::console::Console;
use vifast_kernel::fs::MAX_PATH;
use vifast_kernel::{Cap, Kernel, StackSpec, TaskLayout, UserAction};

#[cfg(feature = "test-exit")]
pub const TIMER_TICKS: u64 = 20;
#[cfg(feature = "test-exit")]
static TEST_READY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
static TEST_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
static TEST_FS_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
static TEST_FS_LARGE_DONE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
static TEST_HARD_LINK_DONE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
static TEST_SYMLINK_DONE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
static TEST_TLB_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-skip-tlb-ipi")]
static TEST_TLB_BAD: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
static TEST_EXPECT_WORKERS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
pub const USER_COPY_MAX: usize = 256;

/// Boot information supplied by the architecture entry point.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlatformInfo {
    pub hartid: usize,
    pub dtb: u64,
    pub system_table: usize,
    pub acpi_rsdp: u64,
    pub ap_trampoline: u64,
    pub memory_base: u64,
    pub memory_size: u64,
}

/// A user image is copied into a Frame owned by the root VSpace.
#[derive(Clone, Copy)]
pub struct UserImage {
    pub code: &'static [u8],
    /// Offset of `user_thread_a` from the start of `code`.
    pub entry_a: usize,
    /// Offset of `user_thread_b` from the start of `code`.
    pub entry_b: usize,
    /// Root task ELF image, loaded into the root VSpace.
    pub root_elf: &'static [u8],
    /// Server/client boot-module ELF images.
    pub server_elf: &'static [u8],
    pub client_elf: &'static [u8],
}

/// The complete architecture/platform contract used by the shared runtime.
///
/// The kernel core only knows [`vifast_arch::Arch`].  This trait is the BSP
/// boundary: it owns MMU state, trap frames, timer/IRQ controllers, user
/// memory copies and the transition to user mode.
pub trait Platform: Arch + Console {
    type Context: Copy + 'static;

    const USER_CODE_VA: usize;
    const USER_DATA_VA: usize;
    /// ELF machine value accepted for the root task image.
    const ELF_MACHINE: u16;

    fn user_code_base() -> usize {
        Self::USER_CODE_VA
    }

    fn init_platform(info: PlatformInfo);
    fn init_kernel() -> &'static mut Kernel;
    fn set_kernel(kernel: *mut Kernel);
    fn kernel() -> &'static mut Kernel;

    fn data_page() -> &'static mut [u8];
    fn metadata_page() -> &'static mut [u8];
    /// Kernel-owned arena for boot task objects.  It is never installed into
    /// a user cspace.
    fn boot_page() -> &'static mut [u8];
    /// The static kernel-only page tables.  Retyped VSpaces copy the shared
    /// kernel slots from these tables before they gain user mappings.
    fn vspace_root() -> u64;
    /// Load `root_pa` with its address-space tag and invalidate that tag's
    /// stale TLB entries.  Returns true only when root or tag changed.
    fn switch_root(root_pa: u64, asid: u16) -> bool;

    fn page_table_ops() -> &'static dyn vifast_kernel::PageTableOps;
    /// Verify that trap entry, kernel stacks, descriptor tables, kernel
    /// storage and MMIO all live in KERNEL_SHARED_SLOTS before switching roots.
    fn check_kernel_mappings(root_pa: u64) -> bool;
    /// End of the boot kernel image/stack in physical memory, or 0 when the
    /// platform's linker layout does not expose such a symbol.
    fn kernel_image_end() -> u64;
    fn read_user(ptr: u64, out: &mut [u8]) -> Result<(), KError>;
    fn write_user(ptr: u64, input: &[u8]) -> Result<(), KError>;

    fn context_for_user(entry: usize, stack: usize) -> Self::Context;
    /// Set the first integer argument (`a0`/`x0`/`rdi`) of a fresh context.
    fn set_context_arg0(context: &mut Self::Context, value: u64);
    fn contexts() -> &'static mut [Self::Context];
    fn enter_user(context: &Self::Context) -> !;

    fn write_result(
        context: &mut Self::Context,
        error: KError,
        value: u64,
        label: u64,
        data: [u64; 4],
    );

    /// Write the badge to its dedicated ABI return register.  This is never
    /// shared with the ordinary message value word.
    fn write_badge(_context: &mut Self::Context, _badge: u64) {}

    fn write_message(context: &mut Self::Context, message: &Message) {
        let mut data = [0u64; 4];
        let words = message.words();
        let mut i = 0;
        while i < data.len() && i < words.len() {
            data[i] = words[i];
            i += 1;
        }
        Self::write_result(context, KError::Ok, 0, message.label, data);
        Self::write_badge(context, message.badge);
    }

    fn prepare_user() {}

    fn start_timer();
    fn reload_timer();
    fn ack_timer();
    fn start_smp();
    fn init_block() {}

    /// Publish the current TCB of this hart to the platform's per-CPU state.
    fn set_current_tcb(_tcb: Option<usize>) {}

    /// Hart whose kernel entry is currently executing.
    fn hart_id() -> usize {
        0
    }

    /// (timer wakeups, IPI wakeups) observed by secondary harts.
    fn secondary_wakeups() -> (usize, usize) {
        (0, 0)
    }

    fn secondary_timer_ticks() -> usize {
        0
    }

    /// Timer deliveries while hart 1 was in user mode.
    fn secondary_user_ticks() -> usize {
        0
    }

    /// Timer deliveries that switched a user task on hart 1.
    fn secondary_preempts() -> usize {
        0
    }

    fn secondary_idle_entries() -> usize {
        0
    }

    /// Optional x86 D4 cross-check: (AP timer ticks, BSP PIT ticks).
    fn timer_rate() -> Option<(usize, usize)> {
        None
    }

    /// Optional x86 D4 maintenance IPI accounting: (sent, seen).
    fn ipi_count() -> Option<(usize, usize)> {
        None
    }

    /// Optional per-VA TLB accounting: (VA flushes, IPIs, skipped IPIs).
    fn tlb_stats() -> Option<(usize, usize, usize)> {
        None
    }

    fn secondary_none_wakes() -> usize {
        0
    }

    /// Called when a non-primary hart blocks in Wait.  The platform uses this
    /// to start the post-worker idle accounting window.
    fn secondary_wait_blocked() {}

    fn post_idle_stats() -> Option<(usize, usize, usize)> {
        None
    }

    fn post_idle_ready() -> bool {
        true
    }

    fn boot_handoff_from() -> bool {
        false
    }

    /// Nudge an idle hart when a TCB with matching affinity becomes ready.
    fn wake_hart(_hart: usize) {}

    fn user_entries(_hart: usize) -> usize {
        0
    }

    /// `(acquisitions, spin iterations)` for each BKL hart.
    fn bkl_stats() -> [(usize, usize); 2] {
        [(0, 0), (0, 0)]
    }

    /// Harts that can schedule ordinary user tasks.
    fn sched_harts() -> u32 {
        0b01
    }

    /// True when a secondary hart may park in the platform idle loop instead
    /// of shutting the kernel down if its runqueue becomes empty.
    fn can_wait_for_work() -> bool {
        false
    }

    fn wait_for_work(_hart: usize) {
        core::hint::spin_loop();
    }

    fn resched_sent() -> usize {
        0
    }

    fn resched_wake() -> usize {
        0
    }

    fn idle_entries() -> usize {
        0
    }

    fn block_read(_lba: u64, _out: &mut [u8; 512]) -> bool {
        false
    }
    fn block_write(_lba: u64, _input: &[u8; 512]) -> bool {
        false
    }
    fn block_flush() -> bool {
        false
    }
}

fn write_bootinfo<A: Platform>(pa: u64, info: &BootInfo) {
    let ops = A::page_table_ops();
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, magic),
        &info.magic.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, version),
        &info.version.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, size),
        &info.size.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, root_vspace),
        &info.root_vspace.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, meta_untyped),
        &info.meta_untyped.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, data_untyped),
        &info.data_untyped.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, server_vspace),
        &info.server_vspace.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, user_code_first),
        &info.user_code_first.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, user_code_count),
        &info.user_code_count.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, free_slot_start),
        &info.free_slot_start.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, free_slot_end),
        &info.free_slot_end.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, block_device),
        &info.block_device.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, server_elf_va),
        &info.server_elf_va.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, server_elf_len),
        &info.server_elf_len.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, client_elf_va),
        &info.client_elf_va.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, client_elf_len),
        &info.client_elf_len.to_le_bytes(),
    );
    ops.copy_to_frame_at(
        pa,
        core::mem::offset_of!(BootInfo, sched_harts),
        &info.sched_harts.to_le_bytes(),
    );
}

/// Load the root owned by the current TCB, if it still has one.  The root is
/// deliberately not cached in a context: TcbConfigure can replace the VSpace,
/// and a revoked VSpace must never remain reachable through stale state.
pub fn ensure_current_root<A: Platform>() -> Result<(), KError> {
    let kernel = A::kernel();
    let hart = A::hart_id();
    let tcb = kernel.current_on(hart).ok_or(KError::IllegalOperation)?;
    let root = kernel.current_root(tcb).ok_or(KError::IllegalOperation)?;
    let asid = kernel.current_asid(tcb).unwrap_or(0);
    if root == 0 {
        return Err(KError::IllegalOperation);
    }
    if A::switch_root(root, asid) {
        kernel.note_root_switch();
    }
    kernel.note_vspace_hart_loaded(tcb, hart);
    Ok(())
}

fn note_current_tcb<A: Platform>(kernel: &Kernel) {
    let hart = A::hart_id();
    A::set_current_tcb(kernel.current_on(hart).map(|tcb| tcb.index as usize));
}

/// Shared boot sequence: kernel self-test, capability setup, user image copy
/// and the first user context.
pub fn boot<A: Platform>(info: PlatformInfo, image: UserImage) -> ! {
    A::init();
    A::irq_disable();
    A::init_platform(info);
    vifast_kernel::klog!(
        A,
        "ViFastOs booting on {} (hart={}, dtb={:#x})",
        <A as Console>::NAME,
        info.hartid,
        info.dtb
    );
    if A::boot_handoff_from() {
        vifast_kernel::klog!(A, "BOOT HANDOFF OK (from=1)");
    }

    let (selftest_ok, stats) = {
        let selftest_kernel = A::init_kernel();
        let ok = vifast_kernel::selftest::run::<A>(selftest_kernel);
        (ok, selftest_kernel.stats())
    };
    if !selftest_ok {
        vifast_kernel::klog!(A, "SELFTEST FAIL");
        A::exit(1);
    }
    let kernel = A::init_kernel();
    kernel.set_page_table_ops(A::page_table_ops());
    vifast_kernel::klog!(
        A,
        "SELFTEST PASS (ipc_tx={} ipc_rx={} ntfn={} ctx={} ticks={})",
        stats.ipc_sent,
        stats.ipc_received,
        stats.notifications,
        stats.context_switches,
        stats.ticks
    );
    A::start_smp();
    let root = kernel.root().unwrap_or_else(|| A::exit(1));
    /* The root task is the fault handler later; keep it above ordinary user
    threads so a queued fault message resumes it promptly. */
    if let Some(table) = kernel.tcb_mut(root) {
        table.priority = 15;
    }
    /* Runtime-owned objects: root TCB, VSpace, metadata Untyped, data Untyped. */
    const RUNTIME_OBJECTS: usize = 5;
    const META_MAX_OBJECTS: usize = 34;
    const DATA_MAX_OBJECTS: usize = 76;
    const BOOT_MAX_OBJECTS: usize = 32;
    if RUNTIME_OBJECTS + META_MAX_OBJECTS + DATA_MAX_OBJECTS + BOOT_MAX_OBJECTS
        > vifast_kernel::MAX_OBJECTS
    {
        vifast_kernel::klog!(A, "UNTYPED LIMIT EXCEEDS OBJECT TABLE");
        A::exit(1);
    }
    let data = A::data_page();
    let metadata = A::metadata_page();
    let boot = A::boot_page();
    let (accessible_lo, accessible_hi) = A::page_table_ops().kernel_accessible_range();
    let image_end = A::kernel_image_end();
    if image_end != 0 && (image_end < accessible_lo || image_end > accessible_hi) {
        vifast_kernel::klog!(A, "KERNEL IMAGE OUTSIDE KERNEL ACCESSIBLE RANGE");
        A::exit(1);
    }
    let accessible = |base: usize, len: usize| {
        accessible_lo >= accessible_hi
            || (base as u64 >= accessible_lo
                && (base as u64).saturating_add(len as u64) <= accessible_hi)
    };
    if !accessible(metadata.as_ptr() as usize, metadata.len())
        || !accessible(data.as_ptr() as usize, data.len())
        || !accessible(boot.as_ptr() as usize, boot.len())
    {
        vifast_kernel::klog!(A, "UNTYPED OUTSIDE KERNEL ACCESSIBLE RANGE");
        A::exit(1);
    }
    let Some(boot_untyped) = kernel.create_untyped_with_limit(
        boot.as_mut_ptr() as usize,
        boot.len(),
        A::PAGE_SIZE,
        BOOT_MAX_OBJECTS,
    ) else {
        A::exit(1);
    };
    let Some(meta_untyped) = kernel.create_untyped_with_limit(
        metadata.as_mut_ptr() as usize,
        metadata.len(),
        A::PAGE_SIZE,
        META_MAX_OBJECTS,
    ) else {
        A::exit(1);
    };
    let Some(data_untyped) = kernel.create_untyped_with_limit(
        data.as_mut_ptr() as usize,
        data.len(),
        A::PAGE_SIZE,
        DATA_MAX_OBJECTS,
    ) else {
        A::exit(1);
    };
    if kernel
        .install_cap(
            root,
            vifast_abi::user_layout::META_UNTYPED_SLOT as u8,
            Cap::new(
                meta_untyped,
                ObjType::Untyped,
                Rights::WRITE | Rights::GRANT,
            ),
        )
        .is_err()
        || kernel
            .install_cap(
                root,
                vifast_abi::user_layout::DATA_UNTYPED_SLOT as u8,
                Cap::new(
                    data_untyped,
                    ObjType::Untyped,
                    Rights::WRITE | Rights::GRANT,
                ),
            )
            .is_err()
    {
        A::exit(1);
    }

    let Ok(block_device) = kernel.retype(meta_untyped, ObjType::BlockDevice) else {
        A::exit(1);
    };
    if kernel
        .install_cap(
            root,
            vifast_abi::user_layout::BLOCK_DEVICE_SLOT as u8,
            Cap::new(
                block_device,
                ObjType::BlockDevice,
                Rights::READ | Rights::WRITE | Rights::GRANT,
            ),
        )
        .is_err()
    {
        A::exit(1);
    }

    let page_size = A::PAGE_SIZE as u64;
    let layout = TaskLayout {
        code_va: vifast_abi::user_layout::CODE_VA as u64,
        stack_va: vifast_abi::user_layout::STACK_BASE as u64,
        /* Root stacks are mapped page-by-page below.  The server stack page
        must stay invalid in the root VSpace so the isolation fault is real. */
        stack_pages: 0,
        /* The client owns DATA_VA and maps its own Frame there.  The builder
        must not consume that mapping. */
        data: None,
    };
    let Ok(vspace) = kernel.build_task_space(boot_untyped, image.code, layout) else {
        vifast_kernel::klog!(A, "ROOT VSPACE BUILD FAILED");
        A::exit(1);
    };
    let Some(root_pa) = kernel.vspace(vspace).map(|space| space.root) else {
        A::exit(1);
    };
    /* The root task shares its VSpace with the client bootstrap.  Keep the
    legacy hint-0 allocation point at DATA_VA without mapping a data frame;
    the client must still FrameMap it explicitly. */
    if let Some(space) = kernel.vspace_mut(vspace) {
        space.next_va = vifast_abi::user_layout::DATA_VA as u64;
    }
    /* Root VSpace maps only the fault-thread stacks.  Client and fault4
    stacks live in the private client VSpace; server stack belongs to the
    server VSpace. */
    const ROOT_USER_STACK_BASES: [usize; 3] = [
        vifast_abi::user_layout::FAULT1_STACK_BASE,
        vifast_abi::user_layout::FAULT2_STACK_BASE,
        vifast_abi::user_layout::FAULT3_STACK_BASE,
    ];
    for va in ROOT_USER_STACK_BASES {
        let Ok(frame) = kernel.retype(boot_untyped, ObjType::Frame) else {
            vifast_kernel::klog!(A, "ROOT STACK RETYPE FAILED");
            A::exit(1);
        };
        if kernel
            .map_frame(vspace, frame, va as u64, Rights::READ | Rights::WRITE)
            .is_err()
        {
            vifast_kernel::klog!(A, "ROOT STACK MAP FAILED va={:#x}", va);
            A::exit(1);
        }
    }

    let root_elf = match vifast_elf::parse(
        image.root_elf,
        A::ELF_MACHINE,
        vifast_abi::user_layout::ROOT_ELF_BASE as u64,
        vifast_abi::user_layout::ROOT_ELF_STACK_GUARD as u64,
    ) {
        Ok(image) => image,
        Err(error) => {
            vifast_kernel::klog!(A, "ROOT ELF INVALID: {:?}", error);
            A::exit(1);
        }
    };
    if kernel
        .build_task_space_elf(
            boot_untyped,
            Some(vspace),
            &root_elf,
            image.root_elf,
            StackSpec {
                base: vifast_abi::user_layout::ROOT_ELF_STACK_BASE as u64,
                pages: 4,
                guard: true,
            },
        )
        .is_err()
    {
        vifast_kernel::klog!(A, "ROOT ELF BUILD FAILED");
        A::exit(1);
    }
    if let Some(space) = kernel.vspace_mut(vspace) {
        space.next_va = vifast_abi::user_layout::DATA_VA as u64;
    }
    if kernel.set_tcb_vspace(root, vspace).is_err() {
        A::exit(1);
    }
    vifast_kernel::klog!(
        A,
        "ROOT ELF entry={:#x} segments={} stack={:#x}",
        root_elf.entry,
        root_elf.segment_count(),
        vifast_abi::user_layout::ROOT_ELF_STACK_TOP
    );
    if kernel
        .validate_user_range(root, root_elf.entry, 1, false)
        .is_err()
    {
        vifast_kernel::klog!(A, "ROOT ELF ENTRY NOT MAPPED");
        A::exit(1);
    }
    if kernel
        .validate_user_range(
            root,
            vifast_abi::user_layout::ROOT_ELF_BASE as u64 + 0x1000,
            1,
            false,
        )
        .is_err()
    {
        vifast_kernel::klog!(A, "ROOT ELF RODATA NOT MAPPED");
        A::exit(1);
    }

    if image.code.len() > A::PAGE_SIZE {
        vifast_kernel::klog!(A, "SHARED CODE REQUIRES ONE PAGE");
        A::exit(1);
    }
    let Some(code_pa) = kernel.mapping_at(vspace, layout.code_va).map(|m| m.pa) else {
        A::exit(1);
    };
    let Some(code_frame) = kernel.frame_for_pa(code_pa) else {
        A::exit(1);
    };
    let server_layout = TaskLayout {
        code_va: vifast_abi::user_layout::CODE_VA as u64,
        stack_va: vifast_abi::user_layout::SERVER_STACK_BASE as u64,
        stack_pages: 3,
        data: None,
    };
    let Ok(server_space) =
        kernel.build_task_space_shared_code(boot_untyped, code_frame, server_layout)
    else {
        vifast_kernel::klog!(A, "SERVER VSPACE BUILD FAILED");
        A::exit(1);
    };
    let Some(server_pa) = kernel.vspace(server_space).map(|space| space.root) else {
        A::exit(1);
    };
    if !A::check_kernel_mappings(root_pa) || !A::check_kernel_mappings(server_pa) {
        vifast_kernel::klog!(
            A,
            "KERNEL MAPPING CHECK FAILED root={:#x} server={:#x}",
            root_pa,
            server_pa
        );
        A::exit(1);
    }
    if kernel.set_tcb_vspace(root, vspace).is_err()
        || kernel
            .install_cap(
                root,
                vifast_abi::user_layout::ROOT_VSPACE_SLOT as u8,
                Cap::new(vspace, ObjType::VSpace, Rights::READ | Rights::WRITE),
            )
            .is_err()
        || kernel
            .install_cap(
                root,
                vifast_abi::user_layout::SERVER_VSPACE_SLOT as u8,
                Cap::new(server_space, ObjType::VSpace, Rights::READ | Rights::WRITE),
            )
            .is_err()
    {
        A::exit(1);
    }

    /* The boot image code is read+execute only.  Root needs these Frame caps
    to build the client VSpace in 7.4 without gaining permission to rewrite
    the shared code. */
    const USER_CODE_FIRST_SLOT: u16 = 20;
    let user_code_count = ((image.code.len() + A::PAGE_SIZE - 1) / A::PAGE_SIZE) as u16;
    let free_slot_start = USER_CODE_FIRST_SLOT + user_code_count;
    let free_slot_end = vifast_kernel::MAX_CAPS as u16;
    if user_code_count == 0 || free_slot_start.saturating_add(8) > free_slot_end {
        vifast_kernel::klog!(A, "BOOT SLOT RANGE TOO SMALL");
        A::exit(1);
    }
    for index in 0..user_code_count {
        let va = layout.code_va + index as u64 * page_size;
        let Some(code_pa) = kernel.mapping_at(vspace, va).map(|mapping| mapping.pa) else {
            A::exit(1);
        };
        let Some(code_frame) = kernel.frame_for_pa(code_pa) else {
            A::exit(1);
        };
        let slot = USER_CODE_FIRST_SLOT + index;
        if kernel
            .install_cap(
                root,
                slot as u8,
                Cap::new(code_frame, ObjType::Frame, Rights::READ | Rights::EXEC),
            )
            .is_err()
        {
            vifast_kernel::klog!(A, "USER CODE CAP INSTALL FAILED slot={}", slot);
            A::exit(1);
        }
    }

    /* Stage boot-module ELF bytes into read-only frames in the root VSpace.
    The runtime writes them through the kernel identity map; the root loader
    then parses the slices and copies segments into child Frames. */
    let module_pages = |bytes: &[u8]| (bytes.len() + A::PAGE_SIZE - 1) / A::PAGE_SIZE;
    if image.server_elf.is_empty() || image.client_elf.is_empty() {
        vifast_kernel::klog!(A, "BOOT MODULE EMPTY");
        A::exit(1);
    }
    for (bytes, base) in [
        (
            image.server_elf,
            vifast_abi::user_layout::ROOT_MODULE_SERVER_VA as u64,
        ),
        (
            image.client_elf,
            vifast_abi::user_layout::ROOT_MODULE_CLIENT_VA as u64,
        ),
    ] {
        let pages = module_pages(bytes);
        for page in 0..pages {
            let Ok(frame) = kernel.retype(data_untyped, ObjType::Frame) else {
                vifast_kernel::klog!(A, "BOOT MODULE FRAME FAILED");
                A::exit(1);
            };
            let start = page * A::PAGE_SIZE;
            let end = core::cmp::min(start + A::PAGE_SIZE, bytes.len());
            if kernel
                .copy_frame_bytes(frame, 0, &bytes[start..end])
                .is_err()
            {
                vifast_kernel::klog!(A, "BOOT MODULE COPY FAILED");
                A::exit(1);
            }
            if kernel
                .map_frame(
                    vspace,
                    frame,
                    base + (page * A::PAGE_SIZE) as u64,
                    Rights::READ,
                )
                .is_err()
            {
                vifast_kernel::klog!(A, "BOOT MODULE MAP FAILED");
                A::exit(1);
            }
        }
    }

    let Ok(boot_info_frame) = kernel.retype(boot_untyped, ObjType::Frame) else {
        vifast_kernel::klog!(A, "BOOTINFO FRAME RETYPE FAILED");
        A::exit(1);
    };
    let Some(boot_info_pa) = kernel.frame(boot_info_frame).map(|frame| frame.pa as u64) else {
        A::exit(1);
    };
    let sched_harts = A::sched_harts();
    assert!(
        sched_harts & 0b01 != 0,
        "BootInfo must include the boot hart"
    );
    assert!(
        sched_harts & !0b11 == 0,
        "BootInfo advertises an unsupported hart"
    );
    let boot_info = BootInfo {
        magic: BOOTINFO_MAGIC,
        version: BOOTINFO_VERSION,
        size: core::mem::size_of::<BootInfo>() as u32,
        root_vspace: vifast_abi::user_layout::ROOT_VSPACE_SLOT as u16,
        meta_untyped: vifast_abi::user_layout::META_UNTYPED_SLOT as u16,
        data_untyped: vifast_abi::user_layout::DATA_UNTYPED_SLOT as u16,
        server_vspace: vifast_abi::user_layout::SERVER_VSPACE_SLOT as u16,
        user_code_first: USER_CODE_FIRST_SLOT,
        user_code_count,
        free_slot_start,
        free_slot_end,
        block_device: vifast_abi::user_layout::BLOCK_DEVICE_SLOT as u16,
        server_elf_va: vifast_abi::user_layout::ROOT_MODULE_SERVER_VA as u64,
        server_elf_len: image.server_elf.len() as u64,
        client_elf_va: vifast_abi::user_layout::ROOT_MODULE_CLIENT_VA as u64,
        client_elf_len: image.client_elf.len() as u64,
        sched_harts,
    };
    write_bootinfo::<A>(boot_info_pa, &boot_info);
    if kernel
        .map_frame(
            vspace,
            boot_info_frame,
            vifast_abi::user_layout::ROOT_BOOTINFO_VA as u64,
            Rights::READ,
        )
        .is_err()
    {
        vifast_kernel::klog!(A, "BOOTINFO MAP FAILED");
        A::exit(1);
    }
    if let Some(space) = kernel.vspace_mut(vspace) {
        space.next_va = vifast_abi::user_layout::DATA_VA as u64;
    }

    if let Some((size, bump, objects, max_objects)) = kernel.untyped_usage(boot_untyped) {
        let total_pages = size / page_size as usize;
        let used_pages = bump / page_size as usize;
        let free_pages = total_pages.saturating_sub(used_pages);
        let free_objects = max_objects.saturating_sub(objects);
        vifast_kernel::klog!(
            A,
            "BOOT UNTYPED pages={}/{} objects={}/{} free_pages={} free_objects={}",
            used_pages,
            total_pages,
            objects,
            max_objects,
            free_pages,
            free_objects
        );
        if free_pages < 2 || free_objects < 2 {
            vifast_kernel::klog!(A, "BOOT UNTYPED TOO SMALL FOR BOOT INFO");
            A::exit(1);
        }
    } else {
        A::exit(1);
    }
    if let Some((size, bump, objects, max_objects)) = kernel.untyped_usage(data_untyped) {
        let pages = size / page_size as usize;
        let used = bump / page_size as usize;
        vifast_kernel::klog!(
            A,
            "DATA UNTYPED pages={}/{} objects={}/{}",
            used,
            pages,
            objects,
            max_objects
        );
        if max_objects < 6 {
            vifast_kernel::klog!(A, "DATA UNTYPED TOO SMALL FOR CLIENT VSPACE");
            A::exit(1);
        }
    }
    A::prepare_user();

    let contexts = A::contexts();
    let mut root_context = A::context_for_user(
        root_elf.entry as usize,
        vifast_abi::user_layout::ROOT_ELF_STACK_TOP,
    );
    A::set_context_arg0(
        &mut root_context,
        vifast_abi::user_layout::ROOT_BOOTINFO_VA as u64,
    );
    contexts[root.index as usize] = root_context;

    A::set_kernel(kernel as *mut Kernel);
    kernel.enqueue(root);
    let _ = kernel.pick_on(A::hart_id());
    note_current_tcb::<A>(kernel);
    if ensure_current_root::<A>().is_err() {
        A::exit(1);
    }
    A::init_block();
    let mut block_probe = [0u8; 512];
    if A::block_read(0, &mut block_probe) {
        vifast_kernel::klog!(A, "BLOCK OK");
    } else {
        vifast_kernel::klog!(A, "BLOCK SKIP");
    }
    A::start_timer();
    #[cfg(feature = "test-exit")]
    TEST_EXPECT_WORKERS.store(
        A::sched_harts() & 0b10 != 0,
        core::sync::atomic::Ordering::SeqCst,
    );
    vifast_kernel::klog!(A, "entering user mode at {:#x}", A::user_code_base());
    let first = contexts[root.index as usize];
    A::enter_user(&first)
}

/// Syscall/action dispatch shared by all trap entry implementations.
pub mod syscall {
    use super::*;

    pub fn dispatch<A: Platform>(context: &mut A::Context, number: u64, args: [u64; 6]) -> bool {
        let mut waiting = false;
        let kernel = A::kernel();
        let hart = A::hart_id();
        let Some(tcb) = kernel.current_on(hart) else {
            A::exit(1);
        };
        let action = kernel.user_syscall_on(
            tcb, hart, number, args[0], args[1], args[2], args[3], args[4], args[5],
        );
        if hart != 0 && number == SyscallId::Wait as u64 && matches!(&action, UserAction::Blocked) {
            A::secondary_wait_blocked();
        }
        if hart == 0 && kernel.has_ready_on(1) {
            A::wake_hart(1);
        }
        match action {
            UserAction::Return { value, error } => {
                A::write_result(context, error, value, 0, [0; 4]);
            }
            UserAction::Message {
                error,
                value,
                label,
                data,
            } => {
                A::write_result(context, error, 0, label, data);
                A::write_badge(context, value);
            }
            UserAction::ConfigureTcb { tcb, entry, stack } => {
                let contexts = A::contexts();
                if let Some(slot) = contexts.get_mut(tcb.index as usize) {
                    *slot = A::context_for_user(entry as usize, stack as usize);
                    A::write_result(context, KError::Ok, 0, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::IllegalOperation, 0, 0, [0; 4]);
                }
            }
            UserAction::RestartTcb { tcb, entry, stack } => {
                let contexts = A::contexts();
                if let Some(slot) = contexts.get_mut(tcb.index as usize) {
                    *slot = A::context_for_user(entry as usize, stack as usize);
                    A::write_result(context, KError::Ok, 0, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::IllegalOperation, 0, 0, [0; 4]);
                }
            }
            UserAction::Open { ptr, len, flags } => {
                let len = (len as usize).min(MAX_PATH);
                let mut path = [0u8; MAX_PATH];
                let ok = len != 0 && A::read_user(ptr, &mut path[..len]).is_ok();
                if !ok {
                    A::write_result(context, KError::InvalidArg, 0, 0, [0; 4]);
                } else {
                    let result = kernel.open_path(tcb, &path[..len], flags);
                    match result {
                        Ok(fd) => A::write_result(context, KError::Ok, fd, 0, [0; 4]),
                        Err(error) => A::write_result(context, error, 0, 0, [0; 4]),
                    }
                }
            }
            UserAction::Read { fd, ptr, len } => {
                let len = (len as usize).min(USER_COPY_MAX);
                let mut buf = [0u8; USER_COPY_MAX];
                match kernel.read_fd(tcb, fd, &mut buf[..len]) {
                    Ok(n) => {
                        if A::write_user(ptr, &buf[..n]).is_ok() {
                            A::write_result(context, KError::Ok, n as u64, 0, [0; 4]);
                        } else {
                            A::write_result(context, KError::InvalidArg, 0, 0, [0; 4]);
                        }
                    }
                    Err(error) => A::write_result(context, error, 0, 0, [0; 4]),
                }
            }
            UserAction::Write { fd, ptr, len } => {
                let len = (len as usize).min(USER_COPY_MAX);
                let mut buf = [0u8; USER_COPY_MAX];
                if A::read_user(ptr, &mut buf[..len]).is_err() {
                    A::write_result(context, KError::InvalidArg, 0, 0, [0; 4]);
                } else {
                    match kernel.write_fd(tcb, fd, &buf[..len]) {
                        Ok(n) => A::write_result(context, KError::Ok, n as u64, 0, [0; 4]),
                        Err(error) => A::write_result(context, error, 0, 0, [0; 4]),
                    }
                }
            }
            UserAction::TcbSuspended { hart } => {
                if let Some(hart) = hart {
                    A::wake_hart(hart);
                }
                A::write_result(context, KError::Ok, 0, 0, [0; 4]);
            }
            UserAction::FrameRemap {
                space,
                frame,
                va,
                rights,
            } => match kernel.remap_frame(space, frame, va, rights) {
                Ok(mapping) => A::write_result(context, KError::Ok, mapping.va, 0, [0; 4]),
                Err(error) => A::write_result(context, error, 0, 0, [0; 4]),
            },
            UserAction::FrameWrite {
                frame,
                offset,
                ptr,
                len,
            } => {
                let len = (len as usize).min(USER_COPY_MAX);
                let mut buf = [0u8; USER_COPY_MAX];
                if A::read_user(ptr, &mut buf[..len]).is_err() {
                    A::write_result(context, KError::InvalidArg, 0, 0, [0; 4]);
                } else {
                    match kernel.copy_frame_bytes(frame, offset as usize, &buf[..len]) {
                        Ok(()) => A::write_result(context, KError::Ok, 0, 0, [0; 4]),
                        Err(error) => A::write_result(context, error, 0, 0, [0; 4]),
                    }
                }
            }
            UserAction::BlockRead { lba, ptr } => {
                let mut buf = [0u8; 512];
                if A::block_read(lba, &mut buf) && A::write_user(ptr, &buf).is_ok() {
                    A::write_result(context, KError::Ok, 512, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::IllegalOperation, 0, 0, [0; 4]);
                }
            }
            UserAction::BlockWrite { lba, ptr } => {
                let mut buf = [0u8; 512];
                if A::read_user(ptr, &mut buf).is_err() || !A::block_write(lba, &buf) {
                    A::write_result(context, KError::IllegalOperation, 0, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::Ok, 512, 0, [0; 4]);
                }
            }
            UserAction::BlockFlush => {
                if A::block_flush() {
                    A::write_result(context, KError::Ok, 0, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::IllegalOperation, 0, 0, [0; 4]);
                }
            }
            UserAction::Debug { ptr, len } => {
                let len = (len as usize).min(USER_COPY_MAX);
                let mut buf = [0u8; USER_COPY_MAX];
                if A::read_user(ptr, &mut buf[..len]).is_err() {
                    A::write_result(context, KError::InvalidArg, 0, 0, [0; 4]);
                } else {
                    #[cfg(feature = "test-exit")]
                    {
                        let ready = buf[..len]
                            .windows(b"ROOT READY".len())
                            .any(|window| window == b"ROOT READY");
                        if ready {
                            TEST_READY.store(true, core::sync::atomic::Ordering::SeqCst);
                            if !TEST_EXPECT_WORKERS.load(core::sync::atomic::Ordering::SeqCst) {
                                TEST_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
                            }
                        }
                        let workers = buf[..len]
                            .windows(b"SMP WORKERS OK".len())
                            .any(|window| window == b"SMP WORKERS OK");
                        if workers {
                            TEST_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
                        }
                        let fs_done = buf[..len]
                            .windows(b"FS PERM OK".len())
                            .any(|window| window == b"FS PERM OK");
                        if fs_done {
                            TEST_FS_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
                        }
                        let fs_large_done = buf[..len]
                            .windows(b"FS LARGE OK".len())
                            .any(|window| window == b"FS LARGE OK");
                        if fs_large_done {
                            TEST_FS_LARGE_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
                        }
                        let hard_link_done = buf[..len]
                            .windows(b"HARD LINK OK".len())
                            .any(|window| window == b"HARD LINK OK");
                        if hard_link_done {
                            TEST_HARD_LINK_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
                        }
                        let symlink_done = buf[..len]
                            .windows(b"SYMLINK OK".len())
                            .any(|window| window == b"SYMLINK OK");
                        if symlink_done {
                            TEST_SYMLINK_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
                        }
                        let tlb_done = buf[..len]
                            .windows(b"TLB REMAP OK".len())
                            .any(|window| window == b"TLB REMAP OK");
                        if tlb_done {
                            TEST_TLB_DONE.store(true, core::sync::atomic::Ordering::SeqCst);
                        }
                        #[cfg(feature = "test-skip-tlb-ipi")]
                        {
                            let tlb_bad = buf[..len]
                                .windows(b"TLB REMAP BAD".len())
                                .any(|window| window == b"TLB REMAP BAD");
                            if tlb_bad {
                                TEST_TLB_BAD.store(true, core::sync::atomic::Ordering::SeqCst);
                            }
                        }
                    }
                    for byte in &buf[..len] {
                        A::putc(*byte);
                    }
                    A::write_result(context, KError::Ok, 0, 0, [0; 4]);
                }
            }
            UserAction::Blocked => {
                if !switch_blocked::<A>(context) {
                    if A::can_wait_for_work() && hart != 0 {
                        waiting = true;
                    } else {
                        A::exit(1);
                    }
                }
            }
            UserAction::ConsoleRead => {
                if let Some(byte) = A::getc() {
                    A::write_result(context, KError::Ok, byte as u64, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::WouldBlock, 0, 0, [0; 4]);
                }
            }
            UserAction::Sleep => {
                if !switch_blocked::<A>(context) {
                    if A::can_wait_for_work() {
                        A::wait_for_work(hart);
                        if !switch_blocked::<A>(context) {
                            A::exit(1);
                        }
                    } else {
                        A::exit(1);
                    }
                }
            }
            UserAction::Exit => {
                A::write_str("\r\nUSER EXIT\r\n");
                A::exit(0);
            }
        }
        if ensure_current_root::<A>().is_err() {
            fault::suspend::<A>(context, 0, 0);
        }
        waiting
    }

    pub fn apply_resume<A: Platform>(context: &mut A::Context, tcb: vifast_kernel::ObjRef) {
        let kernel = A::kernel();
        if let Some(error) = kernel.take_resume_error(tcb) {
            A::write_result(context, error, 0, 0, [0; 4]);
        } else if let Some(message) = kernel.take_resume(tcb) {
            A::write_message(context, &message);
        }
    }

    /// Save the blocked task.  Returns false when its hart has no other
    /// runnable task; the caller may park and retry after an IPI.
    pub fn switch_blocked<A: Platform>(context: &mut A::Context) -> bool {
        let kernel = A::kernel();
        let contexts = A::contexts();
        let hart = A::hart_id();
        if let Some(old) = kernel.current_on(hart) {
            contexts[old.index as usize] = *context;
        }
        let Some(next) = kernel.pick_on(hart) else {
            return false;
        };
        note_current_tcb::<A>(kernel);
        *context = contexts[next.index as usize];
        crate::syscall::apply_resume::<A>(context, next);
        if ensure_current_root::<A>().is_err() {
            fault::suspend::<A>(context, 0, 0);
        }
        true
    }

    /// Resume any task that became ready while a secondary hart was parked.
    pub fn resume_blocked<A: Platform>(context: &mut A::Context) -> bool {
        let kernel = A::kernel();
        let contexts = A::contexts();
        let hart = A::hart_id();
        let Some(next) = kernel.pick_on(hart) else {
            return false;
        };
        note_current_tcb::<A>(kernel);
        *context = contexts[next.index as usize];
        crate::syscall::apply_resume::<A>(context, next);
        if ensure_current_root::<A>().is_err() {
            fault::suspend::<A>(context, 0, 0);
        }
        true
    }
}

/// Minimal user-fault suspension.  A full fault endpoint is future work;
/// for now the faulting TCB becomes Faulted and another TCB continues.
pub mod fault {
    use super::*;

    pub fn suspend<A: Platform>(context: &mut A::Context, cause: u64, addr: u64) {
        let kernel = A::kernel();
        let hart = A::hart_id();
        let old = kernel.current_on(hart);
        vifast_kernel::klog!(
            A,
            "USER FAULT tcb={:?} cause={:#x} addr={:#x}",
            old,
            cause,
            addr
        );
        let next = match old {
            Some(old) if kernel.fault_endpoint(old).is_some() => {
                let endpoint = kernel.fault_endpoint(old).unwrap();
                let fault_slot = kernel
                    .fault_endpoint_slot(old)
                    .map(u64::from)
                    .unwrap_or(u64::MAX);
                match Message::new(LABEL_FAULT, &[addr, cause, fault_slot, old.index as u64]) {
                    Ok(message) => match kernel.fault_to_endpoint(old, endpoint, message) {
                        Ok(()) => {
                            let next = kernel.pick_on(hart);
                            note_current_tcb::<A>(kernel);
                            next
                        }
                        Err(error) => {
                            vifast_kernel::klog!(A, "FAULT ENDPOINT FAILED: {:?}", error);
                            kernel.fault_current_on(hart)
                        }
                    },
                    Err(_) => kernel.fault_current_on(hart),
                }
            }
            _ => kernel.fault_current_on(hart),
        };
        let Some(mut next) = next else {
            vifast_kernel::klog!(A, "no runnable thread after fault");
            A::halt();
        };
        note_current_tcb::<A>(kernel);
        loop {
            *context = A::contexts()[next.index as usize];
            crate::syscall::apply_resume::<A>(context, next);
            if ensure_current_root::<A>().is_ok() {
                return;
            }
            vifast_kernel::klog!(A, "FAULT SUSPEND tcb={:?}: no VSpace", next);
            let Some(candidate) = kernel.fault_current_on(hart) else {
                vifast_kernel::klog!(A, "no runnable thread with VSpace after fault");
                A::halt();
            };
            note_current_tcb::<A>(kernel);
            next = candidate;
        }
    }
}

/// Timer tick, accounting and preemptive context switch.
pub mod timer {
    use super::*;

    pub fn on_timer<A: Platform>(context: &mut A::Context) -> bool {
        let kernel = A::kernel();
        let contexts = A::contexts();
        let hart = A::hart_id();
        let old = kernel.current_on(hart);
        let old_index = old.map(|tcb| tcb.index);
        if let Some(old) = old {
            contexts[old.index as usize] = *context;
        }
        let woke = kernel.tick(hart);
        let irq_woke = kernel.irq_raise(vifast_abi::IRQ_TIMER);
        if hart == 0 && (woke != 0 || irq_woke) && kernel.has_ready_on(1) {
            A::wake_hart(1);
        }
        #[cfg(feature = "test-skip-tlb-ipi")]
        if hart == 0 && TEST_TLB_BAD.load(core::sync::atomic::Ordering::SeqCst) {
            vifast_kernel::klog!(A, "TLB MUTATION DETECTED");
            A::exit(0);
        }
        #[cfg(feature = "test-panic-hart1")]
        if hart == 1 {
            panic!("test-panic-hart1");
        }
        #[cfg(feature = "test-exit")]
        if hart == 0
            && TEST_DONE.load(core::sync::atomic::Ordering::SeqCst)
            && TEST_FS_DONE.load(core::sync::atomic::Ordering::SeqCst)
            && TEST_FS_LARGE_DONE.load(core::sync::atomic::Ordering::SeqCst)
            && TEST_HARD_LINK_DONE.load(core::sync::atomic::Ordering::SeqCst)
            && TEST_SYMLINK_DONE.load(core::sync::atomic::Ordering::SeqCst)
            && TEST_TLB_DONE.load(core::sync::atomic::Ordering::SeqCst)
            && A::post_idle_ready()
            && kernel.ticks_on(0) >= TIMER_TICKS
        {
            let switches = kernel.stats().context_switches.saturating_sub(1);
            vifast_kernel::klog!(
                A,
                "PREEMPT OK (ticks={}, switches={})",
                kernel.ticks_on(0),
                switches
            );
            let (timer_wakes, ipi_wakes) = A::secondary_wakeups();
            let none_wakes = A::secondary_none_wakes();
            let wakeups = timer_wakes + ipi_wakes + none_wakes;
            if wakeups > 0 {
                vifast_kernel::klog!(
                    A,
                    "SMP IDLE OK (hart1_wakeups={}, idle_entries={}, wake_timer={}, wake_sgi={})",
                    wakeups,
                    A::secondary_idle_entries(),
                    timer_wakes,
                    ipi_wakes
                );
            }
            if let Some((ap, bsp)) = A::timer_rate() {
                vifast_kernel::klog!(A, "SMP TIMER RATE (ap={}, bsp={})", ap, bsp);
            }
            if let Some((sent, seen)) = A::ipi_count() {
                vifast_kernel::klog!(A, "SMP IPI COUNT (sent={}, seen={})", sent, seen);
            }
            let hart1_ticks = A::secondary_timer_ticks();
            if hart1_ticks > 0 {
                vifast_kernel::klog!(A, "SMP TIMER OK (hart1_ticks={})", hart1_ticks);
            }
            let user_ticks = A::secondary_user_ticks();
            let preempts = A::secondary_preempts();
            vifast_kernel::klog!(
                A,
                "SMP PREEMPT OK (user_ticks={}, preempts={})",
                user_ticks,
                preempts
            );
            if let Some((ticks, entries, timer_wakes)) = A::post_idle_stats() {
                vifast_kernel::klog!(
                    A,
                    "SMP POST IDLE OK (ticks={}, idle_entries={}, wake_timer={})",
                    ticks,
                    entries,
                    timer_wakes
                );
            }
            let entries1 = A::user_entries(1);
            if entries1 > 0 {
                vifast_kernel::klog!(
                    A,
                    "SMP SCHED OK (hart0={}, hart1={}, resched_sent={}, resched_wake={}, idle_entries={})",
                    A::user_entries(0),
                    entries1,
                    A::resched_sent(),
                    A::resched_wake(),
                    A::idle_entries()
                );
            }
            let bkl = A::bkl_stats();
            vifast_kernel::klog!(
                A,
                "BKL STATS (h0_acq={}, h0_spin={}, h1_acq={}, h1_spin={})",
                bkl[0].0,
                bkl[0].1,
                bkl[1].0,
                bkl[1].1
            );
            if let Some((va_flush, ipi, skipped)) = A::tlb_stats() {
                vifast_kernel::klog!(
                    A,
                    "TLB STATS (va_flush={}, ipi={}, skipped={})",
                    va_flush,
                    ipi,
                    skipped
                );
            }
            vifast_kernel::klog!(A, "ROOT SWITCH {}", kernel.stats().root_switches);
            if let Some((size, bump, objects, max_objects)) =
                kernel.untyped_usage_by_base(A::data_page().as_ptr() as usize)
            {
                let page = A::PAGE_SIZE;
                vifast_kernel::klog!(
                    A,
                    "DATA UNTYPED pages={}/{} objects={}/{}",
                    bump / page,
                    size / page,
                    objects,
                    max_objects
                );
            }
            A::exit(0);
        }
        A::reload_timer();
        A::ack_timer();
        if old.is_some() {
            let _ = kernel.yield_current_on(hart);
        }
        let Some(next) = kernel.pick_on(hart) else {
            A::exit(1);
        };
        note_current_tcb::<A>(kernel);
        *context = contexts[next.index as usize];
        crate::syscall::apply_resume::<A>(context, next);
        if ensure_current_root::<A>().is_err() {
            fault::suspend::<A>(context, 0, 0);
        }
        old_index != Some(next.index)
    }
}
