//! Reusable boot, syscall-dispatch and timer runtime for all ViFastOs ports.
//!
//! This crate contains no architecture instructions and no MMIO.  A concrete
//! port implements [`Platform`] in its `vifast-arch-*` crate; the kernel binary
//! only supplies the boot entry, linker script and user image.

#![no_std]
#![forbid(unsafe_code)]

use vifast_abi::{KError, Message, ObjType, Rights};
use vifast_arch::Arch;
use vifast_kernel::console::Console;
use vifast_kernel::fs::MAX_PATH;
use vifast_kernel::{Cap, Kernel, TaskLayout, UserAction};

#[cfg(feature = "test-exit")]
pub const TIMER_TICKS: u64 = 20;
pub const USER_COPY_MAX: usize = 256;

/// Boot information supplied by the architecture entry point.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlatformInfo {
    pub hartid: usize,
    pub dtb: u64,
    pub system_table: usize,
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
    /// Offset of the root task entry.
    pub entry_root: usize,
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

    fn user_code_base() -> usize {
        Self::USER_CODE_VA
    }
    fn stack_a_top() -> usize;
    fn stack_b_top() -> usize;

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
    /// Select the root loaded by the next `enter_user`.  This remains active
    /// across traps until a later context switch replaces it.
    fn set_active_root(root_pa: u64);

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
    fn contexts() -> &'static mut [Self::Context];
    fn enter_user(context: &Self::Context) -> !;

    fn write_result(
        context: &mut Self::Context,
        error: KError,
        value: u64,
        label: u64,
        data: [u64; 4],
    );

    fn write_message(context: &mut Self::Context, message: &Message) {
        let mut data = [0u64; 4];
        let words = message.words();
        let mut i = 0;
        while i < data.len() && i < words.len() {
            data[i] = words[i];
            i += 1;
        }
        Self::write_result(context, KError::Ok, 0, message.label, data);
    }

    fn prepare_user() {}

    fn start_timer();
    fn reload_timer();
    fn ack_timer();
    fn start_smp();
}

/// Shared boot sequence: kernel self-test, capability setup, user image copy
/// and the first user context.
pub fn boot<A: Platform>(info: PlatformInfo, image: UserImage) -> ! {
    A::init();
    A::irq_disable();
    A::init_platform(info);
    vifast_kernel::klog!(
        A,
        "ViFastOS booting on {} (hart={}, dtb={:#x})",
        <A as Console>::NAME,
        info.hartid,
        info.dtb
    );

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
    /* Runtime-owned objects: root TCB, VSpace, metadata Untyped, data Untyped. */
    const RUNTIME_OBJECTS: usize = 5;
    const META_MAX_OBJECTS: usize = 32;
    const DATA_MAX_OBJECTS: usize = 48;
    const BOOT_MAX_OBJECTS: usize = 32;
    /* 4.5 needs a private server VSpace: two root pages, two page tables and
    one or two stack pages.  Keep enough in the boot arena for that. */
    const SERVER_MIN_PAGES: usize = 6;
    const SERVER_MIN_OBJECTS: usize = 6;
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
            2,
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
                3,
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

    let page_size = A::PAGE_SIZE as u64;
    let layout = TaskLayout {
        code_va: vifast_abi::user_layout::CODE_VA as u64,
        stack_va: vifast_abi::user_layout::STACK_BASE as u64,
        stack_pages: vifast_abi::user_layout::STACK_LEN / A::PAGE_SIZE,
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
    if kernel.set_tcb_vspace(root, vspace).is_err()
        || kernel
            .install_cap(
                root,
                1,
                Cap::new(vspace, ObjType::VSpace, Rights::READ | Rights::WRITE),
            )
            .is_err()
    {
        A::exit(1);
    }
    if !A::check_kernel_mappings(root_pa) {
        vifast_kernel::klog!(A, "KERNEL MAPPING CHECK FAILED root={:#x}", root_pa);
        A::exit(1);
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
        if free_pages < SERVER_MIN_PAGES || free_objects < SERVER_MIN_OBJECTS {
            vifast_kernel::klog!(
                A,
                "BOOT UNTYPED TOO SMALL FOR SERVER VSPACE (need pages={} objects={})",
                SERVER_MIN_PAGES,
                SERVER_MIN_OBJECTS
            );
            A::exit(1);
        }
    } else {
        A::exit(1);
    }
    A::set_active_root(root_pa);
    A::prepare_user();

    let contexts = A::contexts();
    contexts[root.index as usize] =
        A::context_for_user(A::user_code_base() + image.entry_root, A::stack_a_top());

    A::set_kernel(kernel as *mut Kernel);
    kernel.enqueue(root);
    let _ = kernel.pick();
    A::start_timer();
    vifast_kernel::klog!(A, "entering user mode at {:#x}", A::user_code_base());
    let first = contexts[root.index as usize];
    A::enter_user(&first)
}

/// Syscall/action dispatch shared by all trap entry implementations.
pub mod syscall {
    use super::*;

    pub fn dispatch<A: Platform>(context: &mut A::Context, number: u64, args: [u64; 6]) {
        let kernel = A::kernel();
        let Some(tcb) = kernel.current() else {
            A::exit(1);
        };
        let action = kernel.user_syscall(
            tcb, number, args[0], args[1], args[2], args[3], args[4], args[5],
        );
        match action {
            UserAction::Return { value, error } => {
                A::write_result(context, error, value, 0, [0; 4]);
            }
            UserAction::Message {
                error,
                value,
                label,
                data,
            } => A::write_result(context, error, value, label, data),
            UserAction::ConfigureTcb { tcb, entry, stack } => {
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
            UserAction::Debug { ptr, len } => {
                let len = (len as usize).min(USER_COPY_MAX);
                let mut buf = [0u8; USER_COPY_MAX];
                if A::read_user(ptr, &mut buf[..len]).is_err() {
                    A::write_result(context, KError::InvalidArg, 0, 0, [0; 4]);
                } else {
                    for byte in &buf[..len] {
                        A::putc(*byte);
                    }
                    A::write_result(context, KError::Ok, 0, 0, [0; 4]);
                }
            }
            UserAction::Blocked => switch_blocked::<A>(context),
            UserAction::Exit => {
                A::write_str("\r\nUSER EXIT\r\n");
                A::exit(0);
            }
        }
    }

    pub fn apply_resume<A: Platform>(context: &mut A::Context, tcb: vifast_kernel::ObjRef) {
        let kernel = A::kernel();
        if let Some(error) = kernel.take_resume_error(tcb) {
            A::write_result(context, error, 0, 0, [0; 4]);
        } else if let Some(message) = kernel.take_resume(tcb) {
            A::write_message(context, &message);
        }
    }

    pub fn switch_blocked<A: Platform>(context: &mut A::Context) {
        let kernel = A::kernel();
        let contexts = A::contexts();
        if let Some(old) = kernel.current() {
            contexts[old.index as usize] = *context;
        }
        let Some(next) = kernel.pick() else {
            A::exit(1);
        };
        *context = contexts[next.index as usize];
        crate::syscall::apply_resume::<A>(context, next);
    }
}

/// Minimal user-fault suspension.  A full fault endpoint is future work;
/// for now the faulting TCB becomes Faulted and another TCB continues.
pub mod fault {
    use super::*;

    pub fn suspend<A: Platform>(context: &mut A::Context, cause: u64, addr: u64) {
        let kernel = A::kernel();
        let old = kernel.current();
        vifast_kernel::klog!(
            A,
            "USER FAULT tcb={:?} cause={:#x} addr={:#x}",
            old,
            cause,
            addr
        );
        let Some(next) = kernel.fault_current() else {
            vifast_kernel::klog!(A, "no runnable thread after fault");
            A::halt();
        };
        *context = A::contexts()[next.index as usize];
        crate::syscall::apply_resume::<A>(context, next);
    }
}

/// Timer tick, accounting and preemptive context switch.
pub mod timer {
    use super::*;

    pub fn on_timer<A: Platform>(context: &mut A::Context) {
        let kernel = A::kernel();
        let contexts = A::contexts();
        let old = kernel.current();
        if let Some(old) = old {
            contexts[old.index as usize] = *context;
        }
        kernel.tick();
        #[cfg(feature = "test-exit")]
        if kernel.ticks() >= TIMER_TICKS {
            let switches = kernel.stats().context_switches.saturating_sub(1);
            vifast_kernel::klog!(
                A,
                "PREEMPT OK (ticks={}, switches={})",
                kernel.ticks(),
                switches
            );
            A::exit(0);
        }
        A::reload_timer();
        A::ack_timer();
        if old.is_some() {
            let _ = kernel.yield_current();
        }
        let Some(next) = kernel.pick() else {
            A::exit(1);
        };
        *context = contexts[next.index as usize];
        crate::syscall::apply_resume::<A>(context, next);
    }
}
