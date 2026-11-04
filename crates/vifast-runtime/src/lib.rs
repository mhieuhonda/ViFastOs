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
use vifast_kernel::{Cap, Kernel, UserAction};

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

/// A user image is copied into the port's executable user page.
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
    fn set_kernel(kernel: *mut Kernel);
    fn kernel() -> &'static mut Kernel;

    fn code_page() -> &'static mut [u8];
    fn data_page() -> &'static mut [u8];
    fn metadata_page() -> &'static mut [u8];
    fn build_page_tables();
    fn vspace_root() -> u64;

    fn zero_frame(pa: u64, size: usize) -> Result<(), KError>;
    fn map_user_page(va: u64, pa: u64, rights: Rights) -> Result<(), KError>;
    fn unmap_user_page(va: u64) -> Result<(), KError>;
    fn unmap_user_page_at(root: u64, va: u64) -> Result<(), KError> {
        if root == Self::vspace_root() {
            Self::unmap_user_page(va)
        } else {
            Err(KError::InvalidArg)
        }
    }
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

    let mut selftest_kernel = Kernel::new();
    if !vifast_kernel::selftest::run::<A>(&mut selftest_kernel) {
        vifast_kernel::klog!(A, "SELFTEST FAIL");
        A::exit(1);
    }
    let stats = selftest_kernel.stats();
    drop(selftest_kernel);
    let mut kernel = Kernel::new();
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
    let stack_base = (A::stack_a_top() - 0x4000) as u64;
    let stack_len = (A::stack_b_top() - (A::stack_a_top() - 0x4000)) as u64;
    let Some(vspace) = kernel.create_user_vspace(
        A::vspace_root(),
        A::USER_DATA_VA as u64,
        A::PAGE_SIZE as u64,
        A::USER_CODE_VA as u64,
        A::PAGE_SIZE as u64,
        stack_base,
        stack_len,
    ) else {
        A::exit(1);
    };
    if kernel.set_tcb_vspace(root, vspace).is_err() {
        A::exit(1);
    }
    /* Runtime-owned objects: root TCB, VSpace, metadata Untyped, data Untyped. */
    const RUNTIME_OBJECTS: usize = 4;
    const META_MAX_OBJECTS: usize = 16;
    const DATA_MAX_OBJECTS: usize = 1;
    if RUNTIME_OBJECTS + META_MAX_OBJECTS + DATA_MAX_OBJECTS > vifast_kernel::MAX_OBJECTS {
        vifast_kernel::klog!(A, "UNTYPED LIMIT EXCEEDS OBJECT TABLE");
        A::exit(1);
    }
    let data = A::data_page();
    let metadata = A::metadata_page();
    let Some(meta_untyped) = kernel.create_untyped_with_limit(
        metadata.as_mut_ptr() as usize,
        metadata.len(),
        A::PAGE_SIZE,
        META_MAX_OBJECTS,
    ) else {
        A::exit(1);
    };
    let Some(data_untyped) =
        kernel.create_untyped_with_limit(data.as_mut_ptr() as usize, data.len(), A::PAGE_SIZE, 1)
    else {
        A::exit(1);
    };
    if kernel
        .install_cap(
            root,
            1,
            Cap::new(vspace, ObjType::VSpace, Rights::READ | Rights::WRITE),
        )
        .is_err()
        || kernel
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

    let code = A::code_page();
    if image.code.len() > code.len() {
        vifast_kernel::klog!(A, "USER IMAGE TOO LARGE");
        A::exit(1);
    }
    code[..image.code.len()].copy_from_slice(image.code);
    A::build_page_tables();
    A::prepare_user();

    let contexts = A::contexts();
    contexts[root.index as usize] =
        A::context_for_user(A::user_code_base() + image.entry_root, A::stack_a_top());

    A::set_kernel(&mut kernel as *mut Kernel);
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
            UserAction::MapFrame {
                vspace,
                root,
                va,
                pa,
                rights,
            } => {
                let mapped = root == A::vspace_root() && A::map_user_page(va, pa, rights).is_ok();
                if mapped {
                    A::write_result(context, KError::Ok, va, 0, [0; 4]);
                } else {
                    let _ = kernel.unmap_frame(vspace, va);
                    A::write_result(context, KError::MissingPageTable, 0, 0, [0; 4]);
                }
            }
            UserAction::UnmapFrame { root, va, .. } => {
                let ok = root == A::vspace_root() && A::unmap_user_page(va).is_ok();
                if ok {
                    A::write_result(context, KError::Ok, 0, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::NotMapped, 0, 0, [0; 4]);
                }
            }
            UserAction::FrameRetyped { dest, pa, size } => {
                let ok = A::zero_frame(pa, size as usize).is_ok();
                A::write_result(
                    context,
                    if ok {
                        KError::Ok
                    } else {
                        KError::MissingPageTable
                    },
                    if ok { dest } else { 0 },
                    0,
                    [0; 4],
                );
            }
            UserAction::Revoked { unmaps, count } => {
                let mut ok = true;
                for unmap in unmaps.iter().take(count).flatten() {
                    if A::unmap_user_page_at(unmap.root, unmap.va).is_err() {
                        ok = false;
                    }
                }
                A::write_result(
                    context,
                    if ok { KError::Ok } else { KError::NotMapped },
                    0,
                    0,
                    [0; 4],
                );
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
