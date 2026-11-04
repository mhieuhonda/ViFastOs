//! Thin user-space syscall wrappers for all ViFastOs architectures.
//!
//! The wrappers only encode the ABI register convention.  Policy, capability
//! lookup and validation stay in the portable kernel.

#![no_std]

use vifast_abi::{KError, ObjType, SyscallId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyscallResult {
    pub error: KError,
    pub value: u64,
}

impl SyscallResult {
    pub const fn ok(value: u64) -> Self {
        Self {
            error: KError::Ok,
            value,
        }
    }
}

/// Raw syscall.  The return tuple is `(error, value)` as defined by the ABI.
#[inline]
pub unsafe fn syscall(number: u64, args: [u64; 6]) -> (u64, u64) {
    #[cfg(target_arch = "x86_64")]
    {
        let error: u64;
        let value: u64;
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") number => error,
            inlateout("rdi") args[0] => value,
            in("rsi") args[1],
            in("rdx") args[2],
            in("rcx") args[3],
            in("r8") args[4],
            in("r9") args[5],
            lateout("r10") _,
            lateout("r11") _,
            lateout("r12") _,
            lateout("r13") _,
            lateout("r14") _,
            lateout("r15") _,
        );
        (error, value)
    }
    #[cfg(target_arch = "aarch64")]
    {
        let error: u64;
        let value: u64;
        core::arch::asm!(
            "svc #0",
            in("x8") number,
            inlateout("x0") args[0] => error,
            inlateout("x1") args[1] => value,
            in("x2") args[2],
            in("x3") args[3],
            in("x4") args[4],
            in("x5") args[5],
            lateout("x6") _,
            lateout("x7") _,
            lateout("x9") _,
            lateout("x10") _,
            lateout("x11") _,
            lateout("x12") _,
            lateout("x13") _,
            lateout("x14") _,
            lateout("x15") _,
            lateout("x16") _,
            lateout("x17") _,
        );
        (error, value)
    }
    #[cfg(target_arch = "riscv64")]
    {
        let error: u64;
        let value: u64;
        core::arch::asm!(
            "ecall",
            in("a7") number,
            inlateout("a0") args[0] => error,
            inlateout("a1") args[1] => value,
            in("a2") args[2],
            in("a3") args[3],
            in("a4") args[4],
            in("a5") args[5],
            lateout("a6") _,
            lateout("t0") _,
            lateout("t1") _,
            lateout("t2") _,
            lateout("t3") _,
            lateout("t4") _,
            lateout("t5") _,
            lateout("t6") _,
        );
        (error, value)
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        let _ = (number, args);
        (KError::NotImplemented as u64, 0)
    }
}

fn result(error: u64, value: u64) -> SyscallResult {
    SyscallResult {
        error: KError::from_u32(error as u32),
        value,
    }
}

pub fn debug_write(ptr: u64, len: u64) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::DebugWrite as u64, [ptr, len, 0, 0, 0, 0]) };
    result(e, v)
}

pub fn get_time() -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::GetTime as u64, [0; 6]) };
    result(e, v)
}

pub fn yield_now() -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::Yield as u64, [0; 6]) };
    result(e, v)
}

pub fn retype(untyped_slot: u8, ty: u64, dest_slot: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::Retype as u64,
            [untyped_slot as u64, ty, dest_slot as u64, 0, 0, 0],
        )
    };
    result(e, v)
}

pub fn retype_untyped(
    untyped_slot: u8,
    size_bytes: u64,
    child_max_objects: u64,
    dest_slot: u8,
) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::Retype as u64,
            [
                untyped_slot as u64,
                ObjType::Untyped as u64,
                dest_slot as u64,
                size_bytes,
                child_max_objects,
                0,
            ],
        )
    };
    result(e, v)
}

pub fn cap_copy(src_slot: u8, dst_tcb_slot: u8, dst_slot: u8, rights: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::CapCopy as u64,
            [
                src_slot as u64,
                dst_tcb_slot as u64,
                dst_slot as u64,
                rights as u64,
                0,
                0,
            ],
        )
    };
    result(e, v)
}

pub fn page_table_map(page_table_slot: u8, vspace_slot: u8, va: u64) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::PageTableMap as u64,
            [page_table_slot as u64, vspace_slot as u64, va, 0, 0, 0],
        )
    };
    result(e, v)
}

pub fn frame_map(vspace_slot: u8, frame_slot: u8, va: u64, rights: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::FrameMap as u64,
            [
                vspace_slot as u64,
                frame_slot as u64,
                va,
                rights as u64,
                0,
                0,
            ],
        )
    };
    result(e, v)
}

pub fn frame_unmap(vspace_slot: u8, va: u64, frame_slot: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::FrameUnmap as u64,
            [vspace_slot as u64, va, frame_slot as u64, 0, 0, 0],
        )
    };
    result(e, v)
}

pub fn tcb_configure(
    tcb_slot: u8,
    entry: u64,
    stack: u64,
    vspace_slot: u8,
    priority: u8,
) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::TcbConfigure as u64,
            [
                tcb_slot as u64,
                entry,
                stack,
                vspace_slot as u64,
                priority as u64,
                0,
            ],
        )
    };
    result(e, v)
}

pub fn cap_revoke(slot: u8) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::CapRevoke as u64, [slot as u64, 0, 0, 0, 0, 0]) };
    result(e, v)
}

pub fn wait(notification_slot: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::Wait as u64,
            [notification_slot as u64, 0, 0, 0, 0, 0],
        )
    };
    result(e, v)
}

pub fn reply_recv(
    endpoint_slot: u8,
    recv_slot: u8,
    label: u64,
    data0: u64,
    data1: u64,
    data2: u64,
) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::ReplyRecv as u64,
            [
                endpoint_slot as u64,
                recv_slot as u64,
                label,
                data0,
                data1,
                data2,
            ],
        )
    };
    result(e, v)
}

pub fn signal(notification_slot: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::Signal as u64,
            [notification_slot as u64, 0, 0, 0, 0, 0],
        )
    };
    result(e, v)
}

pub fn tcb_resume(tcb_slot: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::TcbResume as u64,
            [tcb_slot as u64, 0, 0, 0, 0, 0],
        )
    };
    result(e, v)
}

pub fn exit() -> ! {
    unsafe {
        let _ = syscall(SyscallId::Exit as u64, [0; 6]);
    }
    loop {
        core::hint::spin_loop();
    }
}
