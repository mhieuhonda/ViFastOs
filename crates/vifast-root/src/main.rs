//! Rust root task image.
//!
//! The bootstrap actions mirror the old `user_root` assembly.  The image is
//! linked as an ELF at `ROOT_ELF_BASE`; the runtime loads PT_LOAD segments
//! before entering `_start`.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::{
    CLIENT_STACK_BASE, CLIENT_STACK_TOP, CODE_VA, FAULT1_STACK_TOP, FAULT2_STACK_TOP,
    FAULT3_STACK_TOP, FAULT4_STACK_BASE, FAULT4_STACK_TOP, IPC_BUF_CLIENT_VA, IPC_BUF_SERVER_VA,
    SERVER_STACK_TOP, USER_ENTRY_FAULT1, USER_ENTRY_FAULT2, USER_ENTRY_FAULT3, USER_ENTRY_FAULT4,
};
use vifast_abi::{BootInfo, KError, ObjType, Rights, BOOTINFO_MAGIC, BOOTINFO_VERSION};
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
static SERVER_ELF: &[u8] = include_bytes!(env!("VIFAST_SERVER_ELF"));
static CLIENT_ELF: &[u8] = include_bytes!(env!("VIFAST_CLIENT_ELF"));
const MODULE_WINDOW: u64 = 0x10_0000;
#[cfg(target_arch = "riscv64")]
const USER_ELF_MACHINE: u16 = 243;
#[cfg(target_arch = "aarch64")]
const USER_ELF_MACHINE: u16 = 183;
#[cfg(target_arch = "x86_64")]
const USER_ELF_MACHINE: u16 = 62;
const CLIENT_UNTYPED_SIZE: usize = 0x1_0000;
const CLIENT_UNTYPED_OBJECTS: usize = 8;

fn check(result: SyscallResult) {
    if result.error != KError::Ok {
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

    let server_entry = load_module(data_untyped, server_vspace, SERVER_ELF, &mut slots);
    let client_entry = load_module(data_untyped, client_vspace, CLIENT_ELF, &mut slots);
    write(MODULES_OK);
    write(CLIENT_VSPACE_OK);

    copy(endpoint_slot, server_tcb, 1, Rights::READ);
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

    copy(endpoint_slot, client_tcb, 1, Rights::WRITE);
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
    write(ROOT_READY);

    loop {
        let message = unsafe { vifast_user::recv_message(fault_endpoint_slot, 0) };
        if message.error == KError::Ok {
            write_fault_addr(message.data[0]);
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let _ = vifast_user::debug_write(ROOT_PANIC.as_ptr() as u64, ROOT_PANIC.len() as u64);
    vifast_user::exit()
}
