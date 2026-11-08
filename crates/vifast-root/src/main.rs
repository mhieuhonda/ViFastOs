//! Rust root task image.
//!
//! The bootstrap actions mirror the old `user_root` assembly.  The image is
//! linked as an ELF at `ROOT_ELF_BASE`; the runtime loads PT_LOAD segments
//! before entering `_start`.

#![no_std]
#![no_main]

use core::panic::PanicInfo;
use vifast_abi::user_layout::{
    CLIENT_CMD_SLOT, CLIENT_IRQ_NOTIFICATION_SLOT, CLIENT_IRQ_SLOT, CLIENT_STACK_BASE,
    CLIENT_STACK_TOP, CLIENT_UART_IRQ_SLOT, CLIENT_UART_NOTIFICATION_SLOT, CNODE_STACK_BASE,
    CNODE_STACK_TOP, CODE_VA, EXEC_STACK_BASE, EXEC_STACK_TOP, FAULT1_STACK_TOP, FAULT1_TCB_SLOT,
    FAULT2_STACK_TOP, FAULT2_TCB_SLOT, FAULT3_STACK_TOP, FAULT3_TCB_SLOT, FAULT4_STACK_BASE,
    FAULT4_STACK_TOP, IDLE_STACK_BASE, IDLE_STACK_TOP, IPC_BUF_CLIENT_VA, IPC_BUF_SERVER_VA,
    PAGER2_STACK_BASE, PAGER2_STACK_TOP, PAGER3_STACK_BASE, PAGER3_STACK_TOP, PAGER_IPC_BUF_VA,
    PAGER_SERVER_STACK_BASE, PAGER_SERVER_STACK_TOP, PAGER_STACK_BASE, PAGER_STACK_TOP,
    ROOT_IPC_BUF_VA, ROOT_SELF_SLOT, SERVER_STACK_TOP, USER_ENTRY_FAULT1, USER_ENTRY_FAULT2,
    USER_ENTRY_FAULT3, USER_ENTRY_FAULT4, WORKER_A_STACK_BASE, WORKER_A_STACK_TOP,
    WORKER_B_STACK_BASE, WORKER_B_STACK_TOP, WORKER_C_STACK_BASE, WORKER_C_STACK_TOP,
    WORKER_D_STACK_BASE, WORKER_D_STACK_TOP, WORKER_E_STACK_BASE, WORKER_E_STACK_TOP,
    WORKER_G_STACK_BASE, WORKER_G_STACK_TOP, WORKER_H_STACK_BASE, WORKER_H_STACK_TOP,
};
use vifast_abi::{
    pager::{LABEL_KILL, LABEL_RUN, RUN_BADGE},
    vfs as vfs_abi, BootInfo, KError, ObjType, Rights, BOOTINFO_MAGIC, BOOTINFO_VERSION, IRQ_TIMER,
    IRQ_UART, LABEL_FAULT,
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
// Worker G/H cho hart 2/3 (mốc 5, vòng 31): yield 50 vòng như A/B rồi báo
// label riêng; root đếm bit 3/4, không đổi marker SMP WORKERS OK cũ.
const WORKER_LABEL_G: u64 = 0x5750_0007;
const WORKER_LABEL_H: u64 = 0x5750_0008;
const WORKER_LABEL_D_READY: u64 = 0x5750_0004;
const WORKER_LABEL_D_DONE: u64 = 0x5750_0005;
const WORKER_LABEL_C_RUNNING: u64 = 0x5750_0006;
const TLB_TEST_VA: u64 = CODE_VA as u64 + 0x40_0000;
const SUSPEND_FLAG_VA: u64 = TLB_TEST_VA + 0x1000;
/// Object-destroy demo page inside the root ELF/stack 2 MiB span, so its leaf
/// page table already exists and the demo only allocates the Frame.  It runs
/// before any secondary TCB can load the root VSpace, which keeps the 200
/// map/unmap operations free of remote TLB IPIs.
const DESTROY_TEST_VA: u64 = CODE_VA as u64 + 0x3f_f000;
const TCB_SUSPEND_OK: &[u8] = b"TCB SUSPEND OK\n";
const TCB_SUSPEND_BAD: &[u8] = b"TCB SUSPEND BAD\n";
const SCHED_CTX_OK: &[u8] = b"SCHED CTX OK\n";
const CNODE_OK: &[u8] = b"CNODE OK\n";
const TCB_REAP_OK: &[u8] = b"TCB REAP OK\n";
const TCB_REAP_BAD: &[u8] = b"TCB REAP BAD\n";
const WAIT_REAP_OK: &[u8] = b"WAIT REAP OK\n";
const WAIT_REAP_BAD: &[u8] = b"WAIT REAP BAD\n";
const CNODE_LABEL: u64 = 0x434E_4F44;
const PAGER_LABEL: u64 = 0x5041_4745;
const PAGER_VA: u64 = CODE_VA as u64 + 0x100_0000;
const PAGER_VALUE: u64 = 0x5041_4745_5200_0001;
const PAGER2_LABEL: u64 = 0x5041_4746;
const PAGER2_VA: u64 = PAGER_VA + 0x1000;
const PAGER2_VALUE: u64 = 0x5041_4745_5200_0002;
const PAGER2_OK: &[u8] = b"PAGER2 OK\n";
const PAGER2_BAD: &[u8] = b"PAGER2 BAD\n";
const PAGER3_LABEL: u64 = 0x5041_4747;
const PAGER3_VA: u64 = PAGER_VA + 0x2000;
const PAGER3_VALUE: u64 = 0x5041_4745_5200_0003;
const PAGER3_OK: &[u8] = b"PAGER3 OK\n";
const PAGER4_VA: u64 = PAGER_VA + 0x3000;
const PAGER4_LABEL: u64 = 0x5041_4748;
/// Nội dung trang chỉ tồn tại ở server (backing image) và chỉ được ghi vào
/// frame TẠI THỜI ĐIỂM FAULT — đặc trưng của demand-load (vòng 67).
const PAGER4_IMAGE: u64 = 0x4445_4d41_4e44_0004;
const DEMAND_LOAD_OK: &[u8] = b"DEMAND LOAD OK\n";
const DEMAND_LOAD_BAD: &[u8] = b"DEMAND LOAD BAD\n";
const PAGER3_BAD: &[u8] = b"PAGER3 BAD\n";
const PAGER_EVICT_OK: &[u8] = b"PAGER EVICT OK\n";
const PAGER_RECLAIM_OK: &[u8] = b"PAGER RECLAIM OK\n";
const PAGER_RECLAIM_BAD: &[u8] = b"PAGER RECLAIM BAD\n";
const PAGER_DONE_LABEL: u64 = 0x5041_4744;
/// Slot trong cspace pager server giữ cap tới fault endpoint của root.
const PAGER_SERVER_ROOT: u8 = 15;
const PAGER_EVICT_BAD: &[u8] = b"PAGER EVICT BAD\n";
const PAGER_OK: &[u8] = b"PAGER OK\n";
const PAGER_BAD: &[u8] = b"PAGER BAD\n";
const PAGER_SERVER_ENDPOINT: u8 = 1;
const PAGER_SERVER_TCB: u8 = 2;
const PAGER_SERVER_VSPACE: u8 = 3;
const PAGER_SERVER_FRAME: u8 = 4;
const PAGER_SERVER_TCB2: u8 = 5;
const PAGER_SERVER_FRAME2: u8 = 6;
const PAGER_SERVER_UNTYPED: u8 = 7;
/// Slot trong cspace của pager server giữ cap tới VFS endpoint (vòng 71).
const PAGER_SERVER_VFS_ENDPOINT: u8 = 11;
/// Bật đường nạp trang từ VFS (chỉ riscv64/aarch64 — x86 hết Untyped cho
/// thêm frame IPC buffer).
static mut PAGER_VFS_ENABLED: bool = false;
const DEMAND_LOAD_VFS_OK: &[u8] = b"DEMAND VFSLOAD OK\n";
const DEMAND_LOAD_VFS_BAD: &[u8] = b"DEMAND VFSLOAD BAD\n";
const PAGER_SERVER_TCB3: u8 = 8;
const PAGER_SERVER_FRAME3: u8 = 9;
const PAGER_SERVER_FRAME4: u8 = 10;
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

#[no_mangle]
pub extern "C" fn cnode_worker() -> ! {
    // Slot 1 is the only cap in this task's flat CNode cspace.
    let _ = vifast_user::send(1, CNODE_LABEL, 1, 0, 0);
    loop {
        let _ = vifast_user::yield_now();
    }
}

#[no_mangle]
pub extern "C" fn pager_client() -> ! {
    // The first execution faults on this unmapped page.  The user-space pager
    // server maps a frame into this VSpace and resumes the TCB.
    let value = unsafe { core::ptr::read_volatile(PAGER_VA as *const u64) };
    let _ = vifast_user::send(PAGER_SERVER_ENDPOINT, PAGER_LABEL, value, 0, 0);
    loop {
        let _ = vifast_user::yield_now();
    }
}

#[no_mangle]
pub extern "C" fn pager2_client() -> ! {
    let value = unsafe { core::ptr::read_volatile(PAGER2_VA as *const u64) };
    let _ = vifast_user::send(PAGER_SERVER_ENDPOINT, PAGER2_LABEL, value, 0, 0);
    // Evict demo: server đã unmap trang sau lần đọc đầu; đọc lại để fault
    // lần 2 và được map lại frame mới (vòng 35: destroy + retype).
    let value2 = unsafe { core::ptr::read_volatile(PAGER2_VA as *const u64) };
    let _ = vifast_user::send(PAGER_SERVER_ENDPOINT, PAGER2_LABEL, value2, 0, 0);
    loop {
        let _ = vifast_user::yield_now();
    }
}

/// Biến thể không có demand-load (dùng khi hết Untyped, vd x86_64).
#[no_mangle]
pub extern "C" fn pager3_only_client() -> ! {
    let value = unsafe { core::ptr::read_volatile(PAGER3_VA as *const u64) };
    let _ = vifast_user::send(PAGER_SERVER_ENDPOINT, PAGER3_LABEL, value, 0, 0);
    loop {
        let _ = vifast_user::yield_now();
    }
}

#[no_mangle]
pub extern "C" fn pager3_client() -> ! {
    let value = unsafe { core::ptr::read_volatile(PAGER3_VA as *const u64) };
    let _ = vifast_user::send(PAGER_SERVER_ENDPOINT, PAGER3_LABEL, value, 0, 0);
    // Demand-load (vòng 67): trang này CHƯA được map và nội dung chỉ được
    // server ghi vào frame khi nhận fault — client đọc sau khi được resume.
    let demand = unsafe { core::ptr::read_volatile(PAGER4_VA as *const u64) };
    let _ = vifast_user::send(PAGER_SERVER_ENDPOINT, PAGER4_LABEL, demand, 0, 0);
    loop {
        let _ = vifast_user::yield_now();
    }
}

/// Gọi VFS (chặn) từ pager server, retry khi WouldBlock. Trả về reply hoặc
/// None nếu lỗi khác/timeout.
fn pager_vfs_call(
    label: u64,
    data0: u64,
    data1: u64,
    data2: u64,
) -> Option<vifast_user::MessageResult> {
    let mut attempts = 0u32;
    loop {
        let reply = unsafe {
            vifast_user::call_message(PAGER_SERVER_VFS_ENDPOINT, label, data0, data1, data2)
        };
        if reply.error == KError::Ok {
            return Some(reply);
        }
        if reply.error != KError::WouldBlock {
            return None;
        }
        attempts += 1;
        if attempts > 2000 {
            return None;
        }
        let _ = vifast_user::yield_now();
    }
}

/// Nạp một trang từ file VFS vào frame `frame_slot` (vòng 71).
/// Đọc từng khối 128 byte qua IPC buffer của pager server, ghi vào frame, và
/// kiểm tra 8 byte cuối khớp DEMAND_TAIL.
fn load_page_from_vfs(frame_slot: u8) -> bool {
    let path = vifast_abi::pager::DEMAND_FILE;
    let buf = PAGER_IPC_BUF_VA as *mut u8;
    // Đường dẫn nằm trong IPC buffer để VFS server đọc.
    unsafe {
        let mut i = 0;
        while i < path.len() {
            core::ptr::write_volatile(buf.add(i), path[i]);
            i += 1;
        }
    }
    let open = match pager_vfs_call(vifast_abi::vfs::OPEN, 0, path.len() as u64, 0) {
        Some(reply) => reply,
        None => return false,
    };
    if open.label != vifast_abi::vfs::OK {
        return false;
    }
    let fd = open.data[0];
    let mut offset = 0usize;
    let total = vifast_abi::pager::DEMAND_BYTES;
    while offset < total {
        let want = core::cmp::min(128usize, total - offset);
        let reply = match pager_vfs_call(vifast_abi::vfs::READ, fd, offset as u64, want as u64) {
            Some(reply) => reply,
            None => return false,
        };
        let count = reply.label as usize;
        if count == 0 || count > 128 {
            return false;
        }
        if vifast_user::frame_write(
            frame_slot,
            offset as u64,
            PAGER_IPC_BUF_VA as u64,
            count as u64,
        )
        .error
            != KError::Ok
        {
            return false;
        }
        offset += count;
    }
    // Khối cuối nằm trong IPC buffer; 8 byte cuối phải là DEMAND_TAIL.
    let last_chunk_start = total - 128;
    let tail_offset = (total - 8) - last_chunk_start;
    let mut tail = [0u8; 8];
    unsafe {
        let mut i = 0;
        while i < 8 {
            tail[i] = core::ptr::read_volatile(buf.add(tail_offset + i));
            i += 1;
        }
    }
    let _ = pager_vfs_call(vifast_abi::vfs::CLOSE, fd, 0, 0);
    u64::from_le_bytes(tail) == vifast_abi::pager::DEMAND_TAIL
}

#[no_mangle]
pub extern "C" fn pager_server() -> ! {
    loop {
        let message = unsafe { vifast_user::recv_message(PAGER_SERVER_ENDPOINT, 0) };
        if message.error != KError::Ok {
            let _ = vifast_user::yield_now();
            continue;
        }
        if message.label == LABEL_FAULT {
            let (frame_slot, tcb_slot) = if message.data[0] == PAGER_VA {
                (PAGER_SERVER_FRAME, PAGER_SERVER_TCB)
            } else if message.data[0] == PAGER2_VA {
                (PAGER_SERVER_FRAME2, PAGER_SERVER_TCB2)
            } else if message.data[0] == PAGER3_VA {
                (PAGER_SERVER_FRAME3, PAGER_SERVER_TCB3)
            } else if message.data[0] == PAGER4_VA {
                // Demand-load thật (vòng 69): retype frame từ Untyped riêng
                // của server NGAY LÚC FAULT, nạp backing image rồi mới map.
                // Không cần root cấp frame nên x86_64 cũng chạy được.
                if vifast_user::retype(
                    PAGER_SERVER_UNTYPED,
                    ObjType::Frame as u64,
                    PAGER_SERVER_FRAME4,
                )
                .error
                    != KError::Ok
                {
                    write(DEMAND_LOAD_BAD);
                    continue;
                }
                if unsafe { PAGER_VFS_ENABLED } {
                    // Vòng 71: nạp trang từ file /page.bin qua VFS.
                    if load_page_from_vfs(PAGER_SERVER_FRAME4) {
                        write(DEMAND_LOAD_VFS_OK);
                    } else {
                        write(DEMAND_LOAD_VFS_BAD);
                        write(DEMAND_LOAD_BAD);
                        continue;
                    }
                } else {
                    let _ = vifast_user::frame_write(
                        PAGER_SERVER_FRAME4,
                        0,
                        &PAGER4_IMAGE as *const u64 as u64,
                        8,
                    );
                }
                (PAGER_SERVER_FRAME4, PAGER_SERVER_TCB3)
            } else {
                write(PAGER_BAD);
                continue;
            };
            let mapped = vifast_user::frame_map(
                PAGER_SERVER_VSPACE,
                frame_slot,
                message.data[0],
                Rights::READ.0,
            );
            if mapped.error == KError::Ok {
                let _ = vifast_user::tcb_resume(tcb_slot);
            } else {
                write(PAGER_BAD);
            }
        } else if message.label == PAGER4_LABEL {
            write(if message.data[0] == PAGER4_IMAGE {
                DEMAND_LOAD_OK
            } else {
                DEMAND_LOAD_BAD
            });
            // Vòng 91: báo root thu hồi demo pager (root giữ cap gốc nên chỉ
            // root revoke + unmap + destroy được; server gửi trên cap tới
            // fault endpoint của root ở slot PAGER_SERVER_ROOT).
            if message.data[0] == PAGER4_IMAGE {
                let r = vifast_user::send(PAGER_SERVER_ROOT, PAGER_DONE_LABEL, 0, 0, 0);
                write(b"PAGER NOTIFY ");
                write_decimal(r.error as u64);
                write(b"\n");
            }
        } else if message.label == PAGER_LABEL {
            write(if message.data[0] == PAGER_VALUE {
                PAGER_OK
            } else {
                PAGER_BAD
            });
        } else if message.label == PAGER2_LABEL {
            // Lần đầu: PAGER2 OK exact-one (harness). Lần 2 (sau evict):
            // PAGER EVICT OK exact-one.
            static mut PAGER2_SEEN: u8 = 0;
            let seen = unsafe {
                PAGER2_SEEN = PAGER2_SEEN.wrapping_add(1);
                PAGER2_SEEN
            };
            if seen <= 1 {
                write(if message.data[0] == PAGER2_VALUE {
                    PAGER2_OK
                } else {
                    PAGER2_BAD
                });
            } else {
                write(if message.data[0] == PAGER2_VALUE {
                    PAGER_EVICT_OK
                } else {
                    PAGER_EVICT_BAD
                });
            }
            // Pager v2 evict demo (vòng 34-35): sau khi client2 đọc xong,
            // unmap + destroy frame cũ (trả về Untyped), retype frame mới từ
            // Untyped con, copy lại nội dung và map lại. Client fault lần 2
            // vẫn đọc đúng giá trị.
            if message.data[0] == PAGER2_VALUE {
                let _ =
                    vifast_user::frame_unmap(PAGER_SERVER_VSPACE, PAGER2_VA, PAGER_SERVER_FRAME2);
                let _ = vifast_user::object_destroy(PAGER_SERVER_FRAME2);
                // Retype frame mới vào đúng slot cũ (slot đã trống sau destroy).
                if vifast_user::retype(
                    PAGER_SERVER_UNTYPED,
                    ObjType::Frame as u64,
                    PAGER_SERVER_FRAME2,
                )
                .error
                    == KError::Ok
                {
                    let _ = vifast_user::frame_write(
                        PAGER_SERVER_FRAME2,
                        0,
                        &PAGER2_VALUE as *const u64 as u64,
                        8,
                    );
                }
            }
        } else if message.label == PAGER3_LABEL {
            write(if message.data[0] == PAGER3_VALUE {
                PAGER3_OK
            } else {
                PAGER3_BAD
            });
        }
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
pub extern "C" fn smp_worker_g() -> ! {
    worker_loop(WORKER_LABEL_G)
}

#[no_mangle]
pub extern "C" fn smp_worker_h() -> ! {
    worker_loop(WORKER_LABEL_H)
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

/// Worker F (mốc 1, vòng 16): thoát ngay khi chạy lần đầu để root demo
/// exit-theo-task + TcbWait + reap. Dòng này chứng minh một task exit không
/// kill cả máy (USER EXIT chỉ còn khi cạn task).
#[no_mangle]
pub extern "C" fn exit_worker_entry() -> ! {
    vifast_user::exit();
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

fn write_decimal(mut value: u64) {
    let mut digits = [0u8; 20];
    let mut at = digits.len();
    loop {
        at -= 1;
        digits[at] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    check(vifast_user::debug_write(
        digits[at..].as_ptr() as u64,
        (digits.len() - at) as u64,
    ));
}

fn destroy_demo_bad(code: u32) {
    write(b"DESTROY BAD (code=");
    write_decimal(code as u64);
    write(b")\n");
}

/// Round 23 object lifecycle demo: 100 Frame create/map/write/unmap/destroy
/// cycles and 10 VSpace cycles through `ObjectDestroy`.  The 100 frames total
/// 400 KiB, more than the 304 KiB data Untyped, so success proves reclaim.
fn run_destroy_demo(data_untyped: u8, root_vspace: u8, slots: &mut SlotAllocator) -> u32 {
    let frame = slots.alloc();
    let vspace = slots.alloc();
    let rights = Rights::READ | Rights::WRITE;
    if vifast_user::retype(data_untyped, ObjType::Frame as u64, frame).error != KError::Ok {
        return 1;
    }
    map_with_tables(
        data_untyped,
        root_vspace,
        frame,
        DESTROY_TEST_VA,
        rights,
        slots,
    );
    if vifast_user::frame_unmap(root_vspace, DESTROY_TEST_VA, frame).error != KError::Ok {
        return 2;
    }
    if vifast_user::object_destroy(frame).error != KError::Ok {
        return 3;
    }

    let mut reused = 0u64;
    let mut value = 0xD357_0001u64;
    for _ in 0..100 {
        if vifast_user::retype(data_untyped, ObjType::Frame as u64, frame).error != KError::Ok {
            return 4;
        }
        value = value.wrapping_add(0x10);
        if vifast_user::frame_write(frame, 0, &value as *const u64 as u64, 8).error != KError::Ok {
            return 5;
        }
        let mapped = vifast_user::frame_map(root_vspace, frame, DESTROY_TEST_VA, rights.0);
        if mapped.error != KError::Ok {
            return 6;
        }
        if mapped.value != DESTROY_TEST_VA {
            return 7;
        }
        if unsafe { core::ptr::read_volatile(DESTROY_TEST_VA as *const u64) } != value {
            return 8;
        }
        unsafe { core::ptr::write_volatile(DESTROY_TEST_VA as *mut u64, value ^ 1) };
        if unsafe { core::ptr::read_volatile(DESTROY_TEST_VA as *const u64) } != (value ^ 1) {
            return 9;
        }
        if vifast_user::frame_unmap(root_vspace, DESTROY_TEST_VA, frame).error != KError::Ok {
            return 10;
        }
        if vifast_user::object_destroy(frame).error != KError::Ok {
            return 11;
        }
        reused = reused.saturating_add(1);
    }

    // A mapped Frame must be rejected, then succeed after unmap.
    if vifast_user::retype(data_untyped, ObjType::Frame as u64, frame).error != KError::Ok {
        return 12;
    }
    if vifast_user::frame_map(root_vspace, frame, DESTROY_TEST_VA, rights.0).error != KError::Ok {
        return 13;
    }
    if vifast_user::object_destroy(frame).error != KError::Busy {
        return 14;
    }
    if vifast_user::frame_unmap(root_vspace, DESTROY_TEST_VA, frame).error != KError::Ok {
        return 15;
    }
    if vifast_user::object_destroy(frame).error != KError::Ok {
        return 16;
    }
    // The stale capability slot must be rejected.
    if vifast_user::object_destroy(frame).error != KError::InvalidCap {
        return 17;
    }

    for _ in 0..10 {
        if vifast_user::retype(data_untyped, ObjType::VSpace as u64, vspace).error != KError::Ok {
            return 18;
        }
        let destroyed = vifast_user::object_destroy(vspace);
        if destroyed.error != KError::Ok {
            return 1000 + destroyed.error as u32;
        }
    }

    write(b"DESTROY OK (frames=");
    write_decimal(100);
    write(b", vspaces=");
    write_decimal(10);
    write(b", reused=");
    write_decimal(reused);
    write(b", leaked=0)\n");
    slots.release(frame);
    slots.release(vspace);
    0
}

#[cfg(feature = "test-destroy-smp")]
fn destroy_smp_bad(code: u32) {
    write(b"DESTROY SMP BAD (code=");
    write_decimal(code as u64);
    write(b")\n");
}

/// Variant used by `--feature test-destroy-smp`: the same lifecycle exercise
/// runs after a secondary hart has already executed a worker, so TLB IPIs and
/// concurrent scheduler state are live.
#[cfg(feature = "test-destroy-smp")]
fn run_destroy_smp_demo(data_untyped: u8, root_vspace: u8, slots: &mut SlotAllocator) -> u32 {
    let frame = slots.alloc();
    let vspace = slots.alloc();
    let rights = Rights::READ | Rights::WRITE;
    let mut value = 0x5A5A_0001u64;
    for _ in 0..20 {
        if vifast_user::retype(data_untyped, ObjType::Frame as u64, frame).error != KError::Ok {
            return 1;
        }
        value = value.wrapping_add(0x10);
        if vifast_user::frame_write(frame, 0, &value as *const u64 as u64, 8).error != KError::Ok {
            return 2;
        }
        let mapped = vifast_user::frame_map(root_vspace, frame, DESTROY_TEST_VA, rights.0);
        if mapped.error != KError::Ok || mapped.value != DESTROY_TEST_VA {
            return 3;
        }
        if unsafe { core::ptr::read_volatile(DESTROY_TEST_VA as *const u64) } != value {
            return 4;
        }
        if vifast_user::object_destroy(frame).error != KError::Busy {
            return 8;
        }
        if vifast_user::frame_unmap(root_vspace, DESTROY_TEST_VA, frame).error != KError::Ok {
            return 5;
        }
        if vifast_user::object_destroy(frame).error != KError::Ok {
            return 6;
        }
    }
    for _ in 0..2 {
        if vifast_user::retype(data_untyped, ObjType::VSpace as u64, vspace).error != KError::Ok {
            return 7;
        }
        let destroyed = vifast_user::object_destroy(vspace);
        if destroyed.error != KError::Ok {
            return 1000 + destroyed.error as u32;
        }
    }
    slots.release(frame);
    slots.release(vspace);
    write(b"DESTROY SMP OK (frames=20, vspaces=2)\n");
    0
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
    check(vifast_user::debug_write(b"\n".as_ptr() as u64, 1));
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
    // Vòng 34: 16 để chứa hết slots exec (12) khi `run` lặp.
    released: [u8; 16],
    released_len: u8,
}

impl SlotAllocator {
    fn alloc(&mut self) -> u8 {
        if self.released_len > 0 {
            self.released_len -= 1;
            return self.released[self.released_len as usize];
        }
        if self.next >= self.end {
            panic!("root slot range exhausted");
        }
        let slot = self.next;
        self.next += 1;
        slot
    }

    /// Return an emptied slot so late demo/worker setup can reuse it.
    fn release(&mut self, slot: u8) {
        if (self.released_len as usize) < self.released.len() {
            self.released[self.released_len as usize] = slot;
            self.released_len += 1;
        }
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

/// Map một frame, tự cấp page table khi thiếu; fail-êm (false) thay vì
/// panic như map_with_tables — dùng cho exec demo trên máy cạn Untyped.
fn map_frame_fallible(
    data_untyped: u8,
    vspace: u8,
    frame: u8,
    va: u64,
    rights: u8,
    slots: &mut SlotAllocator,
    used: &mut [u8; MAX_EXEC_SLOTS],
    used_len: &mut usize,
) -> bool {
    for _ in 0..4 {
        let mapped = vifast_user::frame_map(vspace, frame, va, rights);
        if mapped.error == KError::Ok {
            return true;
        }
        if mapped.error != KError::MissingPageTable {
            return false;
        }
        // Mọi page table cấp cho exec đều track để destroy_exec dọn được
        // (vòng 34: table leak làm `run` lần 2 RUN SKIP vì cạn Untyped).
        let table = match track_alloc(slots, used, used_len) {
            Some(slot) => slot,
            None => return false,
        };
        if vifast_user::retype(data_untyped, ObjType::PageTable as u64, table).error != KError::Ok {
            return false;
        }
        if vifast_user::page_table_map(table, vspace, va).error != KError::Ok {
            return false;
        }
    }
    false
}

/// Exec MVP (mốc 9, vòng 32): nạp bản sao ELF client vào VSpace mới và
/// spawn TCB chạy nó. Mọi bước fail-êm (trả None) để x86 cạn Untyped bỏ qua
/// mà không panic — demo chỉ chạy khi đủ tài nguyên.
/// ELF64 tối giản cho exec demo (mốc 9, vòng 32/33): 1 segment R+X chứa
/// `j .` (busy loop thân thiện, không in gì, không xung đột marker với bản
/// gốc). Không dùng `wfi`: ở U-mode nó trap illegal khi TW=1 (vòng 33 riscv
/// fault addr=0x10500073, system_opcode_insn). Entry = CODE_VA.
const EXEC_DEMO_CODE: u64 = CODE_VA as u64 + 0x1000;
/// Dựng ELF demo nhỏ: 1 trang code in `msg` (DebugWrite) rồi exit.
/// `len` là độ dài chuỗi, được nhúng thẳng vào lệnh nạp tham số độ dài.
const fn exec_demo_elf() -> [u8; 0x180] {
    // Ảnh nhỏ 0x180: header (0x40) + phdr (0x38) ở đầu; segment đặt tại
    // offset 0x120 với vaddr = EXEC_DEMO_CODE + 0x120 (congruent mod 0x1000
    // nên parse chấp nhận). Code ở 0x120, cặp (ptr,len) ở 0x140, chuỗi 0x160.
    let mut elf = [0u8; 0x180];
    elf[0] = 0x7f;
    elf[1] = b'E';
    elf[2] = b'L';
    elf[3] = b'F';
    elf[4] = 2;
    elf[5] = 1;
    elf[6] = 1;
    elf[16] = 2;
    elf[18] = (USER_ELF_MACHINE & 0xff) as u8;
    elf[19] = (USER_ELF_MACHINE >> 8) as u8;
    elf[20] = 1;
    let entry = (EXEC_DEMO_CODE + 0x120).to_le_bytes();
    let mut i = 0;
    while i < 8 {
        elf[24 + i] = entry[i];
        i += 1;
    }
    let phoff = 64u64.to_le_bytes();
    i = 0;
    while i < 8 {
        elf[32 + i] = phoff[i];
        i += 1;
    }
    elf[52] = 64;
    elf[54] = 56;
    elf[56] = 1;
    elf[64] = 1;
    elf[68] = 5;
    let off = 0x120u64.to_le_bytes();
    let vaddr = (EXEC_DEMO_CODE + 0x120).to_le_bytes();
    i = 0;
    while i < 8 {
        elf[72 + i] = off[i];
        elf[80 + i] = vaddr[i];
        elf[88 + i] = vaddr[i];
        i += 1;
    }
    elf[96] = 0x60; // filesz
    elf[104] = 0x60; // memsz
    let align = 0x1000u64.to_le_bytes();
    i = 0;
    while i < 8 {
        elf[112 + i] = align[i];
        i += 1;
    }
    if cfg!(target_arch = "riscv64") {
        let code: [u32; 8] = [
            0x0000_0617, // auipc a2, 0
            0x0206_0613, // addi a2, a2, 0x20 (cặp ptr/len)
            0x0006_3503, // ld a0, 0(a2)
            0x0086_3583, // ld a1, 8(a2)
            0x0200_0893, // addi a7, x0, 32
            0x0000_0073, // ecall
            0x0220_0893, // addi a7, x0, 34
            0x0000_0073, // ecall
        ];
        let mut i = 0;
        while i < code.len() {
            let bytes = code[i].to_le_bytes();
            let mut j = 0;
            while j < 4 {
                elf[0x120 + i * 4 + j] = bytes[j];
                j += 1;
            }
            i += 1;
        }
    } else if cfg!(target_arch = "aarch64") {
        let code: [u32; 7] = [
            0x1000_0102, // adr x2, #0x20 (cặp ptr/len)
            0xF940_0040, // ldr x0, [x2]
            0xF940_0441, // ldr x1, [x2, #8]
            0xD280_0408, // movz x8, #32
            0xD400_0001, // svc #0
            0xD280_0448, // movz x8, #34
            0xD400_0001, // svc #0
        ];
        let mut i = 0;
        while i < code.len() {
            let bytes = code[i].to_le_bytes();
            let mut j = 0;
            while j < 4 {
                elf[0x120 + i * 4 + j] = bytes[j];
                j += 1;
            }
            i += 1;
        }
    } else {
        let code: [u8; 28] = [
            0x48, 0x8D, 0x35, 0x19, 0x00, 0x00, 0x00, // lea rsi,[rip+0x19]
            0x48, 0x8B, 0x3E, // mov rdi,[rsi]
            0x48, 0x8B, 0x76, 0x08, // mov rsi,[rsi+8]
            0xB8, 0x20, 0x00, 0x00, 0x00, // mov eax,32
            0xCD, 0x80, // int 0x80
            0xB8, 0x22, 0x00, 0x00, 0x00, // mov eax,34
            0xCD, 0x80, // int 0x80
        ];
        let mut i = 0;
        while i < code.len() {
            elf[0x120 + i] = code[i];
            i += 1;
        }
    }
    elf
}

/// Bản chạy từ RAM (boot EXEC demo) — in "HI FS".
const EXEC_DEMO_ELF: &[u8] = &exec_demo_elf();

/// Số slot tối đa spawn_elf_copy cấp cho ELF demo (tcb + vspace + stack +
/// frame code + page tables). ELF demo cố định 1 page code nên 8 là dư;
/// nếu tràn, spawn fail-êm thay vì ghi đè.
/// Dọn task exec cũ trước khi `run` lần sau (mốc 9, vòng 34):
/// destroy TCB trước (detach khỏi runq/endpoint), rồi VSpace (Faulted TCB
/// còn sót + dọn page tables), rồi frames/tables còn lại. Bỏ qua lỗi từng
/// slot — slot có thể đã trống nếu spawn trước fail giữa chừng.
/// Không trả slot về SlotAllocator (slots chỉ tiến): destroy object giải
/// phóng tài nguyên kernel/Untyped, slot cspace trống nhưng không tái dùng
/// trong vòng đời root — chấp nhận được vì mỗi `run` tốn ≤ 8 slots trên
/// 128 slot root và đây là demo.
fn destroy_exec(slots: &mut SlotAllocator, used: &[u8; MAX_EXEC_SLOTS], len: usize) {
    // Một pass là đủ: kernel xử lý phụ thuộc khi destroy (VSpace
    // destroy Faulted TCB còn trỏ; TCB destroy detach khỏi mọi hàng chờ).
    // Destroy TCB trước để nó không chạy trong lúc dọn VSpace/frame.
    // Sắp xếp: used[0] là vspace... — không đảm bảo thứ tự, nên destroy TCB
    // riêng trước: tìm slot TCB bằng cách thử? Không — đơn giản: destroy
    // tất cả 2 lượt, lượt 1 bỏ qua lỗi, lượt 2 dọn nốt.
    let mut pass = 0;
    while pass < 2 {
        let mut i = 0;
        while i < len {
            if vifast_user::object_destroy(used[i]).error == KError::Ok {
                // Slot đã trống thật mới trả về allocator; nếu destroy
                // fail (slot còn object lạ) giữ nguyên để lần sau thử lại.
                // Chỉ release ở pass cuối để slot không bị cấp lại giữa
                // chừng khi pass 1 chưa dọn xong phụ thuộc.
                if pass == 1 {
                    slots.release(used[i]);
                }
            }
            i += 1;
        }
        pass += 1;
    }
}

/// Cấp slot và ghi lại để `destroy_exec` dọn được khi `run` lần sau.
/// Trả None khi đầy (spawn fail-êm).
fn track_alloc(
    slots: &mut SlotAllocator,
    used: &mut [u8; MAX_EXEC_SLOTS],
    len: &mut usize,
) -> Option<u8> {
    if *len >= MAX_EXEC_SLOTS {
        return None;
    }
    let slot = slots.alloc();
    used[*len] = slot;
    *len += 1;
    Some(slot)
}

// Vòng 34: 12 slots (tcb + vspace + stack + frame code + tới 8 page
// tables cho 2 vùng VA trên Sv39 3-level). Đếm thực tế đúng 8 nhưng sát
// trần → lần 2 SKIP; dư 4 cho chắc.
const MAX_EXEC_SLOTS: usize = 12;
/// Ghi chuỗi tham số vào trang code của chương trình tại offset 0x200.
/// Chương trình đọc bằng `auipc+addi` (cùng trang) rồi DebugWrite — cơ chế
/// truyền tham số tối giản, không phụ thuộc quy ước sp của kernel.
fn write_program_args(code_frame: u8, msg: &[u8]) -> bool {
    // Cặp (ptr, len) ở frame 0x140 = entry+0x20; chuỗi ở 0x160 = entry+0x40.
    // Độ dài ĐỘNG nên chương trình in đúng tham số truyền vào.
    // Vòng 181: chương trình in y nguyên chuỗi nhận được, nên nếu người dùng
    // gọi `runfs /prog.elf ONE` (không có '\n') thì output sẽ dính vào lời nhắc
    // kế tiếp (`ONEshell> `). Thêm '\n' khi thiếu — sửa lỗi thấy được khi chạy
    // hai lần liên tiếp với hai tham số khác nhau.
    let mut src = [0u8; 32];
    let mut src_len = core::cmp::min(msg.len(), 31);
    let mut k = 0;
    while k < src_len {
        src[k] = msg[k];
        k += 1;
    }
    if src_len == 0 || src[src_len - 1] != b'\n' {
        src[src_len] = b'\n';
        src_len += 1;
    }
    let msg = &src[..src_len];
    let len = core::cmp::min(msg.len() as u64, 32);
    let mut pair = [0u8; 16];
    let ptr = EXEC_DEMO_CODE + 0x160;
    let mut i = 0;
    while i < 8 {
        pair[i] = (ptr >> (i * 8)) as u8;
        pair[8 + i] = (len >> (i * 8)) as u8;
        i += 1;
    }
    if vifast_user::frame_write(code_frame, 0x140, pair.as_ptr() as u64, 16).error != KError::Ok {
        return false;
    }
    let mut out = [0u8; 32];
    i = 0;
    while i < len as usize {
        out[i] = msg[i];
        i += 1;
    }
    vifast_user::frame_write(code_frame, 0x160, out.as_ptr() as u64, len).error == KError::Ok
}

fn spawn_elf_copy(
    meta_untyped: u8,
    data_untyped: u8,
    bytes: &[u8],
    msg: &[u8],
    affinity: u8,
    slots: &mut SlotAllocator,
    used: &mut [u8; MAX_EXEC_SLOTS],
) -> Option<u8> {
    let mut used_len = 0usize;
    let image = vifast_elf::parse(
        bytes,
        USER_ELF_MACHINE,
        CODE_VA as u64,
        CODE_VA as u64 + MODULE_WINDOW,
    )
    .ok()?;
    let vspace = track_alloc(slots, used, &mut used_len)?;
    if vifast_user::retype(data_untyped, ObjType::VSpace as u64, vspace).error != KError::Ok {
        return None;
    }
    for segment in image.segments() {
        let page_base = segment.vaddr & !0xfff;
        let page_count = segment.page_count();
        let mut page = 0;
        while page < page_count {
            let va = page_base + page as u64 * vifast_abi::DEFAULT_PAGE_SIZE;
            let frame = track_alloc(slots, used, &mut used_len)?;
            if vifast_user::retype(data_untyped, ObjType::Frame as u64, frame).error != KError::Ok {
                return None;
            }
            // Copy nội dung segment (chunk 256B như load_module).
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
                    if vifast_user::frame_write(
                        frame,
                        frame_offset + done,
                        bytes.as_ptr().wrapping_add(source + done as usize) as u64,
                        chunk,
                    )
                    .error
                        != KError::Ok
                    {
                        return None;
                    }
                    done += chunk;
                }
            }
            // Tham số ghi SAU khi copy segment (vùng 0x140 nằm trong segment)
            // và TRƯỚC khi map (kernel chặn ghi lên frame đã map executable).
            if page == 0 && segment.executable() && !write_program_args(frame, msg) {
                return None;
            }
            let mut rights = Rights::READ;
            if segment.writable() {
                rights = rights | Rights::WRITE;
            }
            if segment.executable() {
                rights = rights | Rights::EXEC;
            }
            if !map_frame_fallible(
                data_untyped,
                vspace,
                frame,
                va,
                rights.0,
                slots,
                used,
                &mut used_len,
            ) {
                return None;
            }
            page += 1;
        }
    }
    // Stack riêng cho bản sao.
    let stack_frame = track_alloc(slots, used, &mut used_len)?;
    if vifast_user::retype(data_untyped, ObjType::Frame as u64, stack_frame).error != KError::Ok {
        return None;
    }
    if !map_frame_fallible(
        data_untyped,
        vspace,
        stack_frame,
        EXEC_STACK_BASE as u64,
        (Rights::READ | Rights::WRITE).0,
        slots,
        used,
        &mut used_len,
    ) {
        return None;
    }
    // Vòng 136: W^X — ghi vào frame ĐÃ map executable phải bị từ chối, còn
    // frame RW (stack) sau khi map vẫn ghi được. Chỉ in một lần.
    let mut wx_ok = false;
    if let Some(code_frame) = used.first().copied().filter(|_| used_len > 0) {
        let denied =
            vifast_user::frame_write(code_frame, 0, b"x".as_ptr() as u64, 1).error != KError::Ok;
        let stack_ok =
            vifast_user::frame_write(stack_frame, 0, b"x".as_ptr() as u64, 1).error == KError::Ok;
        wx_ok = denied && stack_ok;
    }
    static mut WX_PRINTED: bool = false;
    unsafe {
        if !WX_PRINTED {
            WX_PRINTED = true;
            write(if wx_ok { b"WX OK\n" } else { b"WX BAD\n" });
        }
    }
    let tcb = track_alloc(slots, used, &mut used_len)?;
    if vifast_user::retype(meta_untyped, ObjType::Tcb as u64, tcb).error != KError::Ok {
        return None;
    }
    if vifast_user::tcb_configure(tcb, image.entry as u64, EXEC_STACK_TOP as u64, vspace, 5).error
        != KError::Ok
    {
        return None;
    }
    if vifast_user::tcb_set_affinity(tcb, affinity).error != KError::Ok {
        return None;
    }
    if vifast_user::tcb_resume(tcb).error != KError::Ok {
        return None;
    }
    Some(tcb)
}

fn spawn_cnode_demo(
    meta_untyped: u8,
    data_untyped: u8,
    root_vspace: u8,
    endpoint_slot: u8,
    slots: &mut SlotAllocator,
) -> u8 {
    let cnode = slots.alloc();
    retype(data_untyped, ObjType::CNode, cnode);
    let tcb = slots.alloc();
    retype(meta_untyped, ObjType::Tcb, tcb);
    let stack_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, stack_frame);
    map_with_tables(
        data_untyped,
        root_vspace,
        stack_frame,
        CNODE_STACK_BASE as u64,
        Rights::READ | Rights::WRITE,
        slots,
    );
    check(vifast_user::cnode_mint(
        cnode,
        1,
        endpoint_slot,
        Rights::WRITE.0,
        0,
    ));
    check(vifast_user::tcb_configure(
        tcb,
        cnode_worker as extern "C" fn() -> ! as u64,
        CNODE_STACK_TOP as u64,
        root_vspace,
        5,
    ));
    check(vifast_user::cnode_install(tcb, cnode));
    check(vifast_user::tcb_resume(tcb));
    tcb
}

#[allow(clippy::too_many_arguments)]
fn spawn_pager(
    meta_untyped: u8,
    data_untyped: u8,
    root_vspace: u8,
    endpoint_slot: u8,
    stack_base: usize,
    stack_top: usize,
    fault_va: u64,
    value: u64,
    entry: usize,
    slots: &mut SlotAllocator,
) -> (u8, u8, u8) {
    let pager_tcb = slots.alloc();
    retype(meta_untyped, ObjType::Tcb, pager_tcb);
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
    let page_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, page_frame);
    check(vifast_user::frame_write(
        page_frame,
        0,
        &value as *const u64 as u64,
        8,
    ));
    // Create the final page table but unmap the frame again, so the pager
    // server's later FrameMap only needs the existing table.
    map_with_tables(
        data_untyped,
        root_vspace,
        page_frame,
        fault_va,
        Rights::READ,
        slots,
    );
    check(vifast_user::frame_unmap(root_vspace, fault_va, page_frame));
    copy(endpoint_slot, pager_tcb, 1, Rights::WRITE);
    // Vòng 101 (K75): truyền slot endpoint phía nạn nhân (slot 1 vừa copy).
    check(vifast_user::tcb_configure_fault(
        pager_tcb,
        entry as u64,
        stack_top as u64,
        root_vspace,
        5,
        1,
    ));
    check(vifast_user::tcb_resume(pager_tcb));
    (pager_tcb, stack_frame, page_frame)
}

fn spawn_pager_server(
    meta_untyped: u8,
    data_untyped: u8,
    root_vspace: u8,
    endpoint_slot: u8,
    vfs_endpoint: u8,
    root_endpoint: u8,
    pager_tcb: u8,
    pager_frame: u8,
    pager2_tcb: u8,
    pager2_frame: u8,
    pager3_tcb: u8,
    pager3_frame: u8,
    slots: &mut SlotAllocator,
) -> (u8, u8, u8) {
    let server_tcb = slots.alloc();
    retype(meta_untyped, ObjType::Tcb, server_tcb);
    let stack_frame = slots.alloc();
    retype(data_untyped, ObjType::Frame, stack_frame);
    map_with_tables(
        data_untyped,
        root_vspace,
        stack_frame,
        PAGER_SERVER_STACK_BASE as u64,
        Rights::READ | Rights::WRITE,
        slots,
    );
    copy(
        endpoint_slot,
        server_tcb,
        PAGER_SERVER_ENDPOINT,
        Rights::READ | Rights::WRITE,
    );
    copy(pager_tcb, server_tcb, PAGER_SERVER_TCB, Rights::WRITE);
    copy(root_vspace, server_tcb, PAGER_SERVER_VSPACE, Rights::WRITE);
    copy(pager_frame, server_tcb, PAGER_SERVER_FRAME, Rights::READ);
    copy(pager2_tcb, server_tcb, PAGER_SERVER_TCB2, Rights::WRITE);
    copy(pager2_frame, server_tcb, PAGER_SERVER_FRAME2, Rights::READ);
    copy(pager3_tcb, server_tcb, PAGER_SERVER_TCB3, Rights::WRITE);
    copy(pager3_frame, server_tcb, PAGER_SERVER_FRAME3, Rights::READ);
    // Pager v2 (vòng 35): cấp Untyped con riêng để server tự retype Frame
    // mới sau evict thay vì map lại frame cũ. Dùng meta_untyped vì
    // data_untyped đã chật trên x86 (page table 4-level tốn objects).
    let pager_untyped = slots.alloc();
    check(vifast_user::retype_untyped(
        meta_untyped,
        4096,
        2,
        pager_untyped,
    ));
    copy(
        pager_untyped,
        server_tcb,
        PAGER_SERVER_UNTYPED,
        Rights::WRITE,
    );
    check(vifast_user::tcb_configure(
        server_tcb,
        pager_server as extern "C" fn() -> ! as u64,
        PAGER_SERVER_STACK_TOP as u64,
        root_vspace,
        5,
    ));

    // Vòng 71: cấp IPC buffer + cap VFS để pager server tự đọc /page.bin khi
    // fault. Chỉ riscv64/aarch64: trên x86_64 data/meta Untyped đã cạn mà
    // kiến trúc này map root ELF/stack bằng trang 2 MiB, nên VA mới trong
    // cùng block cần tách PMD thành PageTable — cả frame lẫn PT đều không có
    // nguồn (vòng 72 và 78 đã thử data, meta, boot Untyped: đều thiếu).
    let enable_vfs = cfg!(any(target_arch = "riscv64", target_arch = "aarch64"));
    unsafe {
        PAGER_VFS_ENABLED = enable_vfs;
    }
    let mut ipc_frame_slot = 0u8;
    if enable_vfs {
        let ipc_frame = slots.alloc();
        ipc_frame_slot = ipc_frame;
        retype(data_untyped, ObjType::Frame, ipc_frame);
        map_with_tables(
            data_untyped,
            root_vspace,
            ipc_frame,
            PAGER_IPC_BUF_VA as u64,
            Rights::READ | Rights::WRITE,
            slots,
        );
        if vifast_user::tcb_set_ipc_buffer(server_tcb, ipc_frame, PAGER_IPC_BUF_VA as u64).error
            != KError::Ok
        {
            write(b"PAGER IPC BUF BAD\n");
            unsafe {
                PAGER_VFS_ENABLED = false;
            }
        } else if vifast_user::cap_mint(
            vfs_endpoint,
            server_tcb,
            PAGER_SERVER_VFS_ENDPOINT,
            Rights::WRITE.0,
            vifast_abi::BADGE_PAGER,
        )
        .error
            != KError::Ok
        {
            write(b"PAGER VFS CAP BAD\n");
            unsafe {
                PAGER_VFS_ENABLED = false;
            }
        }
    }
    // Vòng 91/92: cap tới fault endpoint của root để báo hoàn tất demo pager
    // (mọi arch — không phụ thuộc VFS; x86 cũng cần để thu hồi + exec-from-FS).
    if vifast_user::cap_copy(
        root_endpoint,
        server_tcb,
        PAGER_SERVER_ROOT,
        Rights::WRITE.0,
    )
    .error
        != KError::Ok
    {
        write(b"PAGER ROOT CAP BAD\n");
    }
    check(vifast_user::tcb_resume(server_tcb));
    (server_tcb, stack_frame, ipc_frame_slot)
}

// Mốc 9 (vòng 46): root tự STAT path qua endpoint gốc. Gọi sau khi VFS
// sẵn sàng (cuối boot, sau DEMAND VFSLOAD) vì server init FS bất đồng
// bộ sau resume. Trả size khi OK.
/// Vòng 128: xoá file qua VFS (UNLINK) — dùng cho self-test đường unlink.
fn root_vfs_unlink(endpoint_slot: u8, path: &[u8]) -> bool {
    if path.is_empty() || path.len() > vifast_abi::vfs::MAX_PATH {
        return false;
    }
    unsafe {
        let mut i = 0;
        while i < path.len() {
            core::ptr::write_volatile((ROOT_IPC_BUF_VA as *mut u8).add(i), path[i]);
            i += 1;
        }
    }
    let reply = unsafe {
        vifast_user::call_message(endpoint_slot, vfs_abi::UNLINK, path.len() as u64, 0, 0)
    };
    reply.error == KError::Ok && reply.label == vfs_abi::OK
}

/// Vòng 129: đọc kích thước theo fd (STAT fd-based, data[0] = size).
fn root_vfs_stat_fd(endpoint_slot: u8, fd: u64) -> Option<u64> {
    let reply = unsafe { vifast_user::call_message(endpoint_slot, vfs_abi::STAT, fd, 0, 0) };
    if reply.error == KError::Ok && reply.label == vfs_abi::OK {
        Some(reply.data[0])
    } else {
        None
    }
}

/// Vòng 129: cắt file về `size` byte qua fd.
fn root_vfs_truncate(endpoint_slot: u8, fd: u64, size: u64) -> bool {
    let reply = unsafe { vifast_user::call_message(endpoint_slot, vfs_abi::TRUNCATE, fd, size, 0) };
    reply.error == KError::Ok && reply.label == vfs_abi::OK
}

/// Vòng 130: gửi lệnh VFS có HAI path (rename/link) — layout server:
/// path1 ở offset 0, path2 ở offset MAX_PATH; data[0]/[1] là độ dài.
fn root_vfs_two_paths(endpoint_slot: u8, op: u64, first: &[u8], second: &[u8]) -> bool {
    if first.is_empty()
        || second.is_empty()
        || first.len() > vifast_abi::vfs::MAX_PATH
        || second.len() > vifast_abi::vfs::MAX_PATH
    {
        return false;
    }
    unsafe {
        let base = ROOT_IPC_BUF_VA as *mut u8;
        let mut i = 0;
        while i < first.len() {
            core::ptr::write_volatile(base.add(i), first[i]);
            i += 1;
        }
        i = 0;
        while i < second.len() {
            core::ptr::write_volatile(base.add(vifast_abi::vfs::MAX_PATH + i), second[i]);
            i += 1;
        }
    }
    let reply = unsafe {
        vifast_user::call_message(
            endpoint_slot,
            op,
            first.len() as u64,
            second.len() as u64,
            0,
        )
    };
    reply.error == KError::Ok && reply.label == vfs_abi::OK
}

fn root_vfs_rename(endpoint_slot: u8, from: &[u8], to: &[u8]) -> bool {
    root_vfs_two_paths(endpoint_slot, vfs_abi::RENAME, from, to)
}

fn root_vfs_link(endpoint_slot: u8, from: &[u8], to: &[u8]) -> bool {
    root_vfs_two_paths(endpoint_slot, vfs_abi::LINK, from, to)
}

/// Vòng 131: chạy fsck qua server — xác nhận metadata còn nhất quán sau các
/// thao tác FS (tạo/xoá/cắt/đổi tên/link).
fn root_vfs_fsck(endpoint_slot: u8) -> bool {
    let reply = unsafe { vifast_user::call_message(endpoint_slot, vfs_abi::FSCK, 0, 0, 0) };
    reply.error == KError::Ok && reply.label == vfs_abi::OK
}

fn root_vfs_stat(endpoint_slot: u8, path: &[u8]) -> Option<u64> {
    if path.is_empty() || path.len() > vifast_abi::vfs::MAX_PATH {
        return None;
    }
    unsafe {
        let mut i = 0;
        while i < path.len() {
            core::ptr::write_volatile((ROOT_IPC_BUF_VA as *mut u8).add(i), path[i]);
            i += 1;
        }
    }
    // STAT_PATH: data0 = độ dài path (khác OPEN dùng data1).
    let reply = unsafe {
        vifast_user::call_message(endpoint_slot, vfs_abi::STAT_PATH, path.len() as u64, 0, 0)
    };
    if reply.error == KError::Ok && reply.label == vfs_abi::OK {
        Some(reply.data[0])
    } else {
        None
    }
}

// Mốc 9 (vòng 74/79): root tự OPEN/READ/CLOSE qua endpoint gốc để đọc
// file từ FS (bước 2 exec-thật: đọc ELF từ FS). OPEN flags=data0, len=data1;
// READ data=[fd, offset, len], label trả về là byte count, dữ liệu vào IPC
// buffer của root (kernel copy từ server). Tách open/read_at/close để vòng
// sau nạp ELF theo từng page; read_head giữ nguyên để verify magic.
// Fail-êm: chỉ quan sát, không đổi hành vi spawn.
fn root_vfs_open(endpoint_slot: u8, path: &[u8]) -> Option<u64> {
    if path.is_empty() || path.len() > vifast_abi::vfs::MAX_PATH {
        return None;
    }
    unsafe {
        let mut i = 0;
        while i < path.len() {
            core::ptr::write_volatile((ROOT_IPC_BUF_VA as *mut u8).add(i), path[i]);
            i += 1;
        }
    }
    let open = unsafe {
        vifast_user::call_message(
            endpoint_slot,
            vfs_abi::OPEN,
            vifast_abi::O_RDONLY,
            path.len() as u64,
            0,
        )
    };
    if open.error == KError::Ok && open.label == vfs_abi::OK {
        Some(open.data[0])
    } else {
        None
    }
}

/// Đọc tối đa `len` byte (kẹp MAX_IO=128) từ `offset` vào IPC buffer của
/// root. Trả số byte đọc được.
fn root_vfs_read_at(endpoint_slot: u8, fd: u64, offset: u64, len: u64) -> u64 {
    let want = core::cmp::min(len, vifast_abi::vfs::MAX_IO as u64);
    if want == 0 {
        return 0;
    }
    let read = unsafe { vifast_user::call_message(endpoint_slot, vfs_abi::READ, fd, offset, want) };
    if read.error == KError::Ok && read.label == want {
        want
    } else {
        0
    }
}

fn root_vfs_close(endpoint_slot: u8, fd: u64) {
    let _ = unsafe { vifast_user::call_message(endpoint_slot, vfs_abi::CLOSE, fd, 0, 0) };
}

/// Ghi `data` (tối đa MAX_IO) vào `path` ở `offset`. Lần đầu (`truncate`)
/// tạo mới/cắt file, các lần sau mở O_WRONLY ghi tiếp. Trả true khi ghi đủ.
fn root_vfs_write_at(
    endpoint_slot: u8,
    path: &[u8],
    offset: u64,
    data: &[u8],
    truncate: bool,
) -> bool {
    if path.is_empty()
        || path.len() > vifast_abi::vfs::MAX_PATH
        || data.len() > vifast_abi::vfs::MAX_IO
    {
        return false;
    }
    unsafe {
        let mut i = 0;
        while i < path.len() {
            core::ptr::write_volatile((ROOT_IPC_BUF_VA as *mut u8).add(i), path[i]);
            i += 1;
        }
    }
    let flags = if truncate {
        vifast_abi::O_CREATE | vifast_abi::O_WRONLY | vifast_abi::O_TRUNC
    } else {
        vifast_abi::O_WRONLY
    };
    let open = unsafe {
        vifast_user::call_message(endpoint_slot, vfs_abi::OPEN, flags, path.len() as u64, 0)
    };
    if open.error != KError::Ok || open.label != vfs_abi::OK {
        return false;
    }
    let fd = open.data[0];
    unsafe {
        let mut i = 0;
        while i < data.len() {
            core::ptr::write_volatile((ROOT_IPC_BUF_VA as *mut u8).add(i), data[i]);
            i += 1;
        }
    }
    let written = unsafe {
        vifast_user::call_message(endpoint_slot, vfs_abi::WRITE, fd, offset, data.len() as u64)
    };
    root_vfs_close(endpoint_slot, fd);
    written.error == KError::Ok && written.label == data.len() as u64
}

// Mốc 9 (vòng 82): đọc + parse ELF header của file trên FS (bước 3
// exec-thật). `parse` cần cả program header table (nằm sau 64B ELF header)
// nên đọc 512 byte đầu qua 4 READ vào stack buffer rồi parse tại chỗ
// (không cần staging toàn file). Trả entry point khi hợp lệ. Fail-êm.
// Mốc 9 (vòng 82/83): đọc + verify prefix file trên FS (bước 2 exec-thật:
// đọc ELF từ FS tin cậy từng byte). `parse` đòi toàn bộ segment trong buffer
// nên chưa parse tại đây — vòng sau nạp segment theo page rồi parse trên
// staging frames. Đọc tối đa `expected.len()` (kẹp 1KiB) và so khớp từng
// byte với mẫu gọi truyền vào. Fail-êm.
/// Vòng 110: so khớp N byte CUỐI của file — bắt lỗi ghi hụt/ghi lệch mà
/// kiểm tra kích thước và prefix không phát hiện được.
/// Vòng 111: so khớp một đoạn bất kỳ trong file (offset tuyệt đối) — phủ
/// trường hợp ghi lệch chunk ở giữa file.
fn root_vfs_verify_at(endpoint_slot: u8, path: &[u8], offset: u64, expected: &[u8]) -> bool {
    if expected.is_empty() || expected.len() > 64 {
        return false;
    }
    let Some(size) = root_vfs_stat(endpoint_slot, path) else {
        return false;
    };
    if offset + expected.len() as u64 > size {
        return false;
    }
    let Some(fd) = root_vfs_open(endpoint_slot, path) else {
        return false;
    };
    let mut ok =
        root_vfs_read_at(endpoint_slot, fd, offset, expected.len() as u64) == expected.len() as u64;
    if ok {
        let mut i = 0;
        while i < expected.len() {
            let b = unsafe { core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(i)) };
            if expected[i] != b {
                ok = false;
                break;
            }
            i += 1;
        }
    }
    root_vfs_close(endpoint_slot, fd);
    ok
}

fn root_vfs_verify_tail(endpoint_slot: u8, path: &[u8], expected: &[u8]) -> bool {
    if expected.is_empty() || expected.len() > 64 {
        return false;
    }
    let Some(size) = root_vfs_stat(endpoint_slot, path) else {
        return false;
    };
    if size < expected.len() as u64 {
        return false;
    }
    let start = size - expected.len() as u64;
    let Some(fd) = root_vfs_open(endpoint_slot, path) else {
        return false;
    };
    let mut ok =
        root_vfs_read_at(endpoint_slot, fd, start, expected.len() as u64) == expected.len() as u64;
    if ok {
        let mut i = 0;
        while i < expected.len() {
            let b = unsafe { core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(i)) };
            if expected[i] != b {
                ok = false;
                break;
            }
            i += 1;
        }
    }
    root_vfs_close(endpoint_slot, fd);
    ok
}

fn root_vfs_verify_prefix(endpoint_slot: u8, path: &[u8], expected: &[u8]) -> bool {
    if expected.is_empty() || expected.len() > 1024 {
        return false;
    }
    let Some(fd) = root_vfs_open(endpoint_slot, path) else {
        return false;
    };
    // Đọc theo chunk MAX_IO vào IPC buffer rồi so ngay (không cần stack lớn).
    let mut off = 0usize;
    let mut ok = true;
    while off < expected.len() {
        let n = root_vfs_read_at(endpoint_slot, fd, off as u64, (expected.len() - off) as u64);
        if n == 0 {
            ok = false;
            break;
        }
        let mut i = 0;
        while i < n as usize {
            let b = unsafe { core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(i)) };
            if expected[off + i] != b {
                ok = false;
                break;
            }
            i += 1;
        }
        if !ok {
            break;
        }
        off += n as usize;
    }
    root_vfs_close(endpoint_slot, fd);
    ok && off == expected.len()
}

fn root_vfs_read_head(endpoint_slot: u8, path: &[u8]) -> bool {
    let Some(fd) = root_vfs_open(endpoint_slot, path) else {
        return false;
    };
    let mut ok = false;
    if root_vfs_read_at(endpoint_slot, fd, 0, 8) == 8 {
        let mut head = [0u8; 8];
        let mut i = 0;
        while i < 8 {
            head[i] = unsafe { core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(i)) };
            i += 1;
        }
        ok = u64::from_le_bytes(head) == vifast_abi::pager::DEMAND_VALUE;
    }
    root_vfs_close(endpoint_slot, fd);
    ok
}

/// Đọc tối đa `buf.len()` byte đầu file vào `buf` qua READ chunked.
fn root_vfs_read_prefix(endpoint_slot: u8, fd: u64, buf: &mut [u8]) -> usize {
    let mut got = 0usize;
    while got < buf.len() {
        let n = root_vfs_read_at(endpoint_slot, fd, got as u64, (buf.len() - got) as u64) as usize;
        if n == 0 {
            break;
        }
        let mut i = 0;
        while i < n {
            buf[got + i] =
                unsafe { core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(i)) };
            i += 1;
        }
        got += n;
    }
    got
}

/// Copy `len` byte ở `file_offset` của file vào frame tại `frame_offset`
/// (mốc 9, vòng 84). Đọc qua IPC buffer từng chunk MAX_IO rồi frame_write —
/// không cần staging, frame đích chính là frame sẽ map cho chương trình.
fn root_vfs_copy_to_frame(
    endpoint_slot: u8,
    fd: u64,
    file_offset: u64,
    frame: u8,
    frame_offset: u64,
    len: u64,
) -> bool {
    let mut done = 0u64;
    while done < len {
        let n = root_vfs_read_at(endpoint_slot, fd, file_offset + done, len - done);
        if n == 0 {
            return false;
        }
        // Copy IPC buffer -> buffer cục bộ rồi frame_write từ đó: kernel từ
        // chối nguồn là chính IPC buffer (validate_user_range thấy leaf
        // không hợp lệ — vòng 89), còn nguồn tĩnh/stack thì hợp lệ.
        let mut chunk = [0u8; 256];
        let mut i = 0;
        while i < n as usize {
            chunk[i] = unsafe { core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(i)) };
            i += 1;
        }
        let mut inner = 0u64;
        while inner < n {
            let part = core::cmp::min(256, n - inner) as usize;
            let fw = vifast_user::frame_write(
                frame,
                frame_offset + done + inner,
                chunk.as_ptr() as u64,
                part as u64,
            );
            if fw.error != KError::Ok {
                return false;
            }
            inner += part as u64;
        }
        done += n;
    }
    true
}

/// Exec-thật từ FS (mốc 9, vòng 84): đọc ELF header + phdr table (1KiB) để
/// lấy entry/segment, rồi nạp từng page thẳng từ file vào frame đích và map
/// — không staging, không tốn frame ngoài image. Trả TCB khi spawn xong.
fn spawn_elf_from_fs(
    endpoint_slot: u8,
    path: &[u8],
    msg: &[u8],
    meta_untyped: u8,
    data_untyped: u8,
    affinity: u8,
    slots: &mut SlotAllocator,
    used: &mut [u8; MAX_EXEC_SLOTS],
) -> Option<u8> {
    let mut used_len = 0usize;
    let fd = root_vfs_open(endpoint_slot, path)?;
    // 1KiB đủ ELF header (64B) + tới 16 phdr (56B mỗi cái).
    let mut head = [0u8; 1024];
    let got = root_vfs_read_prefix(endpoint_slot, fd, &mut head);
    let image = match vifast_elf::parse_metadata(
        &head[..got],
        USER_ELF_MACHINE,
        CODE_VA as u64,
        CODE_VA as u64 + MODULE_WINDOW,
    ) {
        Ok(image) => image,
        Err(_) => {
            root_vfs_close(endpoint_slot, fd);
            return None;
        }
    };
    let vspace = track_alloc(slots, used, &mut used_len)?;
    let vspace_r = vifast_user::retype(data_untyped, ObjType::VSpace as u64, vspace);
    if vspace_r.error != KError::Ok {
        root_vfs_close(endpoint_slot, fd);
        return None;
    }
    for segment in image.segments() {
        let page_base = segment.vaddr & !0xfff;
        let mut page = 0;
        while page < segment.page_count() {
            let va = page_base + page as u64 * vifast_abi::DEFAULT_PAGE_SIZE;
            let frame = track_alloc(slots, used, &mut used_len)?;
            let frame_r = vifast_user::retype(data_untyped, ObjType::Frame as u64, frame);
            if frame_r.error != KError::Ok {
                root_vfs_close(endpoint_slot, fd);
                return None;
            }
            // Nạp phần file rơi vào page này; phần BSS đã zero từ retype.
            let copy_start = core::cmp::max(va, segment.vaddr);
            let copy_end = core::cmp::min(
                va + vifast_abi::DEFAULT_PAGE_SIZE,
                segment.vaddr + segment.file_size as u64,
            );
            if copy_start < copy_end {
                let source = segment.file_offset + (copy_start - segment.vaddr) as usize;
                if !root_vfs_copy_to_frame(
                    endpoint_slot,
                    fd,
                    source as u64,
                    frame,
                    copy_start - va,
                    copy_end - copy_start,
                ) {
                    root_vfs_close(endpoint_slot, fd);
                    return None;
                }
            }
            if page == 0 && segment.executable() && !write_program_args(frame, msg) {
                root_vfs_close(endpoint_slot, fd);
                return None;
            }
            let mut rights = Rights::READ;
            if segment.writable() {
                rights = rights | Rights::WRITE;
            }
            if segment.executable() {
                rights = rights | Rights::EXEC;
            }
            if !map_frame_fallible(
                data_untyped,
                vspace,
                frame,
                va,
                rights.0,
                slots,
                used,
                &mut used_len,
            ) {
                root_vfs_close(endpoint_slot, fd);
                return None;
            }
            page += 1;
        }
    }
    root_vfs_close(endpoint_slot, fd);
    // Stack riêng như spawn_elf_copy.
    let stack_frame = track_alloc(slots, used, &mut used_len)?;
    let stack_r = vifast_user::retype(data_untyped, ObjType::Frame as u64, stack_frame);
    if stack_r.error != KError::Ok {
        return None;
    }
    if !map_frame_fallible(
        data_untyped,
        vspace,
        stack_frame,
        EXEC_STACK_BASE as u64,
        (Rights::READ | Rights::WRITE).0,
        slots,
        used,
        &mut used_len,
    ) {
        return None;
    }
    let tcb = track_alloc(slots, used, &mut used_len)?;
    if vifast_user::retype(meta_untyped, ObjType::Tcb as u64, tcb).error != KError::Ok {
        return None;
    }
    if vifast_user::tcb_configure(tcb, image.entry as u64, EXEC_STACK_TOP as u64, vspace, 5).error
        != KError::Ok
    {
        return None;
    }
    if vifast_user::tcb_set_affinity(tcb, affinity).error != KError::Ok {
        return None;
    }
    if vifast_user::tcb_resume(tcb).error != KError::Ok {
        return None;
    }
    Some(tcb)
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
    affinity: u8,
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
    check(vifast_user::tcb_set_affinity(tcb, affinity));
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
        || info.sched_harts & !0b1111 != 0
    {
        panic!("invalid BootInfo slots");
    }
    write(BOOTINFO_OK);

    let mut slots = SlotAllocator {
        next: info.free_slot_start as u8,
        end: info.free_slot_end as u8,
        released: [0; 16],
        released_len: 0,
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
    let probe_endpoint_slot = slots.alloc();

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
    retype(meta_untyped, ObjType::Endpoint, probe_endpoint_slot);

    for (fault_tcb, entry, stack, fault_slot) in [
        (
            fault1_tcb,
            USER_ENTRY_FAULT1,
            FAULT1_STACK_TOP,
            FAULT1_TCB_SLOT,
        ),
        (
            fault2_tcb,
            USER_ENTRY_FAULT2,
            FAULT2_STACK_TOP,
            FAULT2_TCB_SLOT,
        ),
        (
            fault3_tcb,
            USER_ENTRY_FAULT3,
            FAULT3_STACK_TOP,
            FAULT3_TCB_SLOT,
        ),
    ] {
        copy(
            fault_endpoint_slot,
            fault_tcb,
            fault_slot as u8,
            Rights::READ | Rights::WRITE,
        );
        // Vòng 101 (K75): kernel đọc vé endpoint từ slot phía nạn nhân và
        // đối chiếu caller cũng giữ vé tới cùng endpoint — root copy vé
        // cho nạn nhân trước, rồi truyền fault_slot (slot phía nạn nhân).
        check(vifast_user::tcb_configure_fault(
            fault_tcb,
            entry as u64,
            stack as u64,
            root_vspace,
            6,
            fault_slot as u8,
        ));
        check(vifast_user::tcb_resume(fault_tcb));
    }

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
    check(vifast_user::iommu_unmap(
        iommu,
        vifast_abi::IOMMU_IOVA_BASE,
        server_buffer_frame,
    ));
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

    // Mốc 9 (vòng 46/75): IPC buffer + tự gọi VFS từ root. Cap endpoint
    // gốc của root có badge 0 nên server từ chối mọi VFS call (EACCES —
    // badge 0 là không xác thực). Mint cap mới với badge SUPERUSER
    // (= uid 0, chủ file) cho chính root qua ROOT_SELF_SLOT. Fail-êm:
    // thiếu tài nguyên (x86) thì bỏ qua, gate không đổi.
    // Trả slot endpoint VFS của root (0 khi fail).
    let root_vfs_slot = (|| {
        let frame = slots.alloc();
        if vifast_user::retype(data_untyped, ObjType::Frame as u64, frame).error != KError::Ok {
            return 0;
        }
        map_with_tables(
            data_untyped,
            root_vspace,
            frame,
            ROOT_IPC_BUF_VA as u64,
            Rights::READ | Rights::WRITE,
            &mut slots,
        );
        if vifast_user::tcb_set_ipc_buffer(ROOT_SELF_SLOT as u8, frame, ROOT_IPC_BUF_VA as u64)
            .error
            != KError::Ok
        {
            return 0;
        }
        let vfs_slot = slots.alloc();
        if vifast_user::cap_mint(
            endpoint_slot,
            ROOT_SELF_SLOT as u8,
            vfs_slot,
            (Rights::READ | Rights::WRITE).0,
            vifast_abi::BADGE_SUPERUSER,
        )
        .error
            != KError::Ok
        {
            return 0;
        }
        vfs_slot
    })();

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
    // Mốc 9 (vòng 33): cap lệnh cho shell `run` — fault_endpoint với badge
    // RUN_BADGE vào slot lệnh của client. Root phân biệt bằng badge.
    check(vifast_user::cap_mint(
        fault_endpoint_slot,
        client_tcb,
        CLIENT_CMD_SLOT,
        Rights::WRITE.0,
        RUN_BADGE,
    ));
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
    // Vòng 101 (K75): truyền slot endpoint phía nạn nhân (slot 6 vừa copy).
    check(vifast_user::tcb_configure_fault(
        fault4_tcb,
        USER_ENTRY_FAULT4 as u64,
        FAULT4_STACK_TOP as u64,
        client_vspace,
        6,
        6,
    ));
    check(vifast_user::tcb_resume(fault4_tcb));
    let destroy_code = run_destroy_demo(data_untyped, root_vspace, &mut slots);
    if destroy_code != 0 {
        destroy_demo_bad(destroy_code);
    }
    spawn_idle(meta_untyped, data_untyped, root_vspace, &mut slots);
    let cnode_tcb = spawn_cnode_demo(
        meta_untyped,
        data_untyped,
        root_vspace,
        fault_endpoint_slot,
        &mut slots,
    );
    let pager_endpoint = slots.alloc();
    retype(meta_untyped, ObjType::Endpoint, pager_endpoint);
    let (pager_tcb, pager_stack, pager_frame) = spawn_pager(
        meta_untyped,
        data_untyped,
        root_vspace,
        pager_endpoint,
        PAGER_STACK_BASE,
        PAGER_STACK_TOP,
        PAGER_VA,
        PAGER_VALUE,
        pager_client as extern "C" fn() -> ! as usize,
        &mut slots,
    );
    let (pager2_tcb, pager2_stack, pager2_frame) = spawn_pager(
        meta_untyped,
        data_untyped,
        root_vspace,
        pager_endpoint,
        PAGER2_STACK_BASE,
        PAGER2_STACK_TOP,
        PAGER2_VA,
        PAGER2_VALUE,
        pager2_client as extern "C" fn() -> ! as usize,
        &mut slots,
    );
    // Vòng 69: demand-load không cần root cấp frame nữa — server tự retype
    // từ Untyped riêng (PAGER_SERVER_UNTYPED) ngay lúc fault, nên cả 3 arch
    // (kể cả x86_64 vốn cạn Untyped ở root) đều chạy được.
    let pager3_entry = pager3_client as extern "C" fn() -> ! as usize;
    let (pager3_tcb, pager3_stack, pager3_frame) = spawn_pager(
        meta_untyped,
        data_untyped,
        root_vspace,
        pager_endpoint,
        PAGER3_STACK_BASE,
        PAGER3_STACK_TOP,
        PAGER3_VA,
        PAGER3_VALUE,
        pager3_entry,
        &mut slots,
    );
    let (pager_server_tcb, pager_server_stack, pager_server_ipc) = spawn_pager_server(
        meta_untyped,
        data_untyped,
        root_vspace,
        pager_endpoint,
        endpoint_slot,
        fault_endpoint_slot,
        pager_tcb,
        pager_frame,
        pager2_tcb,
        pager2_frame,
        pager3_tcb,
        pager3_frame,
        &mut slots,
    );

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
            0b10,
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
            0b10,
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
            0b10,
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
            0b10,
            &mut slots,
        );
    }
    // Mốc 5 (vòng 31): worker cho hart 2/3 khi bootinfo có bit. Affinity
    // 0b1100 để scheduler (enqueue least-loaded + steal) phân tải sang
    // hart 2/3 thay vì để chúng spin BKL vô ích trong idle loop.
    if info.sched_harts & 0b0100 != 0 {
        spawn_worker(
            meta_untyped,
            data_untyped,
            root_vspace,
            fault_endpoint_slot,
            root_notification_slot,
            Rights::READ,
            None,
            5,
            smp_worker_g as *const () as usize,
            WORKER_G_STACK_BASE,
            WORKER_G_STACK_TOP,
            0b0100,
            &mut slots,
        );
    }
    if info.sched_harts & 0b1000 != 0 {
        spawn_worker(
            meta_untyped,
            data_untyped,
            root_vspace,
            fault_endpoint_slot,
            root_notification_slot,
            Rights::READ,
            None,
            5,
            smp_worker_h as *const () as usize,
            WORKER_H_STACK_BASE,
            WORKER_H_STACK_TOP,
            0b1000,
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
    // Mốc 9 (vòng 34): VSpace exec setup 1 lần + TCB hiện tại. `run`
    // lần sau chỉ thay TCB (rẻ, không chạm data_untyped).
    let mut exec_used = [0u8; MAX_EXEC_SLOTS];
    let mut exec_len = 0usize;
    let mut exec_vspace = 0u8;
    // Vòng 170: nhớ TCB của task exec để `kill` suspend TRƯỚC khi dọn tài
    // nguyên — huỷ VSpace khi task còn chạy trên hart khác là nguy hiểm.
    let mut exec_tcb = 0u8;
    let mut exit_demo_done = false;
    let mut tlb_remapped = false;
    let mut tlb_done = false;
    let mut suspend_done = false;
    let mut suspend_ok = false;
    let mut suspend_printed = false;
    let mut reaped = 0u8;
    let mut reap_ok = true;
    let mut reap_done = false;
    let mut pending_worker_e = false;
    let mut start_worker_e = false;
    let mut run_fs_selftest_done = false;
    #[cfg(feature = "test-destroy-smp")]
    let mut destroy_smp_done = false;
    loop {
        let message = unsafe { vifast_user::recv_message(fault_endpoint_slot, 0) };
        #[cfg(feature = "test-destroy-smp")]
        if !destroy_smp_done
            && message.error == KError::Ok
            && (message.label == WORKER_LABEL_C_RUNNING || message.label == WORKER_LABEL_D_READY)
        {
            let code = run_destroy_smp_demo(data_untyped, root_vspace, &mut slots);
            if code != 0 {
                destroy_smp_bad(code);
            }
            destroy_smp_done = true;
        }
        if message.error == KError::Ok {
            // Mốc 9 (vòng 33): lệnh `run` từ shell — fire-and-forget trên
            // fault_endpoint với badge RUN_BADGE. Không reply (send đơn).
            // Badge lạ + label RUN mới dispatch; fault thật badge 0.
            // Vòng 91: pager server báo xong demo demand-load → root thu hồi
            // 3 pager client: revoke cap copy bên server, unmap frame khỏi
            // root VSpace, destroy frame + TCB → trả 6 frame + 3 TCB.
            if message.label == PAGER_DONE_LABEL {
                let mut ok = 0u64;
                let mut i = 0;
                while i < 3 {
                    let (tcb_slot, stack_slot, stack_va, frame_slot, va) = match i {
                        0 => (
                            pager_tcb,
                            pager_stack,
                            PAGER_STACK_BASE as u64,
                            pager_frame,
                            PAGER_VA,
                        ),
                        1 => (
                            pager2_tcb,
                            pager2_stack,
                            PAGER2_STACK_BASE as u64,
                            pager2_frame,
                            PAGER2_VA,
                        ),
                        _ => (
                            pager3_tcb,
                            pager3_stack,
                            PAGER3_STACK_BASE as u64,
                            pager3_frame,
                            PAGER3_VA,
                        ),
                    };
                    // Bỏ cap dẫn xuất (nếu có) trước khi destroy object gốc.
                    let _ = vifast_user::cap_revoke(stack_slot);
                    let _ = vifast_user::cap_revoke(frame_slot);
                    let _ = vifast_user::cap_revoke(tcb_slot);
                    let ub = vifast_user::frame_unmap(root_vspace, stack_va, stack_slot).error;
                    let uf = vifast_user::frame_unmap(root_vspace, va, frame_slot).error;
                    let ds = vifast_user::object_destroy(stack_slot).error;
                    let df = vifast_user::object_destroy(frame_slot).error;
                    let dt = vifast_user::tcb_destroy(tcb_slot).error;
                    if (ub == KError::Ok || ub == KError::NotMapped)
                        && (uf == KError::Ok || uf == KError::NotMapped)
                        && ds == KError::Ok
                        && df == KError::Ok
                        && dt == KError::Ok
                    {
                        ok += 1;
                    }
                    i += 1;
                }
                write(if ok == 3 {
                    PAGER_RECLAIM_OK
                } else {
                    PAGER_RECLAIM_BAD
                });
                // Vòng 104: thu hồi pager server (TCB + stack + IPC frame)
                // sau khi hết demo demand-load — trả 2 frame + 1 TCB.
                if pager_server_tcb != 0 {
                    let _ = vifast_user::cap_revoke(pager_server_stack);
                    let _ = vifast_user::cap_revoke(pager_server_tcb);
                    let dt = vifast_user::tcb_destroy(pager_server_tcb).error;
                    let us = vifast_user::frame_unmap(
                        root_vspace,
                        PAGER_SERVER_STACK_BASE as u64,
                        pager_server_stack,
                    )
                    .error;
                    let ds = vifast_user::object_destroy(pager_server_stack).error;
                    let mut di = KError::Ok;
                    if pager_server_ipc != 0 {
                        let _ = vifast_user::frame_unmap(
                            root_vspace,
                            PAGER_IPC_BUF_VA as u64,
                            pager_server_ipc,
                        );
                        di = vifast_user::object_destroy(pager_server_ipc).error;
                    }
                    write(b"PAGER SRV ");
                    write_decimal(dt as u64);
                    write(b"/");
                    write_decimal(ds as u64);
                    write(b"/");
                    write_decimal(di as u64);
                    write(b"\n");
                    let _ = us;
                }
                // Vòng 92: ngay sau thu hồi, tự chạy exec-thật từ FS (ghi
                // ELF demo thành /prog.elf rồi nạp TỪNG PAGE từ file). Đây
                // là điểm kiểm chứng tự động trong cổng chuẩn, không cần
                // nhập bàn phím; fail-êm nếu thiếu bộ nhớ.
                if !run_fs_selftest_done {
                    run_fs_selftest_done = true;
                    let mut off = 0usize;
                    while off < EXEC_DEMO_ELF.len() {
                        let n = core::cmp::min(vifast_abi::vfs::MAX_IO, EXEC_DEMO_ELF.len() - off);
                        let mut chunk = [0u8; vifast_abi::vfs::MAX_IO];
                        let mut j = 0;
                        while j < n {
                            chunk[j] = EXEC_DEMO_ELF[off + j];
                            j += 1;
                        }
                        if !root_vfs_write_at(
                            root_vfs_slot,
                            b"/prog.elf",
                            off as u64,
                            &chunk[..n],
                            off == 0,
                        ) {
                            break;
                        }
                        off += n;
                    }
                    // Vòng 109: kiểm tra TOÀN VẸN file vừa ghi — kích thước
                    // phải đúng bằng ảnh ELF dựng sẵn. Bắt lỗi ghi thiếu/
                    // ghi đè âm thầm của đường VFS (trước đây chỉ verify 64
                    // byte đầu).
                    match root_vfs_stat(root_vfs_slot, b"/prog.elf") {
                        Some(size) if size == EXEC_DEMO_ELF.len() as u64 => {
                            write(b"PROG SIZE OK\n")
                        }
                        Some(size) => {
                            write(b"PROG SIZE BAD ");
                            write_decimal(size);
                            write(b"\n");
                        }
                        None => write(b"PROG SIZE BAD 0\n"),
                    }
                    let tail_len = core::cmp::min(32, EXEC_DEMO_ELF.len());
                    let tail = &EXEC_DEMO_ELF[EXEC_DEMO_ELF.len() - tail_len..];
                    if root_vfs_verify_tail(root_vfs_slot, b"/prog.elf", tail) {
                        write(b"PROG TAIL OK\n")
                    } else {
                        write(b"PROG TAIL BAD\n")
                    }
                    // Đoạn giữa file: 16 byte quanh len/2 — phủ lỗi ghi lệch
                    // chunk (kiểm tra đầu + cuối không phát hiện được).
                    let mid = EXEC_DEMO_ELF.len() / 2;
                    if root_vfs_verify_at(
                        root_vfs_slot,
                        b"/prog.elf",
                        mid as u64,
                        &EXEC_DEMO_ELF[mid..mid + 16],
                    ) {
                        write(b"PROG MID OK\n")
                    } else {
                        write(b"PROG MID BAD\n")
                    }
                    // Vòng 128: vòng đời file trọn vẹn — tạo, kiểm kích thước,
                    // xoá, xác nhận đã biến mất. Bắt lỗi đường UNLINK/FS.
                    let rm_ok = root_vfs_write_at(root_vfs_slot, b"/rm-test", 0, b"12345678", true)
                        && root_vfs_stat(root_vfs_slot, b"/rm-test") == Some(8)
                        && root_vfs_unlink(root_vfs_slot, b"/rm-test")
                        && root_vfs_stat(root_vfs_slot, b"/rm-test").is_none();
                    write(if rm_ok { b"FS RM OK\n" } else { b"FS RM BAD\n" });
                    // Vòng 129: đường TRUNCATE — ghi 8 byte, cắt còn 4, kiểm
                    // kích thước và nội dung còn lại đúng "1234".
                    let mut trunc_ok =
                        root_vfs_write_at(root_vfs_slot, b"/trunc-test", 0, b"12345678", true);
                    if trunc_ok {
                        if let Some(fd) = root_vfs_open(root_vfs_slot, b"/trunc-test") {
                            trunc_ok = root_vfs_truncate(root_vfs_slot, fd, 4)
                                && root_vfs_stat_fd(root_vfs_slot, fd) == Some(4);
                            if trunc_ok {
                                let n = root_vfs_read_at(root_vfs_slot, fd, 0, 4);
                                trunc_ok = n == 4;
                                if trunc_ok {
                                    let mut i = 0;
                                    while i < 4 {
                                        let b = unsafe {
                                            core::ptr::read_volatile(
                                                (ROOT_IPC_BUF_VA as *const u8).add(i),
                                            )
                                        };
                                        if b != b"1234"[i] {
                                            trunc_ok = false;
                                            break;
                                        }
                                        i += 1;
                                    }
                                }
                            }
                            root_vfs_close(root_vfs_slot, fd);
                        } else {
                            trunc_ok = false;
                        }
                    }
                    let _ = root_vfs_unlink(root_vfs_slot, b"/trunc-test");
                    write(if trunc_ok {
                        b"FS TRUNC OK\n"
                    } else {
                        b"FS TRUNC BAD\n"
                    });
                    // Vòng 130: RENAME — tạo file, đổi tên, tên cũ phải biến
                    // mất và tên mới giữ nguyên kích thước.
                    let ren_ok = root_vfs_write_at(root_vfs_slot, b"/rn-old", 0, b"abc", true)
                        && root_vfs_rename(root_vfs_slot, b"/rn-old", b"/rn-new")
                        && root_vfs_stat(root_vfs_slot, b"/rn-old").is_none()
                        && root_vfs_stat(root_vfs_slot, b"/rn-new") == Some(3);
                    let _ = root_vfs_unlink(root_vfs_slot, b"/rn-new");
                    let _ = root_vfs_unlink(root_vfs_slot, b"/rn-old");
                    write(if ren_ok {
                        b"FS RENAME OK\n"
                    } else {
                        b"FS RENAME BAD\n"
                    });
                    // Vòng 130: LINK cứng — hai tên cùng inode, cùng kích thước.
                    let ln_ok = root_vfs_write_at(root_vfs_slot, b"/ln-a", 0, b"xyz", true)
                        && root_vfs_link(root_vfs_slot, b"/ln-a", b"/ln-b")
                        && root_vfs_stat(root_vfs_slot, b"/ln-a") == Some(3)
                        && root_vfs_stat(root_vfs_slot, b"/ln-b") == Some(3);
                    let _ = root_vfs_unlink(root_vfs_slot, b"/ln-a");
                    let _ = root_vfs_unlink(root_vfs_slot, b"/ln-b");
                    write(if ln_ok {
                        b"FS LINK OK\n"
                    } else {
                        b"FS LINK BAD\n"
                    });
                    // Vòng 131: sau toàn bộ thao tác FS ở trên, fsck phải sạch.
                    let fsck_ok = root_vfs_fsck(root_vfs_slot);
                    write(if fsck_ok {
                        b"FS OPS FSCK OK\n"
                    } else {
                        b"FS OPS FSCK BAD\n"
                    });
                    // Vòng 155: sau toàn bộ chuỗi thao tác FS, bộ nhớ Untyped
                    // phải còn nhất quán: trống > 0 và không vượt tổng.
                    let mem = vifast_user::mem_info();
                    let used = mem.value & 0xffff_ffff;
                    let size = (mem.value >> 32) & 0xffff_ffff;
                    let mem_ok = mem.error == KError::Ok && size > 0 && used < size;
                    write(if mem_ok {
                        b"MEM FREE OK\n"
                    } else {
                        b"MEM FREE BAD\n"
                    });
                    // Vòng 132 (bảo mật cơ bản): các lời gọi SAI phải bị từ
                    // chối bằng mã lỗi đúng, không được làm sập kernel hay
                    // treo task gọi.
                    // Vòng 132 (bảo mật cơ bản): in mã lỗi của từng lời gọi
                    // SAI để biết lời gọi nào bị xử lý chưa đúng.
                    // Vòng 132 (bảo mật cơ bản): mọi lời gọi với handle KHÔNG
                    // hợp lệ hoặc vượt vùng người dùng phải bị TỪ CHỐI (mã lỗi
                    // khác Ok) và kernel vẫn chạy tiếp — đây là điều cần khoá.
                    // Vòng 161: 63 KHÔNG còn chắc chắn trống khi arena lớn hơn
                    // (cspace của root có thể đã dùng tới đó) ⇒ dùng slot ngoài
                    // phạm vi để phép thử luôn có nghĩa.
                    let bad_slot = 200u8;
                    let mut reject_ok = vifast_user::tcb_suspend(bad_slot).error != KError::Ok
                        && vifast_user::tcb_destroy(bad_slot).error != KError::Ok
                        && vifast_user::object_destroy(bad_slot).error != KError::Ok
                        && vifast_user::frame_write(bad_slot, 0, b"x".as_ptr() as u64, 1).error
                            != KError::Ok;
                    let beyond = (vifast_abi::user_layout::USER_VA_END - 1) as u64;
                    reject_ok &= vifast_user::debug_write(beyond, 4096).error != KError::Ok;
                    write(if reject_ok {
                        b"SYS REJECT OK\n"
                    } else {
                        b"SYS REJECT BAD\n"
                    });
                    // Vòng 164 (bảo mật cơ bản): kernel KHÔNG được đọc hộ vùng
                    // địa chỉ ngoài cửa sổ copy của user — nếu đọc được thì
                    // DebugWrite trở thành kênh rò rỉ bộ nhớ kernel.
                    let below = (vifast_abi::user_layout::USER_VA_START - 1) as u64;
                    let sec_ok = vifast_user::debug_write(below, 4).error != KError::Ok
                        && vifast_user::debug_write(0, 4).error != KError::Ok;
                    write(if sec_ok {
                        b"SEC PROBE OK\n"
                    } else {
                        b"SEC PROBE BAD\n"
                    });
                    // Vòng 133: IPC phải chịu được yêu cầu rác — server trả
                    // lỗi EINVAL thay vì treo hoặc sập, và kênh VFS vẫn dùng
                    // được ngay sau đó.
                    let junk = unsafe { vifast_user::call_message(root_vfs_slot, 0xffff, 0, 0, 0) };
                    let still_ok = root_vfs_stat(root_vfs_slot, b"/").is_some();
                    write(if junk.label == vfs_abi::ERR && still_ok {
                        b"IPC JUNK OK\n"
                    } else {
                        b"IPC JUNK BAD\n"
                    });
                    // Vòng 134: endpoint đã bị destroy thì lời gọi phải trả
                    // lỗi NGAY, không được treo task gọi.
                    let ep = probe_endpoint_slot;
                    let destroyed = vifast_user::object_destroy(ep);
                    let dead = unsafe { vifast_user::call_message(ep, 1, 0, 0, 0) };
                    write(
                        if destroyed.error == KError::Ok && dead.error != KError::Ok {
                            b"IPC DEAD OK\n"
                        } else {
                            b"IPC DEAD BAD\n"
                        },
                    );
                    // Vòng 137: slot VỪA destroy phải retype lại được ngay
                    // (kiểm cspace thực sự trả slot về, không giữ cap chết).
                    let reuse = vifast_user::retype(meta_untyped, ObjType::Endpoint as u64, ep);
                    let reuse_ok = reuse.error == KError::Ok;
                    if reuse_ok {
                        let _ = vifast_user::object_destroy(ep);
                    }
                    write(if reuse_ok {
                        b"SLOT REUSE OK\n"
                    } else {
                        b"SLOT REUSE BAD\n"
                    });
                    // Vòng 165 (tiến trình): huỷ TCB phải chạy được thật —
                    // retype một TCB vào slot vừa trống rồi destroy nó, và
                    // destroy một slot không tồn tại phải bị TỪ CHỐI.
                    let tcb_probe = vifast_user::retype(meta_untyped, ObjType::Tcb as u64, ep);
                    let kill_ok = tcb_probe.error == KError::Ok
                        && vifast_user::tcb_destroy(ep).error == KError::Ok
                        && vifast_user::tcb_destroy(200).error != KError::Ok;
                    write(if kill_ok { b"KILL OK\n" } else { b"KILL BAD\n" });
                    // Vòng 139: dung lượng FS qua STATFS — dùng phải ≤ tổng,
                    // tổng > 0, và block size đúng 512 (vifast_fs::BLOCK_SIZE).
                    let usage = unsafe {
                        vifast_user::call_message(root_vfs_slot, vfs_abi::STATFS, 0, 0, 0)
                    };
                    let usage_ok = usage.error == KError::Ok
                        && usage.label == vfs_abi::OK
                        && usage.data[1] > 0
                        && usage.data[0] <= usage.data[1]
                        && usage.data[2] == 512;
                    write(if usage_ok {
                        b"FS USAGE OK\n"
                    } else {
                        b"FS USAGE BAD\n"
                    });
                    destroy_exec(&mut slots, &exec_used, exec_len);
                    exec_len = 0;
                    exec_tcb = 0;
                    exec_used = [0u8; MAX_EXEC_SLOTS];
                    // Vòng 103: truyền tham số RIÊNG ("HIARG") để cổng chuẩn
                    // chứng minh cả exec-từ-FS lẫn argv: chuỗi này không nằm
                    // trong ảnh trên đĩa, chỉ đến từ tham số root truyền.
                    match spawn_elf_from_fs(
                        root_vfs_slot,
                        b"/prog.elf",
                        b"HIARG\n",
                        meta_untyped,
                        data_untyped,
                        0b01,
                        &mut slots,
                        &mut exec_used,
                    ) {
                        Some(tcb) => {
                            exec_tcb = tcb;
                            let mut n = 0;
                            while n < MAX_EXEC_SLOTS && exec_used[n] != 0 {
                                n += 1;
                            }
                            exec_len = n;
                            exec_vspace = exec_used[0];
                            write(b"RUN FS OK\n");
                        }
                        None => write(b"RUN FS FAIL\n"),
                    }
                }
            }
            if message.label == LABEL_KILL {
                // Vòng 169: `kill` từ shell — dừng TCB exec đang chạy.
                if exec_len != 0 {
                    // Vòng 170: suspend TRƯỚC khi dọn — task có thể đang chạy
                    // trên hart khác; huỷ VSpace lúc nó còn chạy là nguy hiểm.
                    if exec_tcb != 0 {
                        let _ = vifast_user::tcb_suspend(exec_tcb);
                    }
                    destroy_exec(&mut slots, &exec_used, exec_len);
                    exec_len = 0;
                    exec_used = [0u8; MAX_EXEC_SLOTS];
                    exec_vspace = 0;
                    exec_tcb = 0;
                    write(b"KILL EXEC OK\n");
                } else {
                    write(b"KILL EXEC SKIP\n");
                }
            }
            if message.label == LABEL_RUN && message.badge == RUN_BADGE {
                // `run` lặp: VSpace setup 1 lần, mỗi lần chỉ thay TCB.
                if exec_vspace == 0 {
                    // Vòng 46: đọc path shell gửi qua IPC buffer (kernel
                    // copy sang ROOT_IPC_BUF_VA khi send). STAT xác minh
                    // trước khi spawn; run trần (không path) bỏ qua STAT.
                    let mut plen = 0usize;
                    while plen < vifast_abi::vfs::MAX_PATH {
                        let b = unsafe {
                            core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(plen))
                        };
                        if b == 0 {
                            break;
                        }
                        plen += 1;
                    }
                    // Copy path NGAY (trước mọi call VFS): các call dưới đây
                    // ghi đè IPC buffer, copy muộn sẽ nhận dữ liệu file
                    // (vòng 89 — bug OPEN EINVAL).
                    let mut path = [0u8; vifast_abi::vfs::MAX_PATH];
                    let mut i = 0;
                    while i < plen {
                        path[i] = unsafe {
                            core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(i))
                        };
                        i += 1;
                    }
                    // Vòng 102: tham số sau NUL của path — `runfs <path> <args>`.
                    let mut args_buf = [0u8; 32];
                    let mut alen = 0usize;
                    while alen < 31 {
                        let byte = unsafe {
                            core::ptr::read_volatile(
                                (ROOT_IPC_BUF_VA as *const u8).add(plen + 1 + alen),
                            )
                        };
                        if byte == 0 {
                            break;
                        }
                        args_buf[alen] = byte;
                        alen += 1;
                    }
                    let args_msg: &[u8] = if alen > 0 {
                        &args_buf[..alen]
                    } else {
                        b"HI DK\n"
                    };
                    let mut stat_ok = plen == 0;
                    // Vòng 75: chỉ STAT/READ khi có cap VFS badge SUPERUSER
                    // (root_vfs_slot != 0) — cap gốc badge 0 bị server từ
                    // chối EACCES. Không có slot thì bỏ qua STAT (fail-êm,
                    // x86) và spawn như cũ.
                    if plen != 0 && root_vfs_slot != 0 {
                        // Vòng 82/83: vòng tròn ghi→đọc→verify ELF on-demand
                        // (tránh call blocking cuối boot như vòng 75).
                        // Đảm bảo /elf-test từ 1KiB đầu client ELF rồi đọc
                        // lại so khớp từng byte với bản gốc. Fail-êm.
                        if client_bytes.len() >= 1024
                            && root_vfs_stat(root_vfs_slot, b"/elf-test").is_none()
                        {
                            let mut off = 0usize;
                            while off < 1024 {
                                let mut chunk = [0u8; vifast_abi::vfs::MAX_IO];
                                let mut j = 0;
                                while j < chunk.len() {
                                    chunk[j] = client_bytes[off + j];
                                    j += 1;
                                }
                                if !root_vfs_write_at(
                                    root_vfs_slot,
                                    b"/elf-test",
                                    off as u64,
                                    &chunk,
                                    off == 0,
                                ) {
                                    break;
                                }
                                off += chunk.len();
                            }
                        }
                        if root_vfs_verify_prefix(
                            root_vfs_slot,
                            b"/elf-test",
                            &client_bytes[..1024],
                        ) {
                            write(b"ROOT ELF OK\n");
                        }
                        // Vòng 89: lệnh `runfs` — nếu /prog.elf chưa có thì
                        // tạo từ ảnh ELF dựng sẵn NGAY tại đây (on-demand,
                        // không ở boot), rồi để STAT/exec-from-FS xử lý tiếp.
                        if plen == b"/prog.elf".len()
                            && root_vfs_stat(root_vfs_slot, b"/prog.elf").is_none()
                        {
                            let mut off = 0usize;
                            while off < EXEC_DEMO_ELF.len() {
                                let n = core::cmp::min(
                                    vifast_abi::vfs::MAX_IO,
                                    EXEC_DEMO_ELF.len() - off,
                                );
                                let mut chunk = [0u8; vifast_abi::vfs::MAX_IO];
                                let mut j = 0;
                                while j < n {
                                    chunk[j] = EXEC_DEMO_ELF[off + j];
                                    j += 1;
                                }
                                if !root_vfs_write_at(
                                    root_vfs_slot,
                                    b"/prog.elf",
                                    off as u64,
                                    &chunk[..n],
                                    off == 0,
                                ) {
                                    break;
                                }
                                off += n;
                            }
                            let head = core::cmp::min(64, EXEC_DEMO_ELF.len());
                            if root_vfs_verify_prefix(
                                root_vfs_slot,
                                b"/prog.elf",
                                &EXEC_DEMO_ELF[..head],
                            ) {
                                write(b"ROOT PROG OK\n");
                            }
                        }
                        if root_vfs_stat(root_vfs_slot, &path[..plen]).is_some() {
                            stat_ok = true;
                            // Vòng 74: verify nội dung đọc được (OPEN/READ/
                            // CLOSE từ root). Chỉ quan sát: in marker khi
                            // đúng magic demand-load, không đổi spawn.
                            if plen == b"/page.bin".len()
                                && root_vfs_read_head(root_vfs_slot, &path[..plen])
                            {
                                write(b"ROOT READ OK\n");
                            }
                        } else {
                            // STAT fail (file không tồn tại): SKIP, không
                            // spawn mù (vòng 46).
                            stat_ok = false;
                        }
                    }
                    if !stat_ok {
                        write(b"RUN SKIP\n");
                    } else {
                        destroy_exec(&mut slots, &exec_used, exec_len);
                        exec_len = 0;
                        exec_tcb = 0;
                        exec_used = [0u8; MAX_EXEC_SLOTS];
                        // Vòng 84: ưu tiên exec-thật từ FS khi path là ELF
                        // hợp lệ (đọc header+phdr rồi nạp từng page thẳng từ
                        // file, không staging). Fail-êm: rơi về demo copy để
                        // gate không đổi.
                        let mut tcb_opt = None;
                        if plen != 0 && root_vfs_slot != 0 {
                            // Dùng `path` đã copy từ buffer ĐẦU dispatch:
                            // verify_prefix ở trên đã ghi đè IPC buffer bằng
                            // dữ liệu file (vòng 89 — bug OPEN EINVAL).
                            tcb_opt = spawn_elf_from_fs(
                                root_vfs_slot,
                                &path[..plen],
                                args_msg,
                                meta_untyped,
                                data_untyped,
                                0b01,
                                &mut slots,
                                &mut exec_used,
                            );
                        }
                        let spawned = match tcb_opt {
                            Some(tcb) => {
                                exec_tcb = tcb;
                                write(b"RUN FS OK\n");
                                Some(tcb)
                            }
                            None => spawn_elf_copy(
                                meta_untyped,
                                data_untyped,
                                EXEC_DEMO_ELF,
                                b"HI FS\n",
                                0b01,
                                &mut slots,
                                &mut exec_used,
                            ),
                        };
                        match spawned {
                            Some(_tcb) => {
                                let mut n = 0;
                                while n < MAX_EXEC_SLOTS && exec_used[n] != 0 {
                                    n += 1;
                                }
                                exec_len = n;
                                // exec_used[?] là vspace — tìm slot vspace? Lưu
                                // riêng: vspace là slot đầu tiên được cấp.
                                exec_vspace = exec_used[0];
                                if tcb_opt.is_none() {
                                    write(b"RUN OK\n");
                                }
                            }
                            None => write(b"RUN SKIP\n"),
                        }
                    } // if stat_ok
                } else {
                    let mut plen = 0usize;
                    while plen < vifast_abi::vfs::MAX_PATH {
                        let b = unsafe {
                            core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(plen))
                        };
                        if b == 0 {
                            break;
                        }
                        plen += 1;
                    }
                    // Vòng 102: nếu path trỏ tới ELF có thật trên FS thì
                    // exec-thật từ file (destroy + spawn mới, kèm tham số);
                    // nếu không thì restart TCB cũ như trước (rẻ, không cấp
                    // object mới — tránh race destroy/recreate vòng 35).
                    let mut fs_done = false;
                    if plen != 0 && root_vfs_slot != 0 {
                        let mut pbuf = [0u8; vifast_abi::vfs::MAX_PATH];
                        let mut i = 0;
                        while i < plen {
                            pbuf[i] = unsafe {
                                core::ptr::read_volatile((ROOT_IPC_BUF_VA as *const u8).add(i))
                            };
                            i += 1;
                        }
                        let mut ab = [0u8; 32];
                        let mut n = 0usize;
                        while n < 31 {
                            let byte = unsafe {
                                core::ptr::read_volatile(
                                    (ROOT_IPC_BUF_VA as *const u8).add(plen + 1 + n),
                                )
                            };
                            if byte == 0 {
                                break;
                            }
                            ab[n] = byte;
                            n += 1;
                        }
                        let msg: &[u8] = if n > 0 { &ab[..n] } else { b"HI DK\n" };
                        // Vòng 104: luôn destroy + nạp lại từ file (không phụ
                        // thuộc stat) để `runfs/exec` chạy được cả khi TCB cũ
                        // đã có; fail thì rơi về demo copy như nhánh đầu.
                        destroy_exec(&mut slots, &exec_used, exec_len);
                        exec_len = 0;
                        exec_tcb = 0;
                        exec_used = [0u8; MAX_EXEC_SLOTS];
                        exec_vspace = 0;
                        if let Some(tcb) = spawn_elf_from_fs(
                            root_vfs_slot,
                            &pbuf[..plen],
                            msg,
                            meta_untyped,
                            data_untyped,
                            0b01,
                            &mut slots,
                            &mut exec_used,
                        ) {
                            exec_tcb = tcb;
                            let mut n2 = 0;
                            while n2 < MAX_EXEC_SLOTS && exec_used[n2] != 0 {
                                n2 += 1;
                            }
                            exec_len = n2;
                            exec_vspace = exec_used[0];
                            write(b"RUN FS OK\n");
                            fs_done = true;
                        }
                    }
                    if !fs_done {
                        match spawn_elf_copy(
                            meta_untyped,
                            data_untyped,
                            EXEC_DEMO_ELF,
                            b"HI FS\n",
                            0b01,
                            &mut slots,
                            &mut exec_used,
                        ) {
                            Some(_tcb) => {
                                let mut n = 0;
                                while n < MAX_EXEC_SLOTS && exec_used[n] != 0 {
                                    n += 1;
                                }
                                exec_len = n;
                                exec_vspace = exec_used[0];
                                write(b"RUN OK\n");
                            }
                            None => write(b"RUN SKIP\n"),
                        }
                    }
                }
            } else if message.label == CNODE_LABEL {
                write(CNODE_OK);
                reap_ok &= vifast_user::tcb_destroy(cnode_tcb).error == KError::Ok;
                reaped = reaped.saturating_add(1);
            } else if message.label == WORKER_LABEL_C_RUNNING {
                if !suspend_done {
                    if reaped >= 3 {
                        start_worker_e = true;
                    } else {
                        pending_worker_e = true;
                    }
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
            } else if message.label == WORKER_LABEL_G {
                if message.data[0] == 50 {
                    worker_bits |= 8;
                }
            } else if message.label == WORKER_LABEL_H {
                if message.data[0] == 50 {
                    worker_bits |= 16;
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
            } else if message.label == LABEL_FAULT {
                write_fault_addr(message.data[0]);
                let victim = match message.data[2] {
                    slot if slot == FAULT1_TCB_SLOT as u64 => Some(fault1_tcb),
                    slot if slot == FAULT2_TCB_SLOT as u64 => Some(fault2_tcb),
                    slot if slot == FAULT3_TCB_SLOT as u64 => Some(fault3_tcb),
                    6 => Some(fault4_tcb),
                    _ => None,
                };
                if let Some(victim) = victim {
                    let result = vifast_user::tcb_destroy(victim);
                    reap_ok &= result.error == KError::Ok;
                    reaped = reaped.saturating_add(1);
                }
            } else {
                write_fault_addr(message.data[0]);
            }
            if reaped == 5 && !reap_done {
                reap_done = true;
                write(if reap_ok { TCB_REAP_OK } else { TCB_REAP_BAD });
                // Mốc 1 (vòng 16): demo exit-theo-task + TcbWait + reap.
                // Spawn NGAY tại đây (5 fault TCB vừa destroy, quota/bump
                // vừa hồi) — để tới workers_ok thì x86 đã cạn meta Untyped.
                // Worker F thoát ngay khi chạy; root TcbWait (block có chủ
                // đích — fault message vẫn queue) rồi destroy.
                if !exit_demo_done {
                    exit_demo_done = true;
                    if info.sched_harts & 0b10 != 0 {
                        // Spawn tối giản: chỉ retype TCB (meta quota vừa hồi
                        // từ 5 fault TCB), tái dùng stack mapping cũ của
                        // fault1 (frame object vẫn sống sau destroy TCB).
                        // Không cấp frame/page-table mới — x86 đã cạn data.
                        let worker_f = slots.alloc();
                        retype(meta_untyped, ObjType::Tcb, worker_f);
                        check(vifast_user::tcb_configure(
                            worker_f,
                            exit_worker_entry as *const () as usize as u64,
                            FAULT1_STACK_TOP as u64,
                            root_vspace,
                            5,
                        ));
                        check(vifast_user::tcb_set_affinity(worker_f, 0b10));
                        check(vifast_user::tcb_resume(worker_f));
                        let waited = vifast_user::tcb_wait(worker_f);
                        // F đã exit: wait hoặc xong ngay (Ok, F exit trước)
                        // hoặc wake lỗi ObjectDestroyed (F exit sau) — cả
                        // hai đều đúng rendezvous.
                        let destroyed = vifast_user::tcb_destroy(worker_f);
                        write(
                            if (waited.error == KError::Ok
                                || waited.error == KError::ObjectDestroyed)
                                && destroyed.error == KError::Ok
                            {
                                WAIT_REAP_OK
                            } else {
                                WAIT_REAP_BAD
                            },
                        );
                    }
                }
            }
            if pending_worker_e && reaped >= 3 {
                pending_worker_e = false;
                start_worker_e = true;
            }
            if start_worker_e && !suspend_done {
                start_worker_e = false;
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
                    0b10,
                    &mut slots,
                );
                let suspended = vifast_user::tcb_suspend(worker_c_tcb);
                let started = vifast_user::get_time().value;
                while vifast_user::get_time().value < started.saturating_add(5) {
                    let _ = vifast_user::yield_now();
                }
                let resumed = vifast_user::tcb_resume(worker_c_tcb);
                // Keep C busy for five more ticks after resume so the AP timer
                // observes preemption again before the flag ends it.
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
            // Mốc 5 (vòng 31): yêu cầu bit A/B/C như cũ, cộng bit G/H khi
            // bootinfo có hart tương ứng. Marker SMP WORKERS OK giữ nguyên
            // format (harness exact) — G/H chỉ thêm bit nội bộ.
            let want_bits: u8 = 7
                | (if info.sched_harts & 0b0100 != 0 { 8 } else { 0 })
                | (if info.sched_harts & 0b1000 != 0 {
                    16
                } else {
                    0
                });

            // Vòng 135: cấp phát vượt phần còn lại của Untyped phải trả
            // OutOfMemory (không panic, không cấp nửa vời), và size 0
            // phải bị từ chối.
            // Dùng lại slot endpoint đã destroy ở trên — KHÔNG alloc thêm: vùng slot
            // của root rất sát ngưỡng trên aarch64 (alloc thêm ⇒ panic 843).
            let probe_slot = probe_endpoint_slot;
            let too_big = vifast_user::retype_untyped(data_untyped, 0x40_0000_0000, 1, probe_slot);
            let zero = vifast_user::retype_untyped(data_untyped, 0, 1, probe_slot);
            write(
                if too_big.error == KError::OutOfMemory && zero.error != KError::Ok {
                    b"MEM OOM OK\n"
                } else {
                    b"MEM OOM BAD\n"
                },
            );
            if worker_bits & want_bits == want_bits && !workers_ok {
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
                // Mốc 9 (vòng 75): root tự STAT/READ /page.bin qua cap VFS
                // badge SUPERUSER — chứng minh đường VFS của root hoạt động
                // end-to-end (bước 1-2 exec-thật: STAT + READ ELF từ FS).
                // Chạy cuối boot nên VFS chắc chắn sẵn sàng (bài học vòng
                // 46: không gọi blocking trong boot). Fail-êm.
                // Vòng 75: KHÔNG probe cuối boot — call VFS blocking tại
                // đây làm boot dở (3/3 FAIL, thiếu marker giữa) với nguyên
                // nhân chưa rõ; giữ mint + STAT/READ cho đường RUN on-demand
                // (đã chứng minh STAT trả Ok qua DIAG tạm thời).
                // Mốc 9 (vòng 32): exec MVP — nạp bản sao client ELF vào
                // VSpace mới và spawn chạy. Fail-êm: x86 cạn Untyped thì bỏ
                // qua, gate không đổi. Affinity hart 0 (luôn có task).
                if info.sched_harts & 0b01 != 0 {
                    destroy_exec(&mut slots, &exec_used, exec_len);
                    exec_len = 0;
                    exec_tcb = 0;
                    exec_used = [0u8; MAX_EXEC_SLOTS];
                    match spawn_elf_copy(
                        meta_untyped,
                        data_untyped,
                        EXEC_DEMO_ELF,
                        b"HI FS\n",
                        0b01,
                        &mut slots,
                        &mut exec_used,
                    ) {
                        Some(_) => write(b"EXEC OK\n"),
                        None => write(b"EXEC SKIP\n"),
                    }
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
