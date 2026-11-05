//! Rust root task image.
//!
//! The bootstrap actions mirror the old `user_root` assembly.  The image is
//! linked as an ELF at `ROOT_ELF_BASE`; the runtime loads PT_LOAD segments
//! before entering `_start`.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::{
    CLIENT_IRQ_NOTIFICATION_SLOT, CLIENT_IRQ_SLOT, CLIENT_STACK_BASE, CLIENT_STACK_TOP,
    CLIENT_UART_IRQ_SLOT, CLIENT_UART_NOTIFICATION_SLOT, CODE_VA, FAULT1_STACK_TOP,
    FAULT2_STACK_TOP, FAULT3_STACK_TOP, FAULT4_STACK_BASE, FAULT4_STACK_TOP, IDLE_STACK_BASE,
    IDLE_STACK_TOP, IPC_BUF_CLIENT_VA, IPC_BUF_SERVER_VA, SERVER_STACK_TOP, USER_ENTRY_FAULT1,
    USER_ENTRY_FAULT2, USER_ENTRY_FAULT3, USER_ENTRY_FAULT4, WORKER_A_STACK_BASE,
    WORKER_A_STACK_TOP, WORKER_B_STACK_BASE, WORKER_B_STACK_TOP, WORKER_C_STACK_BASE,
    WORKER_C_STACK_TOP, WORKER_D_STACK_BASE, WORKER_D_STACK_TOP, WORKER_E_STACK_BASE,
    WORKER_E_STACK_TOP,
};
use vifast_abi::{
    BootInfo, KError, ObjType, Rights, BOOTINFO_MAGIC, BOOTINFO_VERSION, IRQ_TIMER, IRQ_UART,
};
use vifast_user::SyscallResult;

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("start_x86_64.S"));

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("start_aarch64.S"));

#[cfg(target_arch = "riscv64")]
core::arch::global_asm!(include_str!("start_riscv64.S"));

const ROOT_RUST: &[u8] = b"ROOT RUST\n";
const ROOT_READY: &[u8] = b"ROOT READY\n";
const BOOTINFO_OK: &[u8] = b"BOOTINFO OK\n";
const CLIENT_VSPACE_OK: &[u8] = b"CLIENT VSPACE OK\n";
const FAULT_EP_OK: &[u8] = b"FAULT EP OK addr=0x";
const ROOT_PANIC: &[u8] = b"ROOT PANIC\n";
const MODULES_OK: &[u8] = b"RUST MODULES OK\n";
const IOMMU_OK: &[u8] = b"IOMMU OK\n";
const MODULE_WINDOW: u64 = 0x10_0000;
const WORKER_LABEL_A: u64 = 0x5750_0001;
const WORKER_LABEL_B: u64 = 0x5750_0002;
const WORKER_LABEL_C: u64 = 0x5750_0003;
const WORKER_LABEL_D_READY: u64 = 0x5750_0004;
const WORKER_LABEL_D_DONE: u64 = 0x5750_0005;
const WORKER_LABEL_C_RUNNING: u64 = 0x5750_0006;
const TLB_TEST_VA: u64 = CODE_VA as u64 + 0x40_0000;
const SUSPEND_FLAG_VA: u64 = TLB_TEST_VA + 0x1000;
const TCB_SUSPEND_OK: &[u8] = b"TCB SUSPEND OK\n";
const TCB_SUSPEND_BAD: &[u8] = b"TCB SUSPEND BAD\n";
const SCHED_CTX_OK: &[u8] = b"SCHED CTX OK\n";
const TLB_A: u64 = 0xAAAA_AAAA_AAAA_AAAA;
const TLB_B: u64 = 0xBBBB_BBBB_BBBB_BBBB;
const TLB_REMAP_OK: &[u8] = b"TLB REMAP OK (seen_new=1)\n";
const TLB_REMAP_BAD: &[u8] = b"TLB REMAP BAD (stale)\n";
const SMP_WORKERS_OK: &[u8] = b"SMP WORKERS OK (a=50, b=50, c=1)\n";
const TIME_MONOTONIC_OK: &[u8] = b"TIME MONOTONIC OK\n";
const TIME_MONOTONIC_BAD: &[u8] = b"TIME MONOTONIC BAD\n";
#[cfg(target_arch = "riscv64")]
const USER_ELF_MACHINE: u16 = 243;
#[cfg(target_arch = "aarch64")]
const USER_ELF_MACHINE: u16 = 183;
#[cfg(target_arch = "x86_64")]
const USER_ELF_MACHINE: u16 = 62;
const CLIENT_UNTYPED_SIZE: usize = 0x4000;
const CLIENT_UNTYPED_OBJECTS: usize = 4;

#[no_mangle]
pub extern "C" fn idle_main() -> ! {
    loop {
        let _ = vifast_user::yield_now();
    }
}

fn worker_loop(label: u64) -> ! {
    let mut i = 0;
    while i < 50 {
        let _ = vifast_user::yield_now();
        i += 1;
    }
    // Endpoint slot 1 is installed by spawn_worker.  The root task services
    // this endpoint while it is already blocked for fault delivery; a
    // notification-only design would block fault4 handling.
    let _ = vifast_user::send(1, label, 50, 0, 0);
    // Slot 2 is a private notification that is never signalled.  Blocking here
    // leaves CPU1 idle so the post-worker timer/WFI tests have a long window.
    let _ = vifast_user::wait(2);
    loop {
        let _ = vifast_user::yield_now();
    }
}

#[no_mangle]
pub extern "C" fn smp_worker_a() -> ! {
    worker_loop(WORKER_LABEL_A)
}

#[no_mangle]
pub extern "C" fn smp_worker_b() -> ! {
    worker_loop(WORKER_LABEL_B)
}

/// Busy worker: no Yield until at least five timer ticks have elapsed.  The
/// timer must therefore preempt it for a/b to run; it then signals root bit 2.
#[no_mangle]
pub extern "C" fn smp_worker_c() -> ! {
    let start = vifast_user::get_time().value;
    let _ = vifast_user::send(1, WORKER_LABEL_C_RUNNING, start, 0, 0);
    let mut last = start;
    // Busy until root has suspended/resumed us and sets the shared flag.
    while unsafe { core::ptr::read_volatile(SUSPEND_FLAG_VA as *const u64) } == 0 {
        let now = vifast_user::get_time().value;
        if now < last {
            write(TIME_MONOTONIC_BAD);
            panic!("GetTime went backwards on hart 1");
        }
        last = now;
    }
    let elapsed = vifast_user::get_time().value.saturating_sub(start);
    let _ = vifast_user::signal_bits(3, 2);
    let _ = vifast_user::send(1, WORKER_LABEL_C, 1, elapsed, 0);
    let _ = vifast_user::wait(2);
    loop {
        let _ = vifast_user::yield_now();
    }
}

#[no_mangle]
pub extern "C" fn smp_worker_e() -> ! {
    let start = vifast_user::get_time().value;
    // Keeper for the five-tick suspension window: C is the only busy task,
    // so hart 1 needs another runnable TCB or its timer would find no task.
    while vifast_user::get_time().value < start.saturating_add(12) {
        let _ = vifast_user::yield_now();
    }
    let _ = vifast_user::wait(2);
    loop {
        let _ = vifast_user::yield_now();
    }
}

#[no_mangle]
pub extern "C" fn smp_worker_d() -> ! {
    let start = vifast_user::get_time().value;
    let mut next_ready = start;
    let mut a_seen = 0u64;
    let mut seen_new = false;
    let done_at;
    loop {
        let now = vifast_user::get_time().value;
        // Continuous polling while root performs the unmap/remap.  Do not
        // block or switch root: the stale TLB entry must be observable.
        let value = unsafe { core::ptr::read_volatile(TLB_TEST_VA as *const u64) };
        if value == TLB_A {
            a_seen = a_seen.saturating_add(1);
            if now >= next_ready {
                let _ = vifast_user::send(1, WORKER_LABEL_D_READY, 1, 0, 0);
                next_ready = now.saturating_add(5);
            }
        }
        if value == TLB_B {
            seen_new = true;
            done_at = now;
            break;
        }
        if now >= start.saturating_add(50) {
            done_at = now;
            break;
        }
    }
    let _ = vifast_user::send(
        1,
        WORKER_LABEL_D_DONE,
        seen_new as u64,
        a_seen,
        done_at.saturating_sub(start),
    );
    let _ = vifast_user::wait(2);
    loop {
        let _ = vifast_user::yield_now();
    }
}

fn check(result: SyscallResult) {
    if result.error != KError::Ok {
        let mut digits = [0u8; 10];
        let mut value = result.error as u32;
        let mut at = digits.len();
        loop {
            at -= 1;
            digits[at] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        let _ = vifast_user::debug_write(b"ROOT ERR ".as_ptr() as u64, 9);
        let _ = vifast_user::debug_write(digits[at..].as_ptr() as u64, (digits.len() - at) as u64);
        let _ = vifast_user::debug_write(b"\n".as_ptr() as u64, 1);
        panic!("root syscall failed: {:?}", result.error);
    }
}

fn write(bytes: &[u8]) {
    check(vifast_user::debug_write(
        bytes.as_ptr() as u64,
        bytes.len() as u64,
    ));
}

fn retype(untyped_slot: u8, ty: ObjType, slot: u8) {
    check(vifast_user::retype(untyped_slot, ty as u64, slot));
}

fn write_fault_addr(addr: u64) {
    write(FAULT_EP_OK);
    let mut hex = [b'0'; 16];
    let digits = b"0123456789abcdef";
    let mut value = addr;
    let mut index = 16;
    while index != 0 {
        index -= 1;
        hex[index] = digits[(value & 0xf) as usize];
        value >>= 4;
    }
    check(vifast_user::debug_write(
        hex.as_ptr() as u64,
        hex.len() as u64,
    ));
}

fn copy(src: u8, tcb: u8, dst: u8, rights: Rights) {
    check(vifast_user::cap_copy(src, tcb, dst, rights.0));
}

fn map_with_tables(
    untyped_slot: u8,
    vspace_slot: u8,
    frame_slot: u8,
    va: u64,
    rights: Rights,
    slots: &mut SlotAllocator,
) {
    for _ in 0..4 {
        let mapped = vifast_user::frame_map(vspace_slot, frame_slot, va, rights.0);
        if mapped.error == KError::Ok {
            return;
        }
        if mapped.error != KError::MissingPageTable {
            let mut digits = [0u8; 10];
            let mut value = mapped.error as u32;
            let mut at = digits.len();
            loop {
                at -= 1;
                digits[at] = b'0' + (value % 10) as u8;
                value /= 10;
                if value == 0 {
                    break;
                }
            }
            let _ = vifast_user::debug_write(b"ROOT MAP ERR ".as_ptr() as u64, 13);
            let _ =
                vifast_user::debug_write(digits[at..].as_ptr() as u64, (digits.len() - at) as u64);
            let _ = vifast_user::debug_write(b"\n".as_ptr() as u64, 1);
            panic!("frame map failed: {:?}", mapped.error);
        }
        let table = slots.alloc();
        retype(untyped_slot, ObjType::PageTable, table);
        check(vifast_user::page_table_map(table, vspace_slot, va));
    }
    panic!("page table depth exceeded");
}

struct SlotAllocator {
    next: u8,
    end: u8,
}

impl SlotAllocator {
    fn alloc(&mut self) -> u8 {
        if self.next >= self.end {
            panic!("root slot range exhausted");
        }
        let slot = self.next;
        self.next += 1;
        slot
    }
}

fn load_module(
    data_untyped: u8,
    target_vspace: u8,
    bytes: &[u8],
    slots: &mut SlotAllocator,
) -> u64 {
    let image = match vifast_elf::parse(
        bytes,
        USER_ELF_MACHINE,
        CODE_VA as u64,
        CODE_VA as u64 + MODULE_WINDOW,
    ) {
        Ok(image) => image,
        Err(_) => {
            write(b"MODULE PARSE BAD\n");
            panic!("module ELF invalid");
        }
    };
    for segment in image.segments() {
        let page_base = segment.vaddr & !0xfff;
        let page_count = segment.page_count();
        let mut page = 0;
        while page < page_count {
            let va = page_base + page as u64 * vifast_abi::DEFAULT_PAGE_SIZE;
            let frame = slots.alloc();
            retype(data_untyped, ObjType::Frame, frame);
            let copy_start = core::cmp::max(va, segment.vaddr);
            let copy_end = core::cmp::min(
                va + vifast_abi::DEFAULT_PAGE_SIZE,
                segment.vaddr + segment.file_size as u64,
            );
            if copy_start < copy_end {
                let source = segment.file_offset + (copy_start - segment.vaddr) as usize;
                let frame_offset = copy_start - va;
                let mut done = 0u64;
                let total = copy_end - copy_start;
                while done < total {
                    let chunk = core::cmp::min(256, total - done);
                    check(vifast_user::frame_write(
                        frame,
                        frame_offset + done,
                        bytes.as_ptr().wrapping_add(source + done as usize) as u64,
                        chunk,
                    ));
                    done += chunk;
                }
            }
            let mut rights = Rights::READ;
            if segment.writable() {
                rights = rights | Rights::WRITE;
            }
            if segment.executable() {
                rights = rights | Rights::EXEC;
            }
            map_with_tables(data_untyped, target_vspace, frame, va, rights, slots);
            page += 1;
        }
    }
    image.entry
}

fn spawn_idle(meta_untyped: u8, data_untyped: u8, root_vspace: u8, slots: &mut SlotAllocator) {
    let tcb = slots.alloc();
    retype(meta_untyped, ObjType::Tcb, tcb);
    let stack_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, stack_frame);
    map_with_tables(
        data_untyped,
        root_vspace,
        stack_frame,
        IDLE_STACK_BASE as u64,
        Rights::READ | Rights::WRITE,
        slots,
    );
    check(vifast_user::tcb_configure(
        tcb,
        idle_main as *const () as u64,
        IDLE_STACK_TOP as u64,
        root_vspace,
        0,
    ));
    check(vifast_user::tcb_resume(tcb));
}

fn spawn_worker(
    meta_untyped: u8,
    data_untyped: u8,
    root_vspace: u8,
    fault_endpoint: u8,
    idle_notification: u8,
    notification_rights: Rights,
    completion_notification: Option<u8>,
    priority: u8,
    entry: usize,
    stack_base: usize,
    stack_top: usize,
    slots: &mut SlotAllocator,
) -> u8 {
    let tcb = slots.alloc();
    retype(meta_untyped, ObjType::Tcb, tcb);
    let stack_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, stack_frame);
    map_with_tables(
        data_untyped,
        root_vspace,
        stack_frame,
        stack_base as u64,
        Rights::READ | Rights::WRITE,
        slots,
    );
    check(vifast_user::tcb_configure(
        tcb,
        entry as u64,
        stack_top as u64,
        root_vspace,
        priority,
    ));
    copy(fault_endpoint, tcb, 1, Rights::WRITE);
    copy(idle_notification, tcb, 2, notification_rights);
    if let Some(notification) = completion_notification {
        copy(notification, tcb, 3, Rights::WRITE);
    }
    check(vifast_user::tcb_set_affinity(tcb, 0b10));
    check(vifast_user::tcb_resume(tcb));
    tcb
}

#[no_mangle]
pub extern "C" fn root_main(bootinfo_ptr: u64) -> ! {
    write(ROOT_RUST);

    if bootinfo_ptr == 0 {
        panic!("missing BootInfo pointer");
    }
    let info = unsafe { core::ptr::read_volatile(bootinfo_ptr as *const BootInfo) };
    if info.magic != BOOTINFO_MAGIC
        || info.version != BOOTINFO_VERSION
        || info.size as usize != core::mem::size_of::<BootInfo>()
    {
        panic!("invalid BootInfo");
    }
    let root_vspace = info.root_vspace as u8;
    let meta_untyped = info.meta_untyped as u8;
    let data_untyped = info.data_untyped as u8;
    let server_vspace = info.server_vspace as u8;
    if root_vspace == 0
        || meta_untyped == 0
        || data_untyped == 0
        || server_vspace == 0
        || info.user_code_count == 0
        || info.free_slot_start == 0
        || info.free_slot_start >= info.free_slot_end
        || info.block_device == 0
        || info.server_elf_va == 0
        || info.server_elf_len == 0
        || info.client_elf_va == 0
        || info.client_elf_len == 0
        || info.sched_harts & 0b01 == 0
        || info.sched_harts & !0b11 != 0
    {
        panic!("invalid BootInfo slots");
    }
    write(BOOTINFO_OK);

    let mut slots = SlotAllocator {
        next: info.free_slot_start as u8,
        end: info.free_slot_end as u8,
    };
    let endpoint_slot = slots.alloc();
    let server_tcb = slots.alloc();
    let client_tcb = slots.alloc();
    let notification_slot = slots.alloc();
    let root_notification_slot = slots.alloc();
    let completion_notification_slot = slots.alloc();
    let fault1_tcb = slots.alloc();
    let fault2_tcb = slots.alloc();
    let fault3_tcb = slots.alloc();
    let fault4_tcb = slots.alloc();
    let fault_endpoint_slot = slots.alloc();

    retype(meta_untyped, ObjType::Endpoint, endpoint_slot);
    retype(meta_untyped, ObjType::Tcb, server_tcb);
    retype(meta_untyped, ObjType::Tcb, client_tcb);
    retype(meta_untyped, ObjType::Notification, notification_slot);
    retype(meta_untyped, ObjType::Notification, root_notification_slot);
    retype(
        meta_untyped,
        ObjType::Notification,
        completion_notification_slot,
    );
    retype(meta_untyped, ObjType::Tcb, fault1_tcb);
    retype(meta_untyped, ObjType::Tcb, fault2_tcb);
    retype(meta_untyped, ObjType::Tcb, fault3_tcb);
    retype(meta_untyped, ObjType::Tcb, fault4_tcb);
    retype(meta_untyped, ObjType::Endpoint, fault_endpoint_slot);

    check(vifast_user::tcb_configure(
        fault1_tcb,
        USER_ENTRY_FAULT1 as u64,
        FAULT1_STACK_TOP as u64,
        root_vspace,
        6,
    ));
    check(vifast_user::tcb_resume(fault1_tcb));
    check(vifast_user::tcb_configure(
        fault2_tcb,
        USER_ENTRY_FAULT2 as u64,
        FAULT2_STACK_TOP as u64,
        root_vspace,
        6,
    ));
    check(vifast_user::tcb_resume(fault2_tcb));
    check(vifast_user::tcb_configure(
        fault3_tcb,
        USER_ENTRY_FAULT3 as u64,
        FAULT3_STACK_TOP as u64,
        root_vspace,
        6,
    ));
    check(vifast_user::tcb_resume(fault3_tcb));

    let client_vspace = slots.alloc();
    retype(data_untyped, ObjType::VSpace, client_vspace);
    for index in 0..info.user_code_count {
        let code_slot = info.user_code_first + index;
        let va = CODE_VA as u64 + index as u64 * vifast_abi::DEFAULT_PAGE_SIZE;
        map_with_tables(
            data_untyped,
            client_vspace,
            code_slot as u8,
            va,
            Rights::READ | Rights::EXEC,
            &mut slots,
        );
    }
    // CODE_VA, CLIENT_STACK_BASE, FAULT4_STACK_BASE and DATA_VA share one
    // 2 MiB table.  Building the code and stack mappings therefore leaves the
    // final-level table in place for the client's later FrameMap at DATA_VA.
    let client_stack_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, client_stack_frame);
    map_with_tables(
        data_untyped,
        client_vspace,
        client_stack_frame,
        CLIENT_STACK_BASE as u64,
        Rights::READ | Rights::WRITE,
        &mut slots,
    );
    let fault4_stack_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, fault4_stack_frame);
    map_with_tables(
        data_untyped,
        client_vspace,
        fault4_stack_frame,
        FAULT4_STACK_BASE as u64,
        Rights::READ | Rights::WRITE,
        &mut slots,
    );
    let server_buffer_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, server_buffer_frame);
    map_with_tables(
        data_untyped,
        server_vspace,
        server_buffer_frame,
        IPC_BUF_SERVER_VA as u64,
        Rights::READ | Rights::WRITE,
        &mut slots,
    );
    let client_buffer_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, client_buffer_frame);
    map_with_tables(
        data_untyped,
        client_vspace,
        client_buffer_frame,
        IPC_BUF_CLIENT_VA as u64,
        Rights::READ | Rights::WRITE,
        &mut slots,
    );

    let iommu = slots.alloc();
    retype(data_untyped, ObjType::Iommu, iommu);
    check(vifast_user::iommu_map(
        iommu,
        vifast_abi::IOMMU_IOVA_BASE,
        server_buffer_frame,
        (Rights::READ | Rights::WRITE).0,
    ));
    check(vifast_user::iommu_unmap(iommu, vifast_abi::IOMMU_IOVA_BASE));
    write(IOMMU_OK);

    let server_bytes = unsafe {
        core::slice::from_raw_parts(
            info.server_elf_va as *const u8,
            info.server_elf_len as usize,
        )
    };
    let client_bytes = unsafe {
        core::slice::from_raw_parts(
            info.client_elf_va as *const u8,
            info.client_elf_len as usize,
        )
    };
    let server_entry = load_module(data_untyped, server_vspace, server_bytes, &mut slots);
    let client_entry = load_module(data_untyped, client_vspace, client_bytes, &mut slots);
    write(MODULES_OK);
    write(CLIENT_VSPACE_OK);

    copy(endpoint_slot, server_tcb, 1, Rights::READ);
    copy(
        info.block_device as u8,
        server_tcb,
        2,
        Rights::READ | Rights::WRITE,
    );
    check(vifast_user::tcb_configure(
        server_tcb,
        server_entry,
        SERVER_STACK_TOP as u64,
        server_vspace,
        5,
    ));
    check(vifast_user::tcb_set_ipc_buffer(
        server_tcb,
        server_buffer_frame,
        IPC_BUF_SERVER_VA as u64,
    ));
    check(vifast_user::tcb_resume(server_tcb));

    let client_untyped = slots.alloc();
    check(vifast_user::retype_untyped(
        data_untyped,
        CLIENT_UNTYPED_SIZE as u64,
        CLIENT_UNTYPED_OBJECTS as u64,
        client_untyped,
    ));

    check(vifast_user::cap_mint(
        endpoint_slot,
        client_tcb,
        1,
        Rights::WRITE.0,
        1,
    ));
    check(vifast_user::cap_mint(
        endpoint_slot,
        client_tcb,
        6,
        Rights::WRITE.0,
        2,
    ));
    // Give the client a cap to its own TCB so it can attempt an illegal
    // rebadge of the badge-2 endpoint.  The endpoint itself remains the only
    // client endpoint cap; there is no badge-0 endpoint in the client.
    copy(client_tcb, client_tcb, 7, Rights::WRITE);
    copy(client_vspace, client_tcb, 2, Rights::WRITE);
    copy(client_untyped, client_tcb, 3, Rights::WRITE);
    copy(
        notification_slot,
        client_tcb,
        5,
        Rights::READ | Rights::WRITE | Rights::GRANT,
    );
    check(vifast_user::tcb_configure(
        client_tcb,
        client_entry,
        CLIENT_STACK_TOP as u64,
        client_vspace,
        5,
    ));
    check(vifast_user::tcb_set_ipc_buffer(
        client_tcb,
        client_buffer_frame,
        IPC_BUF_CLIENT_VA as u64,
    ));

    // Timer IRQ capability and notification for the client IRQ demo.  Root is
    // the only task allowed to call IrqGet; the cap is copied to the client.
    let irq_notification = slots.alloc();
    retype(meta_untyped, ObjType::Notification, irq_notification);
    let irq_slot = slots.alloc();
    check(vifast_user::irq_get(IRQ_TIMER, irq_slot));
    copy(
        irq_slot,
        client_tcb,
        CLIENT_IRQ_SLOT,
        Rights::READ | Rights::WRITE,
    );
    copy(
        irq_notification,
        client_tcb,
        CLIENT_IRQ_NOTIFICATION_SLOT,
        Rights::READ | Rights::WRITE | Rights::GRANT,
    );
    check(vifast_user::irq_set_notification(
        irq_slot,
        irq_notification,
    ));

    let uart_notification = slots.alloc();
    retype(meta_untyped, ObjType::Notification, uart_notification);
    let uart_irq_slot = slots.alloc();
    check(vifast_user::irq_get(IRQ_UART, uart_irq_slot));
    copy(
        uart_irq_slot,
        client_tcb,
        CLIENT_UART_IRQ_SLOT,
        Rights::READ | Rights::WRITE,
    );
    copy(
        uart_notification,
        client_tcb,
        CLIENT_UART_NOTIFICATION_SLOT,
        Rights::READ | Rights::WRITE | Rights::GRANT,
    );
    check(vifast_user::irq_set_notification(
        uart_irq_slot,
        uart_notification,
    ));
    check(vifast_user::tcb_resume(client_tcb));

    copy(
        fault_endpoint_slot,
        fault4_tcb,
        6,
        Rights::READ | Rights::WRITE,
    );
    check(vifast_user::tcb_configure_fault(
        fault4_tcb,
        USER_ENTRY_FAULT4 as u64,
        FAULT4_STACK_TOP as u64,
        client_vspace,
        6,
        6,
    ));
    check(vifast_user::tcb_resume(fault4_tcb));
    spawn_idle(meta_untyped, data_untyped, root_vspace, &mut slots);

    // Per-VA TLB demo: map A, perform a local-only unmap (hart 1 has not
    // loaded the root VSpace yet), then map A again and let worker d establish
    // the hart-1 TLB entry.
    let tlb_frame_a = slots.alloc();
    retype(data_untyped, ObjType::Frame, tlb_frame_a);
    let tlb_frame_b = slots.alloc();
    retype(data_untyped, ObjType::Frame, tlb_frame_b);
    let a_value = TLB_A;
    let b_value = TLB_B;
    check(vifast_user::frame_write(
        tlb_frame_a,
        0,
        &a_value as *const u64 as u64,
        8,
    ));
    check(vifast_user::frame_write(
        tlb_frame_b,
        0,
        &b_value as *const u64 as u64,
        8,
    ));
    map_with_tables(
        data_untyped,
        root_vspace,
        tlb_frame_a,
        TLB_TEST_VA,
        Rights::READ,
        &mut slots,
    );
    check(vifast_user::frame_unmap(
        root_vspace,
        TLB_TEST_VA,
        tlb_frame_a,
    ));
    map_with_tables(
        data_untyped,
        root_vspace,
        tlb_frame_a,
        TLB_TEST_VA,
        Rights::READ,
        &mut slots,
    );

    let suspend_flag_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, suspend_flag_frame);
    let zero = 0u64;
    check(vifast_user::frame_write(
        suspend_flag_frame,
        0,
        &zero as *const u64 as u64,
        8,
    ));
    map_with_tables(
        data_untyped,
        root_vspace,
        suspend_flag_frame,
        SUSPEND_FLAG_VA,
        Rights::READ,
        &mut slots,
    );

    let mut worker_c_tcb = 0u8;
    if info.sched_harts & 0b10 != 0 {
        spawn_worker(
            meta_untyped,
            data_untyped,
            root_vspace,
            fault_endpoint_slot,
            root_notification_slot,
            Rights::READ,
            None,
            8,
            smp_worker_d as *const () as usize,
            WORKER_D_STACK_BASE,
            WORKER_D_STACK_TOP,
            &mut slots,
        );
        spawn_worker(
            meta_untyped,
            data_untyped,
            root_vspace,
            fault_endpoint_slot,
            root_notification_slot,
            Rights::READ,
            None,
            5,
            smp_worker_a as *const () as usize,
            WORKER_A_STACK_BASE,
            WORKER_A_STACK_TOP,
            &mut slots,
        );
        spawn_worker(
            meta_untyped,
            data_untyped,
            root_vspace,
            fault_endpoint_slot,
            root_notification_slot,
            Rights::READ,
            None,
            5,
            smp_worker_b as *const () as usize,
            WORKER_B_STACK_BASE,
            WORKER_B_STACK_TOP,
            &mut slots,
        );
        worker_c_tcb = spawn_worker(
            meta_untyped,
            data_untyped,
            root_vspace,
            fault_endpoint_slot,
            root_notification_slot,
            Rights::READ,
            Some(completion_notification_slot),
            5,
            smp_worker_c as *const () as usize,
            WORKER_C_STACK_BASE,
            WORKER_C_STACK_TOP,
            &mut slots,
        );
    }
    if worker_c_tcb != 0 {
        let sched_slot = slots.alloc();
        check(vifast_user::retype(
            data_untyped,
            ObjType::SchedContext as u64,
            sched_slot,
        ));
        check(vifast_user::sched_context_configure(sched_slot, 1000, 1000));
        check(vifast_user::sched_context_bind(worker_c_tcb, sched_slot));
        write(SCHED_CTX_OK);
    }
    write(ROOT_READY);

    let mut worker_bits = 0u8;
    let mut workers_ok = false;
    let mut tlb_remapped = false;
    let mut tlb_done = false;
    let mut suspend_done = false;
    let mut suspend_ok = false;
    let mut suspend_printed = false;
    loop {
        let message = unsafe { vifast_user::recv_message(fault_endpoint_slot, 0) };
        if message.error == KError::Ok {
            if message.label == WORKER_LABEL_C_RUNNING {
                if !suspend_done {
                    spawn_worker(
                        meta_untyped,
                        data_untyped,
                        root_vspace,
                        fault_endpoint_slot,
                        root_notification_slot,
                        Rights::READ,
                        None,
                        5,
                        smp_worker_e as *const () as usize,
                        WORKER_E_STACK_BASE,
                        WORKER_E_STACK_TOP,
                        &mut slots,
                    );
                    let suspended = vifast_user::tcb_suspend(worker_c_tcb);
                    let started = vifast_user::get_time().value;
                    while vifast_user::get_time().value < started.saturating_add(5) {
                        let _ = vifast_user::yield_now();
                    }
                    let resumed = vifast_user::tcb_resume(worker_c_tcb);
                    // Keep C busy for five more ticks after resume so the AP
                    // timer observes preemption again before the flag ends it.
                    let resumed_at = vifast_user::get_time().value;
                    while vifast_user::get_time().value < resumed_at.saturating_add(5) {
                        let _ = vifast_user::yield_now();
                    }
                    let one = 1u64;
                    check(vifast_user::frame_write(
                        suspend_flag_frame,
                        0,
                        &one as *const u64 as u64,
                        8,
                    ));
                    suspend_ok = suspended.error == KError::Ok && resumed.error == KError::Ok;
                    suspend_done = true;
                }
            } else if message.label == WORKER_LABEL_D_READY {
                if !tlb_remapped {
                    check(vifast_user::frame_remap(
                        root_vspace,
                        tlb_frame_b,
                        TLB_TEST_VA,
                        Rights::READ.0,
                    ));
                    tlb_remapped = true;
                }
            } else if message.label == WORKER_LABEL_D_DONE {
                if !tlb_done {
                    tlb_done = true;
                    if message.data[0] == 1 {
                        write(TLB_REMAP_OK);
                        let mut digits = [0u8; 20];
                        let mut value = message.data[1];
                        let mut at = digits.len();
                        loop {
                            at -= 1;
                            digits[at] = b'0' + (value % 10) as u8;
                            value /= 10;
                            if value == 0 {
                                break;
                            }
                        }
                        let _ = vifast_user::debug_write(b"TLB A SEEN (count=".as_ptr() as u64, 18);
                        let _ = vifast_user::debug_write(
                            digits[at..].as_ptr() as u64,
                            (digits.len() - at) as u64,
                        );
                        let _ = vifast_user::debug_write(b")\n".as_ptr() as u64, 2);
                        let mut value = message.data[2];
                        let mut at = digits.len();
                        loop {
                            at -= 1;
                            digits[at] = b'0' + (value % 10) as u8;
                            value /= 10;
                            if value == 0 {
                                break;
                            }
                        }
                        let _ =
                            vifast_user::debug_write(b"TLB LATENCY (ticks=".as_ptr() as u64, 19);
                        let _ = vifast_user::debug_write(
                            digits[at..].as_ptr() as u64,
                            (digits.len() - at) as u64,
                        );
                        let _ = vifast_user::debug_write(b")\n".as_ptr() as u64, 2);
                    } else {
                        write(TLB_REMAP_BAD);
                    }
                }
            } else if message.label == WORKER_LABEL_A {
                if message.data[0] == 50 {
                    worker_bits |= 1;
                }
            } else if message.label == WORKER_LABEL_B {
                if message.data[0] == 50 {
                    worker_bits |= 2;
                }
            } else if message.label == WORKER_LABEL_C {
                let bits = vifast_user::poll(completion_notification_slot);
                if message.data[0] == 1 && bits.error == KError::Ok && bits.value & 2 != 0 {
                    worker_bits |= 4;
                    if suspend_done && !suspend_printed {
                        suspend_printed = true;
                        if suspend_ok && message.data[1] >= 5 {
                            write(TCB_SUSPEND_OK);
                        } else {
                            write(TCB_SUSPEND_BAD);
                        }
                    }
                }
            } else {
                write_fault_addr(message.data[0]);
            }
            if worker_bits == 7 && !workers_ok {
                write(SMP_WORKERS_OK);
                workers_ok = true;
                // The AP worker already checked its own sequence.  Repeat the
                // check on the BSP so a task on each hart observed monotonic
                // system time, then publish one aggregate line.
                let mut last = vifast_user::get_time().value;
                let mut monotonic = true;
                for _ in 0..8 {
                    let now = vifast_user::get_time().value;
                    if now < last {
                        monotonic = false;
                    }
                    last = now;
                }
                if monotonic {
                    write(TIME_MONOTONIC_OK);
                } else {
                    write(TIME_MONOTONIC_BAD);
                }
            }
        }
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let _ = vifast_user::debug_write(ROOT_PANIC.as_ptr() as u64, ROOT_PANIC.len() as u64);
    if let Some(location) = info.location() {
        let file = location.file().as_bytes();
        let _ = vifast_user::debug_write(file.as_ptr() as u64, file.len() as u64);
        let mut digits = [0u8; 20];
        let mut value = location.line();
        let mut at = digits.len();
        loop {
            at -= 1;
            digits[at] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        let _ = vifast_user::debug_write(digits[at..].as_ptr() as u64, (digits.len() - at) as u64);
        let _ = vifast_user::debug_write(b"\n".as_ptr() as u64, 1);
    }
    vifast_user::exit()
}
