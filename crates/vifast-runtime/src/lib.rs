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

#[cfg(all(feature = "test-exit", feature = "test-shell-linger"))]
pub const TIMER_TICKS: u64 = 2000;
#[cfg(all(feature = "test-exit", not(feature = "test-shell-linger")))]
pub const TIMER_TICKS: u64 = 20;
#[cfg(feature = "test-exit")]
static TEST_READY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
static TEST_DONE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "test-exit")]
/// Vòng 151: chỉ in chẩn đoán SMP IDLE STALL một lần.
static POST_IDLE_STALLED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
/// T33a (r288): đã in chẩn đoán bộ cờ `SMP WAIT` chưa (chỉ in một lần).
#[cfg(feature = "test-exit")]
static WAIT_DIAG: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
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
pub const USER_COPY_MAX: usize = vifast_abi::USER_COPY_MAX;

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
    /// Số tick timer mỗi giây (nhịp lập lịch). Mặc định 100 Hz — cả ba port
    /// hiện cấu hình đúng nhịp này (riscv: mtime 10 MHz / 100_000; aarch64:
    /// `TIMER_HZ`; x86_64: LAPIC initial = hz/100). Port nào đổi nhịp phải
    /// override.
    fn tick_hz() -> u32 {
        100
    }
    /// Vòng 160: port này có an toàn để thu hồi trang staging module không.
    /// Mặc định TẮT: chỉ bật khi đã chứng minh trên phần cứng/emulator đó.
    fn staging_release_safe() -> bool {
        false
    }
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
    /// D15 (r246): set the second integer argument (`a1`/`x1`/`rsi`) — the
    /// `argv` pointer of an exec'd program.
    fn set_context_arg1(context: &mut Self::Context, value: u64);
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

    /// `true` khi `Context` mà `fault::suspend` nhận là frame user đầy đủ
    /// (aarch64/riscv64 truyền frame trap thật). x86_64 hiện truyền frame
    /// rỗng vì trap.S đẩy vector/error trước SAVE_REGS, nên không được lưu nó
    /// vào slot (sẽ làm task chạy lại từ frame 0 → GP).
    fn fault_frame_complete() -> bool {
        true
    }

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
    /// Boot-stack high-water mark from the 0xA5 paint, `(used, size)`.
    /// r229 (T4): canary vùng đệm sau `KERNEL_STORAGE`.
    /// `Some(true)` = còn nguyên, `Some(false)` = đã bị ghi đè.
    fn kernel_storage_canary() -> Option<bool> {
        None
    }

    fn boot_stack_usage() -> Option<(usize, usize)> {
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

    /// T33 (r275): trạng thái hart phụ tại lúc chẩn đoán `SMP IDLE STALL`.
    /// Chỉ dùng cho CHẨN ĐOÁN (không phải marker của gate); mặc định `None`
    /// để các nền tảng khác không phải đổi gì.
    ///
    /// Thứ tự: online, idle_window, idle_generation, wake_last_generation,
    /// ipi_sent, resched_sent.
    fn stall_detail() -> Option<[usize; 6]> {
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
    fn bkl_stats() -> [(usize, usize); vifast_arch::MAX_HARTS] {
        [(0, 0); vifast_arch::MAX_HARTS]
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

/// D19 (r259): bất biến sched kiểm ở RANH GIỚI trả về user.
///
/// Hợp đồng "Running ⇔ là `current`" chỉ đúng ở ranh giới: bên trong một
/// lời gọi syscall, TCB hiện tại có thể vừa bị chặn vừa còn trong trạng thái
/// Running (chờ đối tác), nên kiểm ở đó sẽ báo sai. Vì thế phép kiểm nằm
/// ngay sau khi hạt nhân đã chọn và cài TCB kế tiếp, trước khi trả về user.
///
/// Bản biến thể `check-boundary-mutation` cố ý làm hỏng bất biến đúng MỘT
/// lần (D18) để chứng minh phép kiểm không im lặng.
#[cfg(feature = "check-boundary")]
pub mod boundary {
    use super::*;
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    static CHECKS: AtomicU64 = AtomicU64::new(0);
    static VIOLATIONS: AtomicU64 = AtomicU64::new(0);
    static POISONED: AtomicBool = AtomicBool::new(false);

    /// Số ranh giới đã kiểm.
    pub fn checks() -> u64 {
        CHECKS.load(Ordering::Relaxed)
    }

    /// Số lần bất biến bị vi phạm.
    pub fn violations() -> u64 {
        VIOLATIONS.load(Ordering::Relaxed)
    }

    /// Kiểm ngay sau khi TCB kế tiếp đã được cài vào `current[hart]`.
    pub fn check<A: Platform>() {
        let hart = A::hart_id();
        #[cfg(feature = "check-boundary-mutation")]
        {
            if !POISONED.swap(true, Ordering::SeqCst) {
                let _ = A::kernel().poison_running_queued(hart);
            }
        }
        let result = A::kernel().check_boundary(hart);
        CHECKS.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "check-boundary-mutation")]
        {
            // Trả lại trạng thái sạch để chỉ có ĐÚNG MỘT marker BAD.
            let _ = A::kernel().clear_running_queued(hart);
        }
        if let Err(error) = result {
            VIOLATIONS.fetch_add(1, Ordering::Relaxed);
            let kind = match error {
                vifast_kernel::BoundaryError::NoCurrent => 0,
                vifast_kernel::BoundaryError::CurrentNotRunning => 1,
                vifast_kernel::BoundaryError::RunningAndQueued => 2,
            };
            vifast_kernel::klog!(A, "BOUNDARY BAD (hart={}, kind={})", hart, kind);
        }
    }
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

/// r300 (T-IPC): số syscall ĐÃ VÀO KERNEL của từng tcb. Đèn `NEXT` là one-shot
/// nên không trả lời được "client còn chạy không"; bộ đếm này thì trả lời được:
/// in ở cuối boot, nếu con số không tăng nữa ⇒ client đã ngừng vào kernel.
static SYSCALL_COUNT: [core::sync::atomic::AtomicU32; 128] =
    [const { core::sync::atomic::AtomicU32::new(0) }; 128];

/// r301 (T-IPC): chẩn đoán in MỘT lần ở cuối boot, gói trong một hàm để không
/// phải chèn khối `{}` giữa điều kiện `if let` và thân của nó (đã làm hỏng cú
/// pháp một lần — bài học: đèn nhiều dòng thì phải nằm trong hàm).
fn emit_boot_diag<A: Platform>(kernel: &vifast_kernel::Kernel) {
    let count = SYSCALL_COUNT[62].load(core::sync::atomic::Ordering::Relaxed);
    vifast_kernel::klog!(A, "SYSCALLS t=62 n={}", count);
    // r302: số lần tcb client ĐƯỢC CHỌN CHẠY. PICKS tăng mà SYSCALLS đứng ⇒
    // client vẫn chạy nhưng quay trong user mode; PICKS đứng ⇒ không còn được
    // chọn (rơi hàng đợi hoặc đang Blocked).
    let picks = vifast_kernel::PICKS[62].load(core::sync::atomic::Ordering::Relaxed);
    vifast_kernel::klog!(A, "PICKS t=62 n={}", picks);
    // r302: trạng thái của client lúc kết thúc boot — "Blocked" nào, còn `queued`
    // hay không. Đây là thứ phân biệt BlockedReply với rơi-khỏi-hàng-đợi.
    match kernel.tcb_state_at(62) {
        Some((state, queued)) => {
            vifast_kernel::klog!(A, "STATE t=62 s={} q={}", state as u8, queued as u8)
        }
        None => vifast_kernel::klog!(A, "STATE t=62 GONE"),
    }
    let len = vifast_kernel::KILLED_LEN.load(core::sync::atomic::Ordering::Relaxed);
    let mut line = [0u8; 96];
    let mut at = 0usize;
    for &byte in b"KILLED" {
        line[at] = byte;
        at += 1;
    }
    for slot in 0..(len as usize).min(16) {
        line[at] = b' ';
        at += 1;
        let value = vifast_kernel::KILLED[slot].load(core::sync::atomic::Ordering::Relaxed);
        if value != 0 {
            dec_into(&mut line, &mut at, (value - 1) as u64);
        }
    }
    for byte in line.iter().take(at) {
        A::putc(*byte);
    }
    A::putc(b'\n');
}

/// r300 (T-IPC): in `DW <loại> t=<tcb> v=<số>` — đèn phía KERNEL cho `DebugWrite`,
/// để phân biệt "kernel từ chối / không đọc được buffer user" với "đã đẩy byte
/// mà byte không ra cổng". Client dùng `let _ = debug_write(...)` nên mọi lỗi ở
/// đây đều IM LẶNG phía user.
fn emit_dw_error<A: Platform>(tag: &[u8], tcb: u64, value: u64) {
    let mut line = [0u8; 40];
    let mut at = 0usize;
    for &byte in tag {
        if at < line.len() {
            line[at] = byte;
            at += 1;
        }
    }
    dec_into(&mut line, &mut at, tcb);
    for &byte in b" v=" {
        if at < line.len() {
            line[at] = byte;
            at += 1;
        }
    }
    dec_into(&mut line, &mut at, value);
    if at < line.len() {
        line[at] = b'\n';
        at += 1;
    }
    for byte in line.iter().take(at) {
        A::putc(*byte);
    }
}

/// r298: ghi số thập phân vào `line` tại `at` (không cấp phát, không panic).
fn dec_into(line: &mut [u8], at: &mut usize, value: u64) {
    let mut digits = [0u8; 20];
    let mut count = 0usize;
    let mut rest = value;
    loop {
        digits[count] = b'0' + (rest % 10) as u8;
        rest /= 10;
        count += 1;
        if rest == 0 {
            break;
        }
    }
    while count > 0 {
        count -= 1;
        if *at < line.len() {
            line[*at] = digits[count];
            *at += 1;
        }
    }
}

fn note_current_tcb<A: Platform>(kernel: &Kernel) {
    let hart = A::hart_id();
    A::set_current_tcb(kernel.current_on(hart).map(|tcb| tcb.index as usize));
}

/// r295 (bước 0): id của lần build, do `scripts/build.sh` truyền qua biến môi
/// trường cho MỌI crate. `unknown` chỉ xuất hiện khi ai đó build bằng tay
/// (không qua build.sh) — khi đó harness báo `T-BUILD` vì id không khớp.
pub const BUILD_ID: &str = match option_env!("VIFAST_BUILD_ID") {
    Some(id) => id,
    None => "unknown",
};

/// Bộ feature của thành phần này (chỉ để chứng minh biến thể, không dùng để
/// so khớp giữa các thành phần).
pub const BUILD_FEATS: &str = match option_env!("VIFAST_BUILD_FEATS") {
    Some(feats) => feats,
    None => "none",
};

/// Shared boot sequence: kernel self-test, capability setup, user image copy
/// and the first user context.
pub fn boot<A: Platform>(info: PlatformInfo, image: UserImage) -> ! {
    A::init();
    A::irq_disable();
    // 294.C: mốc sớm NHẤT mà kernel chắc chắn in được — dùng để phân biệt
    // T-BOOT (chết trước khi kernel chạy) với các lớp khác. In trước cả
    // `init_platform` để nếu platform chết thì vẫn biết kernel đã vào.
    vifast_kernel::klog!(A, "KERNEL ENTRY");
    // r295 (bước 0): dòng BUILD — chứng minh lần đo này chạy ĐÚNG binary vừa
    // build từ trạng thái nguồn hiện tại, không phải ảnh cũ còn sót.
    vifast_kernel::klog!(A, "BUILD kernel={} feats={}", BUILD_ID, BUILD_FEATS);
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
    kernel.set_timer_hz(A::tick_hz());
    // Guard against static-storage overflow: `Kernel` lives in a platform
    // `.data`/`.bss` static. If it grows past its neighbors (e.g. a pool
    // array added to `Kernel`), `reset()` silently clobbers them — on AArch64
    // this zeroes KERNEL_PTR/BKL and faults after BOOTINFO (vòng 30).
    // Budget is defined once in `vifast_kernel::KERNEL_STATIC_BUDGET`.
    if core::mem::size_of::<Kernel>() > vifast_kernel::KERNEL_STATIC_BUDGET {
        vifast_kernel::klog!(
            A,
            "KERNEL STATIC OVERFLOW size={} budget={}",
            core::mem::size_of::<Kernel>(),
            vifast_kernel::KERNEL_STATIC_BUDGET
        );
        A::exit(1);
    }
    // Publish the kernel pointer before any secondary hart is started: the
    // AP boot path calls `Platform::kernel()` as soon as PSCI/HSM/SIPI brings
    // it online.
    A::set_kernel(kernel as *mut Kernel);
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
    const DATA_MAX_OBJECTS: usize = 86;
    const BOOT_MAX_OBJECTS: usize = 40;
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

    // Mốc 9 (vòng 46): self-TCB cap để root tự set IPC buffer + tự gọi VFS
    // (STAT/READ file exec). Không GRANT để root không phát tán quyền
    // điều khiển chính nó.
    if kernel
        .install_cap(
            root,
            vifast_abi::user_layout::ROOT_SELF_SLOT as u8,
            Cap::new(root, ObjType::Tcb, Rights::READ | Rights::WRITE),
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
            let page_va = base + (page * A::PAGE_SIZE) as u64;
            if kernel
                .map_frame(vspace, frame, page_va, Rights::READ)
                .is_err()
            {
                vifast_kernel::klog!(A, "BOOT MODULE MAP FAILED");
                A::exit(1);
            }
            if A::staging_release_safe() {
                kernel.register_staging(vspace, page_va, frame);
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
        sched_harts & !0b1111 == 0,
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
    let root_slot = A::kernel().context_slot(root).unwrap_or(0) as usize;
    contexts[root_slot] = root_context;

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
    #[cfg(feature = "diag-sched")]
    vifast_kernel::klog!(A, "RING ADDR {:#x}", crate::diag::address());
    A::start_timer();
    #[cfg(feature = "test-exit")]
    TEST_EXPECT_WORKERS.store(
        A::sched_harts() & 0b10 != 0,
        core::sync::atomic::Ordering::SeqCst,
    );
    vifast_kernel::klog!(A, "entering user mode at {:#x}", A::user_code_base());
    let root_slot = A::kernel().context_slot(root).unwrap_or(0) as usize;
    let first = contexts[root_slot];
    A::enter_user(&first)
}

/// Syscall/action dispatch shared by all trap entry implementations.
/// r294.B: ghi sự kiện PICK vào vòng đệm (không in — xem `diag`).
#[cfg(feature = "diag-sched")]
pub fn note_context<A: Platform>(
    kernel: &vifast_kernel::Kernel,
    next: vifast_kernel::ObjRef,
    hart: usize,
) {
    let slot = kernel
        .context_slot(next)
        .map(usize::from)
        .unwrap_or(usize::MAX);
    let state = kernel
        .tcb(next)
        .map(|table| table.state as u64)
        .unwrap_or(255);
    diag::record(
        diag::PICK,
        hart,
        next.index as usize,
        slot,
        state,
        kernel.fast_time(),
    );
}

#[cfg(not(feature = "diag-sched"))]
#[inline]
pub fn note_context<A: Platform>(
    _kernel: &vifast_kernel::Kernel,
    _next: vifast_kernel::ObjRef,
    _hart: usize,
) {
}

pub mod diag {
    use super::*;
    use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    pub const PICK: u64 = 1;
    pub const CTX_SAVE: u64 = 2;
    pub const CTX_LOAD: u64 = 3;
    pub const SUSPEND: u64 = 4;
    pub const RESUME: u64 = 5;

    const RING: usize = 96;
    const EMPTY: u64 = u64::MAX;
    static EVENTS: [AtomicU64; RING] = [const { AtomicU64::new(EMPTY) }; RING];
    static INDEX: AtomicUsize = AtomicUsize::new(0);
    static RECORDED: AtomicUsize = AtomicUsize::new(0);

    /// Đóng gói một sự kiện vào 64 bit (không cấp phát, không khoá).
    pub fn pack(kind: u64, hart: usize, tcb: usize, slot: usize, state: u64, tick: u64) -> u64 {
        (kind & 0xf)
            | ((hart as u64 & 0x3) << 4)
            | ((tcb as u64 & 0xff) << 6)
            | ((slot as u64 & 0xff) << 14)
            | ((state & 0xf) << 22)
            | ((tick & 0xffff_ffff) << 26)
    }

    pub fn unpack(raw: u64) -> (u64, usize, usize, usize, u64, u64) {
        (
            raw & 0xf,
            ((raw >> 4) & 0x3) as usize,
            ((raw >> 6) & 0xff) as usize,
            ((raw >> 14) & 0xff) as usize,
            (raw >> 22) & 0xf,
            (raw >> 26) & 0xffff_ffff,
        )
    }

    pub fn record(kind: u64, hart: usize, tcb: usize, slot: usize, state: u64, tick: u64) {
        let position = INDEX.fetch_add(1, Ordering::Relaxed) % RING;
        EVENTS[position].store(pack(kind, hart, tcb, slot, state, tick), Ordering::Release);
        RECORDED.fetch_add(1, Ordering::Relaxed);
    }

    /// Địa chỉ (VA) của vòng đệm — để harness đọc bằng monitor QEMU khi CẢ HAI
    /// hart đều không chạy được nữa (trường hợp `PIC STALL`/treo câm: hart 0
    /// quay user với IRQ0 đang chờ, hart 1 `hlt` với IF=0 ⇒ không hook nào chạy).
    pub fn address() -> u64 {
        &EVENTS as *const _ as u64
    }

    pub fn capacity() -> usize {
        RING
    }

    /// Đổ vòng đệm ra serial (đường in của kernel, KHÔNG qua syscall).
    pub fn flush<A: Platform>(why: &str) {
        let recorded = RECORDED.swap(0, Ordering::AcqRel);
        if recorded == 0 {
            return;
        }
        let total = INDEX.load(Ordering::Acquire);
        let count = recorded.min(RING);
        vifast_kernel::klog!(A, "RING DUMP ({} sự kiện, vì {})", count, why);
        for offset in 0..count {
            let position = (total + RING - count + offset) % RING;
            let raw = EVENTS[position].swap(EMPTY, Ordering::AcqRel);
            if raw == EMPTY {
                continue;
            }
            let (kind, hart, tcb, slot, state, tick) = unpack(raw);
            vifast_kernel::klog!(
                A,
                "RING kind={} hart={} tcb={} slot={} state={} tick={}",
                kind,
                hart,
                tcb,
                slot,
                state,
                tick
            );
        }
    }
}

/// D1.a (kế hoạch qua đêm, khối D): dựng marker `SMP NO CURRENT` thành HÀM
/// THUẦN để test host được (không cần fake Platform). Trả về số byte đã ghi.
pub fn no_current_marker(hart: u32, number: u64, out: &mut [u8; 40]) -> usize {
    let mut at = 0usize;
    for byte in b"SMP NO CURRENT hart=" {
        if at < out.len() {
            out[at] = *byte;
            at += 1;
        }
    }
    at = write_decimal_into(hart as u64, out, at);
    for byte in b" nr=" {
        if at < out.len() {
            out[at] = *byte;
            at += 1;
        }
    }
    write_decimal_into(number, out, at)
}

fn write_decimal_into(mut value: u64, out: &mut [u8; 40], mut at: usize) -> usize {
    let mut digits = [0u8; 20];
    let mut count = 0usize;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        value /= 10;
        count += 1;
        if value == 0 {
            break;
        }
    }
    while count > 0 {
        count -= 1;
        if at < out.len() {
            out[at] = digits[count];
            at += 1;
        }
    }
    at
}

/// T31/bản phân tích: `context_slot(..)` trả `None` (TCB chưa có slot / đã bị
/// thu hồi) mà mã cũ lặng lẽ `unwrap_or(0)` — tức ghi/đọc NHẦM context của
/// slot 0 (thường là root). Nay báo MỘT lần rồi vẫn giữ hành vi cũ để không
/// đổi ngữ nghĩa trong đêm, nhưng có bằng chứng trên serial.
/// Phần THUẦN (test host được): `(slot dùng, có thiếu slot không)`.
/// `None` ⇒ (0, true): hành vi cũ là rơi về slot 0, nhưng nay phải BÁO.
pub fn slot_choice(slot: Option<u8>) -> (usize, bool) {
    match slot {
        Some(value) => (value as usize, false),
        None => (0, true),
    }
}

pub fn slot_or_report<A: Platform>(
    kernel: &vifast_kernel::Kernel,
    next: vifast_kernel::ObjRef,
    hart: usize,
) -> usize {
    let (slot, missing) = slot_choice(kernel.context_slot(next));
    if missing {
        {
            static REPORTED: core::sync::atomic::AtomicBool =
                core::sync::atomic::AtomicBool::new(false);
            if !REPORTED.swap(true, core::sync::atomic::Ordering::Relaxed) {
                vifast_kernel::klog!(
                    A,
                    "CTX SLOT NONE hart={} tcb={} (dùng slot 0)",
                    hart,
                    next.index
                );
            }
        }
    }
    slot
}

pub mod syscall {
    use super::*;

    pub fn dispatch<A: Platform>(context: &mut A::Context, number: u64, args: [u64; 6]) -> bool {
        let mut waiting = false;
        let kernel = A::kernel();
        let hart = A::hart_id();
        let Some(tcb) = kernel.current_on(hart) else {
            // D1.a: in marker bằng ĐƯỜNG IN CỦA KERNEL (không qua syscall) rồi
            // mới exit — để lần hỏng sau phân loại được là "mất current" chứ
            // không phải "hết việc".
            let mut buffer = [0u8; 40];
            let length = crate::no_current_marker(hart as u32, number, &mut buffer);
            for byte in buffer.iter().take(length) {
                A::putc(*byte);
            }
            A::putc(b'\n');
            A::exit(1);
        };
        // T-IPC: đèn do CHÍNH KERNEL in (bỏ qua một lớp syscall của user) để
        // tách "kẹt trong syscall" khỏi "treo đường ghi console" — hai cơ chế
        // này trước đây trùng chữ ký (bản rà soát chỉ ra).
        if number == SyscallId::CapMint as u64 {
            for byte in b"MINT IN\n" {
                A::putc(*byte);
            }
        }
        let action = kernel.user_syscall_on(
            tcb, hart, number, args[0], args[1], args[2], args[3], args[4], args[5],
        );
        if number == SyscallId::CapMint as u64 {
            let tag: &[u8] = match &action {
                vifast_kernel::UserAction::Return { error, .. } => {
                    if *error as u64 == 16 {
                        b"MINT OUT ilop\n"
                    } else {
                        b"MINT OUT other\n"
                    }
                }
                _ => b"MINT OUT act\n",
            };
            for byte in tag {
                A::putc(*byte);
            }
            // r298 (T-IPC): sau khi `CapMint` xử lý xong, client KHÔNG in
            // `IPC MINT RET` trong ~7,5 % boot. Đèn này trả lời: kết quả được
            // ghi vào context của CHÍNH tác vụ gọi hay của tác vụ khác (hart
            // chéo) — và in cả hart để biết syscall chạy trên hart nào.
            let slot = kernel
                .context_slot(tcb)
                .map(|slot| slot as usize)
                .unwrap_or(usize::MAX);
            let own = if slot == usize::MAX {
                false
            } else {
                let base = A::contexts().as_ptr() as usize;
                let expected = base + slot * core::mem::size_of::<A::Context>();
                core::ptr::addr_of!(*context) as usize == expected
            };
            let mut line = [0u8; 48];
            let mut at = 0usize;
            for &byte in b"MINT RES h=" {
                if at < line.len() {
                    line[at] = byte;
                    at += 1;
                }
            }
            dec_into(&mut line, &mut at, hart as u64);
            for &byte in b" t=" {
                if at < line.len() {
                    line[at] = byte;
                    at += 1;
                }
            }
            dec_into(&mut line, &mut at, tcb.index as u64);
            for &byte in b" s=" {
                if at < line.len() {
                    line[at] = byte;
                    at += 1;
                }
            }
            dec_into(
                &mut line,
                &mut at,
                if slot == usize::MAX { 99 } else { slot as u64 },
            );
            for &byte in b" own=" {
                if at < line.len() {
                    line[at] = byte;
                    at += 1;
                }
            }
            if at < line.len() {
                line[at] = if own { b'1' } else { b'0' };
                at += 1;
            }
            if at < line.len() {
                line[at] = b'\n';
                at += 1;
            }
            for byte in line.iter().take(at) {
                A::putc(*byte);
            }
        }
        SYSCALL_COUNT[(tcb.index as usize).min(127)]
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if number == SyscallId::DebugWrite as u64 {
            if let vifast_kernel::UserAction::Return { error, .. } = &action {
                if *error != vifast_abi::KError::Ok {
                    emit_dw_error::<A>(b"DW KERR t=", tcb.index as u64, *error as u64);
                }
            }
        }
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
            UserAction::ConfigureTcb {
                tcb,
                entry,
                stack,
                argc,
                argv,
            } => {
                let contexts = A::contexts();
                let index = kernel
                    .context_slot(tcb)
                    .map(|slot| slot as usize)
                    .unwrap_or(usize::MAX);
                if let Some(slot) = contexts.get_mut(index) {
                    let mut fresh = A::context_for_user(entry as usize, stack as usize);
                    A::set_context_arg0(&mut fresh, argc);
                    A::set_context_arg1(&mut fresh, argv);
                    *slot = fresh;
                    A::write_result(context, KError::Ok, 0, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::IllegalOperation, 0, 0, [0; 4]);
                }
            }
            UserAction::RestartTcb {
                tcb,
                entry,
                stack,
                argc,
                argv,
            } => {
                let contexts = A::contexts();
                let index = kernel
                    .context_slot(tcb)
                    .map(|slot| slot as usize)
                    .unwrap_or(usize::MAX);
                if let Some(slot) = contexts.get_mut(index) {
                    let mut fresh = A::context_for_user(entry as usize, stack as usize);
                    A::set_context_arg0(&mut fresh, argc);
                    A::set_context_arg1(&mut fresh, argv);
                    *slot = fresh;
                    A::write_result(context, KError::Ok, 0, 0, [0; 4]);
                } else {
                    A::write_result(context, KError::IllegalOperation, 0, 0, [0; 4]);
                }
            }
            UserAction::SetTcbArgs { tcb, argc, argv } => {
                let contexts = A::contexts();
                let index = kernel
                    .context_slot(tcb)
                    .map(|slot| slot as usize)
                    .unwrap_or(usize::MAX);
                if let Some(slot) = contexts.get_mut(index) {
                    A::set_context_arg0(slot, argc);
                    A::set_context_arg1(slot, argv);
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
                    // r300 (T-IPC): mất dòng của client không để lại dấu vết.
                    // Đây là một trong hai chỗ nuốt cả dòng IM LẶNG
                    // (client dùng `let _ = debug_write(...)`).
                    emit_dw_error::<A>(b"DW RERR t=", tcb.index as u64, 0);
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
                    // T-IPC/r294: khoá console cho CẢ KHỐI, không chỉ từng
                    // byte — nếu không, klog của hart khác chen vào giữa dòng
                    // marker của user và harness thấy "thiếu marker" oan.
                    vifast_kernel::console::with_tx_lock(|| {
                        for byte in &buf[..len] {
                            A::putc(*byte);
                        }
                    });
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
                        // T33a (r288): thức dậy mà vẫn không có task KHÔNG phải
                        // lỗi chí tử — trước đây chỗ này `exit(1)` làm hart phụ
                        // chết và boot treo (`PREEMPT OK` không bao giờ in).
                        // Chờ lại, có chặn trên để không treo vô hạn nếu sai thật.
                        let mut spurious = 0u32;
                        while !switch_blocked::<A>(context) {
                            spurious += 1;
                            if spurious > 64 {
                                A::exit(1);
                            }
                            A::wait_for_work(hart);
                        }
                    } else {
                        A::exit(1);
                    }
                }
            }
            UserAction::Exit => {
                // Phiếu soát §1.1: `vifast_user::exit()` luôn gửi mã 0 nên
                // "exit mã khác 0" KHÔNG BAO GIỜ đúng — phải đổ vòng đệm ngay
                // khi XỬ LÝ Exit, nếu không mọi ca panic→exit sẽ mất bằng chứng.
                #[cfg(feature = "diag-sched")]
                crate::diag::flush::<A>("UserAction::Exit");
                // Mốc 1: exit chỉ kết thúc TASK gọi, không kill cả máy.
                // Task rời scheduler thành zombie (Exited); hart chọn task
                // khác chạy tiếp. Chỉ halt khi không còn task nào.
                let kernel = A::kernel();
                let hart = A::hart_id();
                if let Some(old) = kernel.current_on(hart) {
                    if let Some(slot) = kernel.context_slot(old) {
                        A::contexts()[slot as usize] = *context;
                    }
                }
                if kernel.exit_current_on(hart).is_some() {
                    // Có task kế tiếp: nạp context của nó như switch_blocked.
                    let next = kernel.current_on(hart).unwrap();
                    note_current_tcb::<A>(kernel);
                    crate::note_context::<A>(kernel, next, hart);
                    let slot = slot_or_report::<A>(kernel, next, hart);
                    *context = A::contexts()[slot];
                    crate::syscall::apply_resume::<A>(context, next);
                    A::contexts()[slot] = *context;
                    if ensure_current_root::<A>().is_err() {
                        fault::suspend::<A>(context, 0, 0);
                    }
                    return waiting;
                }
                // T33b (r288): hart này hết task KHÔNG có nghĩa cả máy hết
                // việc. Trước đây rơi thẳng xuống `USER EXIT` ⇒ một worker
                // thoát trên hart phụ giết cả hệ thống (đo được: guest in
                // `USER EXIT` ngay sau `TCB REAP OK`, boot dừng giữa đường,
                // tần suất ~1–2% trên cả riscv64 và x86_64). Chỉ kết thúc khi
                // thật sự không còn TCB `Ready`/`Running` nào.
                let others_busy = (0..vifast_kernel::MAX_HARTS)
                    .any(|other| other != hart && kernel.current_on(other).is_some());
                if (others_busy || kernel.has_any_ready()) && A::can_wait_for_work() {
                    waiting = true;
                } else {
                    A::write_str("\r\nUSER EXIT\r\n");
                    A::exit(0);
                }
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
            let slot = kernel.context_slot(old).unwrap_or(0) as usize;
            contexts[slot] = *context;
        }
        let Some(next) = kernel.pick_on(hart) else {
            return false;
        };
        note_current_tcb::<A>(kernel);
        crate::note_context::<A>(kernel, next, hart);
        let slot = slot_or_report::<A>(kernel, next, hart);
        *context = contexts[slot];
        #[cfg(feature = "diag-sched")]
        crate::diag::record(
            crate::diag::CTX_LOAD,
            hart,
            next.index as usize,
            slot,
            kernel
                .tcb(next)
                .map(|table| table.state as u64)
                .unwrap_or(255),
            kernel.fast_time(),
        );
        crate::syscall::apply_resume::<A>(context, next);
        // Ghi LẠI slot: `apply_resume` vá context SỐNG, nếu không ghi lại thì
        // lần nạp sau sẽ chạy lại snapshot cũ (bản phân tích T31: cơ chế (c)).
        contexts[slot] = *context;
        if ensure_current_root::<A>().is_err() {
            fault::suspend::<A>(context, 0, 0);
        }
        #[cfg(feature = "check-boundary")]
        boundary::check::<A>();
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
        crate::note_context::<A>(kernel, next, hart);
        let slot = slot_or_report::<A>(kernel, next, hart);
        *context = contexts[slot];
        #[cfg(feature = "diag-sched")]
        crate::diag::record(
            crate::diag::CTX_LOAD,
            hart,
            next.index as usize,
            slot,
            kernel
                .tcb(next)
                .map(|table| table.state as u64)
                .unwrap_or(255),
            kernel.fast_time(),
        );
        crate::syscall::apply_resume::<A>(context, next);
        // Ghi LẠI slot: `apply_resume` vá context SỐNG, nếu không ghi lại thì
        // lần nạp sau sẽ chạy lại snapshot cũ (bản phân tích T31: cơ chế (c)).
        contexts[slot] = *context;
        if ensure_current_root::<A>().is_err() {
            fault::suspend::<A>(context, 0, 0);
        }
        #[cfg(feature = "check-boundary")]
        boundary::check::<A>();
        true
    }
}

/// Minimal user-fault suspension.  A full fault endpoint is future work;
/// for now the faulting TCB becomes Faulted and another TCB continues.
pub mod fault {
    use super::*;

    pub fn suspend<A: Platform>(context: &mut A::Context, cause: u64, addr: u64) -> bool {
        let kernel = A::kernel();
        let hart = A::hart_id();
        let old = kernel.current_on(hart);
        // T33b (r293): ngoại lệ khi hart KHÔNG chạy TCB nào — hart đang idle
        // trong kernel, nên không có ngữ cảnh user để "fault".
        //
        // QUYẾT ĐỊNH CỦA AGENT 0 (r293): **mặc định vẫn `halt()`**. Bản cũ đi
        // thẳng tới `fault_current_on(hart)` (không có current ⇒ None) ⇒ "no
        // runnable thread after fault" ⇒ `halt()`. Hành vi "bỏ qua + ack" chỉ
        // được bật sau feature `diag-idle-fault` (mặc định TẮT), vì bỏ qua
        // im lặng sẽ NUỐT đúng lớp lỗi họ T33 mà r294 cần đếm. Khi feature
        // bật, mỗi dòng `KERNEL FAULT` vẫn bị harness tính là một sự kiện
        // T-SCHED (xem `classify_failure`).
        #[cfg(feature = "diag-idle-fault")]
        if old.is_none() {
            #[cfg(feature = "test-exit")]
            vifast_kernel::klog!(
                A,
                "KERNEL FAULT (hart={}, cause={:#x}, addr={:#x}) — hart idle, bỏ qua",
                hart,
                cause,
                addr
            );
            A::ack_timer();
            return;
        }
        // Vòng 90 (audit #4 P1-3): lưu frame đang fault vào context slot của
        // TCB để nếu pager cứu nó (map trang rồi cho chạy lại), task tiếp tục
        // đúng điểm fault thay vì chạy lại frame cũ / entry point.
        if A::fault_frame_complete() {
            if let Some(faulting) = old {
                if let Some(slot) = kernel.context_slot(faulting) {
                    A::contexts()[slot as usize] = *context;
                }
            }
        }
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
            // r294.4: KHÔNG `halt()` ở đây. Hàm này chạy trong trap handler
            // ĐANG GIỮ BKL và IF=0; `halt()` là `loop { hlt }` nên guard BKL
            // không bao giờ được nhả ⇒ hart còn lại quay BKL mãi với IF=0 ⇒
            // CẢ MÁY chết (đúng chữ ký `USER FAULT tcb=None` +
            // `no runnable thread after fault` trong 4/305 lần đo nền).
            // Trả `false` để caller nhả khoá rồi mới halt.
            vifast_kernel::klog!(A, "no runnable thread after fault");
            return false;
        };
        note_current_tcb::<A>(kernel);
        loop {
            crate::note_context::<A>(kernel, next, hart);
            let slot = slot_or_report::<A>(kernel, next, hart);
            *context = A::contexts()[slot];
            crate::syscall::apply_resume::<A>(context, next);
            A::contexts()[slot] = *context;
            if ensure_current_root::<A>().is_ok() {
                return true;
            }
            vifast_kernel::klog!(A, "FAULT SUSPEND tcb={:?}: no VSpace", next);
            let Some(candidate) = kernel.fault_current_on(hart) else {
                vifast_kernel::klog!(A, "no runnable thread with VSpace after fault");
                return false;
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
            let slot = kernel.context_slot(old).unwrap_or(0) as usize;
            contexts[slot] = *context;
            #[cfg(feature = "diag-sched")]
            crate::diag::record(
                crate::diag::CTX_SAVE,
                hart,
                old.index as usize,
                slot,
                kernel
                    .tcb(old)
                    .map(|table| table.state as u64)
                    .unwrap_or(255),
                kernel.fast_time(),
            );
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
        // T33a (r288): chẩn đoán BẬT ĐÈN — in MỘT lần bộ cờ mà `PREEMPT OK`
        // đang chờ, kèm trạng thái tiến trình, để biết nó kẹt ở điều kiện nào.
        // KHÔNG phải marker của gate (không nằm trong harness_spec).
        #[cfg(feature = "test-exit")]
        if hart == 0
            && kernel.ticks_on(0) >= TIMER_TICKS
            && !WAIT_DIAG.load(core::sync::atomic::Ordering::SeqCst)
        {
            let ready_now = TEST_DONE.load(core::sync::atomic::Ordering::SeqCst)
                && TEST_FS_DONE.load(core::sync::atomic::Ordering::SeqCst)
                && TEST_FS_LARGE_DONE.load(core::sync::atomic::Ordering::SeqCst)
                && TEST_HARD_LINK_DONE.load(core::sync::atomic::Ordering::SeqCst)
                && TEST_SYMLINK_DONE.load(core::sync::atomic::Ordering::SeqCst)
                && TEST_TLB_DONE.load(core::sync::atomic::Ordering::SeqCst)
                && A::post_idle_ready();
            if !ready_now {
                WAIT_DIAG.store(true, core::sync::atomic::Ordering::SeqCst);
                vifast_kernel::klog!(
                    A,
                    "SMP WAIT (ticks={}, done={} fs={} fsl={} link={} sym={} tlb={} post={} cur0={} cur1={} rq0={} rq1={})",
                    kernel.ticks_on(0),
                    TEST_DONE.load(core::sync::atomic::Ordering::SeqCst),
                    TEST_FS_DONE.load(core::sync::atomic::Ordering::SeqCst),
                    TEST_FS_LARGE_DONE.load(core::sync::atomic::Ordering::SeqCst),
                    TEST_HARD_LINK_DONE.load(core::sync::atomic::Ordering::SeqCst),
                    TEST_SYMLINK_DONE.load(core::sync::atomic::Ordering::SeqCst),
                    TEST_TLB_DONE.load(core::sync::atomic::Ordering::SeqCst),
                    A::post_idle_ready(),
                    kernel.current_on(0).is_some(),
                    kernel.current_on(1).is_some(),
                    kernel.runq_depth(0),
                    kernel.runq_depth(1)
                );
            }
        }
        #[cfg(feature = "test-exit")]
        if hart == 0
            && !A::post_idle_ready()
            && kernel.ticks_on(0) >= TIMER_TICKS
            && !POST_IDLE_STALLED.load(core::sync::atomic::Ordering::SeqCst)
            && TEST_DONE.load(core::sync::atomic::Ordering::SeqCst)
        {
            // Vòng 151: chẩn đoán "vì sao PREEMPT OK không bao giờ in" — chỉ in
            // MỘT lần và KHÔNG phải marker của gate.
            POST_IDLE_STALLED.store(true, core::sync::atomic::Ordering::SeqCst);
            let (timer_wakes, ipi_wakes) = A::secondary_wakeups();
            if let Some((post_ticks, idle_entries, timer_counts)) = A::post_idle_stats() {
                vifast_kernel::klog!(
                    A,
                    "SMP IDLE STALL (ticks={}, wake_timer={}, wake_sgi={}, post_ticks={}, post_idle={}, post_wakes={})",
                    kernel.ticks_on(0),
                    timer_wakes,
                    ipi_wakes,
                    post_ticks,
                    idle_entries,
                    timer_counts
                );
                // T33: thêm trạng thái hart phụ để biết nó CHẾT hay chỉ NGỦ.
                if let Some([online, window, gen, wake_gen, ipi, resched]) = A::stall_detail() {
                    vifast_kernel::klog!(
                        A,
                        "SMP AP STATE (online={}, window={}, gen={}, wake_gen={}, ipi={}, resched={})",
                        online,
                        window,
                        gen,
                        wake_gen,
                        ipi,
                        resched
                    );
                }
            } else {
                vifast_kernel::klog!(
                    A,
                    "SMP IDLE STALL (ticks={}, wake_timer={}, wake_sgi={}, post_idle_stats=none)",
                    kernel.ticks_on(0),
                    timer_wakes,
                    ipi_wakes
                );
            }
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
                "BKL STATS (h0_acq={}, h0_spin={}, h1_acq={}, h1_spin={}, h2_acq={}, h2_spin={}, h3_acq={}, h3_spin={})",
                bkl[0].0,
                bkl[0].1,
                bkl[1].0,
                bkl[1].1,
                bkl[2].0,
                bkl[2].1,
                bkl[3].0,
                bkl[3].1
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
            #[cfg(feature = "check-boundary")]
            {
                let checks = boundary::checks();
                let violations = boundary::violations();
                let (running, harts, running_harts) = kernel.boundary_summary();
                // Bất biến toàn cục ở test-exit: mỗi TCB Running thuộc đúng
                // một hart đang chạy user, và không TCB Running nào thừa.
                // Hart idle vẫn giữ `current` nên `harts` có thể lớn hơn —
                // đó KHÔNG phải vi phạm (đo được ở r259: harts=2, running=1).
                if violations == 0 && running > 0 && running == running_harts && running <= harts {
                    vifast_kernel::klog!(
                        A,
                        "BOUNDARY OK (checks={}, running={}, harts={})",
                        checks,
                        running,
                        harts
                    );
                } else {
                    // Marker riêng cho "có vi phạm": BOUNDARY BAD chỉ in tại
                    // ranh giới vi phạm (đúng một lần nhờ clear sau kiểm), nên
                    // ở đây phải là marker khác để không đếm trùng.
                    vifast_kernel::klog!(
                        A,
                        "BOUNDARY VIOLATIONS (checks={}, violations={}, running={}, harts={})",
                        checks,
                        violations,
                        running,
                        harts
                    );
                }
            }
            vifast_kernel::klog!(A, "ROOT SWITCH {}", kernel.stats().root_switches);
            vifast_kernel::klog!(A, "CTX SIZE {}", core::mem::size_of::<A::Context>());
            emit_boot_diag::<A>(&kernel);
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
                // Vòng 36: bytes leak tích lũy (free list đầy khi reclaim).
                // Kỳ vọng 0 sau boot chuẩn; >0 là tín hiệu cần dọn.
                if let Some(leaked) = kernel.untyped_leak_by_base(A::data_page().as_ptr() as usize)
                {
                    vifast_kernel::klog!(A, "DATA LEAKED {}", leaked);
                }
                if let Some((used, size)) = A::boot_stack_usage() {
                    vifast_kernel::klog!(A, "BOOT STACK used={} size={}", used, size);
                }
                // r229 (T4): canary vùng đệm sau KERNEL_STORAGE. Marker bắt buộc
                // exact-one khi còn nguyên; "KSTORE CANARY BAD" nằm trong
                // FORBIDDEN nên chỉ cần xuất hiện là gate đỏ.
                // r300: đếm syscall của tác vụ client (index 62 trên cấu hình
                // hiện tại) để biết nó có ngừng vào kernel hay không.
                {
                    let count = SYSCALL_COUNT[62].load(core::sync::atomic::Ordering::Relaxed);
                    vifast_kernel::klog!(A, "SYSCALLS t=62 n={}", count);
                }
                match A::kernel_storage_canary() {
                    Some(true) => vifast_kernel::klog!(A, "KSTORE CANARY OK"),
                    Some(false) => vifast_kernel::klog!(A, "KSTORE CANARY BAD"),
                    None => {}
                }
            }
            A::exit(0);
        }
        A::reload_timer();
        A::ack_timer();
        if old.is_some() {
            let _ = kernel.yield_current_on(hart);
        }
        let Some(next) = kernel.pick_on(hart) else {
            // T33a (r293): chẩn đoán KHÔNG-phải-marker. Thoát im lặng ở đây
            // là "hart 0 không còn TCB nào chạy được"; nếu lúc đó bài test
            // chưa xong thì đó chính là lần hỏng "thiếu PREEMPT OK".
            // 294.1: chỉ tồn tại trong build test-exit.
            #[cfg(feature = "test-exit")]
            vifast_kernel::klog!(
                A,
                "SMP NO READY (hart={}, ticks={}, exitted_current={}, alive={})",
                hart,
                kernel.ticks_on(hart),
                old.is_some(),
                kernel.live_tcb_count()
            );
            A::exit(1);
        };
        note_current_tcb::<A>(kernel);
        crate::note_context::<A>(kernel, next, hart);
        let slot = slot_or_report::<A>(kernel, next, hart);
        *context = contexts[slot];
        #[cfg(feature = "diag-sched")]
        crate::diag::record(
            crate::diag::CTX_LOAD,
            hart,
            next.index as usize,
            slot,
            kernel
                .tcb(next)
                .map(|table| table.state as u64)
                .unwrap_or(255),
            kernel.fast_time(),
        );
        crate::syscall::apply_resume::<A>(context, next);
        // Ghi LẠI slot: `apply_resume` vá context SỐNG, nếu không ghi lại thì
        // lần nạp sau sẽ chạy lại snapshot cũ (bản phân tích T31: cơ chế (c)).
        contexts[slot] = *context;
        if ensure_current_root::<A>().is_err() {
            fault::suspend::<A>(context, 0, 0);
        }
        old_index != Some(next.index)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    /// D1.a (kế hoạch qua đêm): marker `SMP NO CURRENT` là HÀM THUẦN để test
    /// host được (không cần dựng fake Platform 30+ method).
    #[test]
    fn no_current_marker_formats_hart_and_syscall_number() {
        let mut out = [0u8; 40];
        let length = no_current_marker(1, 57, &mut out);
        let text = core::str::from_utf8(&out[..length]).unwrap();
        assert_eq!(text, "SMP NO CURRENT hart=1 nr=57");
    }

    #[test]
    fn no_current_marker_handles_zero_and_large_numbers() {
        let mut out = [0u8; 40];
        let length = no_current_marker(0, 0, &mut out);
        assert_eq!(
            core::str::from_utf8(&out[..length]).unwrap(),
            "SMP NO CURRENT hart=0 nr=0"
        );
        let length = no_current_marker(3, 4_294_967_295, &mut out);
        assert_eq!(
            core::str::from_utf8(&out[..length]).unwrap(),
            "SMP NO CURRENT hart=3 nr=4294967295"
        );
    }

    /// T31 (bản phân tích): `context_slot(..) == None` KHÔNG được lặng lẽ dùng
    /// slot 0. Phần quyết định tách thành hàm THUẦN `slot_choice` để test host
    /// được mà không phải dựng fake Platform (đúng cách đã làm với D1.a).
    #[test]
    fn slot_choice_keeps_the_real_slot_and_flags_a_missing_one() {
        assert_eq!(slot_choice(Some(7)), (7, false));
        assert_eq!(
            slot_choice(Some(0)),
            (0, false),
            "slot 0 hợp lệ vẫn là hợp lệ"
        );
        assert_eq!(slot_choice(None), (0, true), "thiếu slot thì phải BÁO");
    }

    /// Test canh: marker KHÔNG được tràn buffer kể cả khi số lớn.
    #[test]
    fn no_current_marker_never_overflows_a_small_buffer() {
        let mut out = [0u8; 40];
        let length = no_current_marker(u32::MAX, u64::MAX, &mut out);
        assert!(length <= out.len());
        assert!(core::str::from_utf8(&out[..length]).is_ok());
    }
}
