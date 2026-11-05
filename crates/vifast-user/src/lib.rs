//! Thin user-space syscall wrappers for all ViFastOs architectures.
//!
//! The wrappers only encode the ABI register convention.  Policy, capability
//! lookup and validation stay in the portable kernel.

#![no_std]

use vifast_abi::{KError, ObjType, SyscallId, CAP_NONE};

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
            inlateout("rsi") args[1] => _,
            inlateout("rdx") args[2] => _,
            inlateout("rcx") args[3] => _,
            inlateout("r8") args[4] => _,
            inlateout("r9") args[5] => _,
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
            inlateout("x2") args[2] => _,
            inlateout("x3") args[3] => _,
            inlateout("x4") args[4] => _,
            inlateout("x5") args[5] => _,
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
            inlateout("a2") args[2] => _,
            inlateout("a3") args[3] => _,
            inlateout("a4") args[4] => _,
            inlateout("a5") args[5] => _,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageResult {
    pub error: KError,
    pub value: u64,
    pub label: u64,
    pub data: [u64; 4],
}

pub unsafe fn recv_message(endpoint_slot: u8, recv_slot: u8) -> MessageResult {
    #[cfg(target_arch = "x86_64")]
    {
        let error: u64;
        let value: u64;
        let label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") SyscallId::Recv as u64 => error,
            inlateout("rdi") endpoint_slot as u64 => value,
            inlateout("rsi") recv_slot as u64 => label,
            lateout("rdx") d0,
            lateout("rcx") d1,
            lateout("r8") d2,
            lateout("r9") d3,
            lateout("r10") _,
            lateout("r11") _,
            lateout("r12") _,
            lateout("r13") _,
            lateout("r14") _,
            lateout("r15") _,
        );
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let error: u64;
        let value: u64;
        let label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "svc #0",
            in("x8") SyscallId::Recv as u64,
            inlateout("x0") endpoint_slot as u64 => error,
            inlateout("x1") recv_slot as u64 => value,
            lateout("x2") label,
            lateout("x3") d0,
            lateout("x4") d1,
            lateout("x5") d2,
            lateout("x6") d3,
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
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        let error: u64;
        let value: u64;
        let label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "ecall",
            in("a7") SyscallId::Recv as u64,
            inlateout("a0") endpoint_slot as u64 => error,
            inlateout("a1") recv_slot as u64 => value,
            lateout("a2") label,
            lateout("a3") d0,
            lateout("a4") d1,
            lateout("a5") d2,
            lateout("a6") d3,
            lateout("t0") _,
            lateout("t1") _,
            lateout("t2") _,
            lateout("t3") _,
            lateout("t4") _,
            lateout("t5") _,
            lateout("t6") _,
        );
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        let _ = (endpoint_slot, recv_slot);
        MessageResult {
            error: KError::NotImplemented,
            value: 0,
            label: 0,
            data: [0; 4],
        }
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

pub fn frame_write(frame_slot: u8, offset: u64, ptr: u64, len: u64) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::FrameWrite as u64,
            [frame_slot as u64, offset, ptr, len, 0, 0],
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

pub fn block_read(lba: u64, ptr: u64) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::BlockRead as u64, [lba, ptr, 0, 0, 0, 0]) };
    result(e, v)
}

pub fn block_write(lba: u64, ptr: u64) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::BlockWrite as u64, [lba, ptr, 0, 0, 0, 0]) };
    result(e, v)
}

pub fn iommu_map(domain_slot: u8, iova: u64, frame_slot: u8, rights: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::IoMap as u64,
            [
                domain_slot as u64,
                iova,
                frame_slot as u64,
                rights as u64,
                0,
                0,
            ],
        )
    };
    result(e, v)
}

pub fn iommu_unmap(domain_slot: u8, iova: u64) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::IoUnmap as u64,
            [domain_slot as u64, iova, 0, 0, 0, 0],
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

pub fn tcb_configure_fault(
    tcb_slot: u8,
    entry: u64,
    stack: u64,
    vspace_slot: u8,
    priority: u8,
    fault_endpoint_slot: u8,
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
                fault_endpoint_slot as u64,
            ],
        )
    };
    result(e, v)
}

pub fn tcb_set_affinity(tcb_slot: u8, mask: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::TcbSetAffinity as u64,
            [tcb_slot as u64, mask as u64, 0, 0, 0, 0],
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

pub fn send(endpoint_slot: u8, label: u64, data0: u64, data1: u64, data2: u64) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::Send as u64,
            [endpoint_slot as u64, label, data0, data1, data2, CAP_NONE],
        )
    };
    result(e, v)
}

/// Blocking Call.  The reply is delivered through the normal message
/// registers when the server eventually replies.
/// Call and retain the reply label/data words.  A blocking Call resumes
/// through the normal message registers, so this is the only way to consume
/// request/reply payloads (the plain `call` wrapper keeps only error/value).
pub unsafe fn call_message(
    endpoint_slot: u8,
    label: u64,
    data0: u64,
    data1: u64,
    data2: u64,
) -> MessageResult {
    let cap = CAP_NONE;
    #[cfg(target_arch = "x86_64")]
    {
        let error: u64;
        let value: u64;
        let out_label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") SyscallId::Call as u64 => error,
            inlateout("rdi") endpoint_slot as u64 => value,
            inlateout("rsi") label => out_label,
            inlateout("rdx") data0 => d0,
            inlateout("rcx") data1 => d1,
            inlateout("r8") data2 => d2,
            inlateout("r9") cap => d3,
            lateout("r10") _,
            lateout("r11") _,
            lateout("r12") _,
            lateout("r13") _,
            lateout("r14") _,
            lateout("r15") _,
        );
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label: out_label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let error: u64;
        let value: u64;
        let out_label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "svc #0",
            in("x8") SyscallId::Call as u64,
            inlateout("x0") endpoint_slot as u64 => error,
            inlateout("x1") label => value,
            inlateout("x2") data0 => out_label,
            inlateout("x3") data1 => d0,
            inlateout("x4") data2 => d1,
            inlateout("x5") cap => d2,
            lateout("x6") d3,
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
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label: out_label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        let error: u64;
        let value: u64;
        let out_label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "ecall",
            in("a7") SyscallId::Call as u64,
            inlateout("a0") endpoint_slot as u64 => error,
            inlateout("a1") label => value,
            inlateout("a2") data0 => out_label,
            inlateout("a3") data1 => d0,
            inlateout("a4") data2 => d1,
            inlateout("a5") cap => d2,
            lateout("a6") d3,
            lateout("t0") _,
            lateout("t1") _,
            lateout("t2") _,
            lateout("t3") _,
            lateout("t4") _,
            lateout("t5") _,
            lateout("t6") _,
        );
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label: out_label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        let _ = (endpoint_slot, label, data0, data1, data2);
        MessageResult {
            error: KError::NotImplemented,
            value: 0,
            label: 0,
            data: [0; 4],
        }
    }
}

/// Reply to the pending caller and receive the next message, preserving the
/// reply label/data words of the next request.
pub unsafe fn reply_recv_message(
    endpoint_slot: u8,
    recv_slot: u8,
    reply_label: u64,
    reply0: u64,
    reply1: u64,
    reply2: u64,
) -> MessageResult {
    #[cfg(target_arch = "x86_64")]
    {
        let error: u64;
        let value: u64;
        let out_label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") SyscallId::ReplyRecv as u64 => error,
            inlateout("rdi") endpoint_slot as u64 => value,
            inlateout("rsi") recv_slot as u64 => out_label,
            inlateout("rdx") reply_label => d0,
            inlateout("rcx") reply0 => d1,
            inlateout("r8") reply1 => d2,
            inlateout("r9") reply2 => d3,
            lateout("r10") _,
            lateout("r11") _,
            lateout("r12") _,
            lateout("r13") _,
            lateout("r14") _,
            lateout("r15") _,
        );
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label: out_label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let error: u64;
        let value: u64;
        let out_label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "svc #0",
            in("x8") SyscallId::ReplyRecv as u64,
            inlateout("x0") endpoint_slot as u64 => error,
            inlateout("x1") recv_slot as u64 => value,
            inlateout("x2") reply_label => out_label,
            inlateout("x3") reply0 => d0,
            inlateout("x4") reply1 => d1,
            inlateout("x5") reply2 => d2,
            lateout("x6") d3,
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
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label: out_label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(target_arch = "riscv64")]
    {
        let error: u64;
        let value: u64;
        let out_label: u64;
        let d0: u64;
        let d1: u64;
        let d2: u64;
        let d3: u64;
        core::arch::asm!(
            "ecall",
            in("a7") SyscallId::ReplyRecv as u64,
            inlateout("a0") endpoint_slot as u64 => error,
            inlateout("a1") recv_slot as u64 => value,
            inlateout("a2") reply_label => out_label,
            inlateout("a3") reply0 => d0,
            inlateout("a4") reply1 => d1,
            inlateout("a5") reply2 => d2,
            lateout("a6") d3,
            lateout("t0") _,
            lateout("t1") _,
            lateout("t2") _,
            lateout("t3") _,
            lateout("t4") _,
            lateout("t5") _,
            lateout("t6") _,
        );
        MessageResult {
            error: KError::from_u32(error as u32),
            value,
            label: out_label,
            data: [d0, d1, d2, d3],
        }
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        let _ = (
            endpoint_slot,
            recv_slot,
            reply_label,
            reply0,
            reply1,
            reply2,
        );
        MessageResult {
            error: KError::NotImplemented,
            value: 0,
            label: 0,
            data: [0; 4],
        }
    }
}

pub fn call(endpoint_slot: u8, label: u64, data0: u64, data1: u64, data2: u64) -> SyscallResult {
    call_with_cap(endpoint_slot, label, data0, data1, data2, CAP_NONE)
}

/// Call with an attached capability slot.  `cap_slot` must be `CAP_NONE`
/// when no capability should be transferred.
pub fn call_with_cap(
    endpoint_slot: u8,
    label: u64,
    data0: u64,
    data1: u64,
    data2: u64,
    cap_slot: u64,
) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::Call as u64,
            [endpoint_slot as u64, label, data0, data1, data2, cap_slot],
        )
    };
    result(e, v)
}

pub fn poll(notification_slot: u8) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::Poll as u64,
            [notification_slot as u64, 0, 0, 0, 0, 0],
        )
    };
    result(e, v)
}

pub fn open(path: u64, len: u64, flags: u64) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::Open as u64, [path, len, flags, 0, 0, 0]) };
    result(e, v)
}

pub fn read(fd: u64, buffer: u64, len: u64) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::Read as u64, [fd, buffer, len, 0, 0, 0]) };
    result(e, v)
}

pub fn write(fd: u64, buffer: u64, len: u64) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::Write as u64, [fd, buffer, len, 0, 0, 0]) };
    result(e, v)
}

pub fn close(fd: u64) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::Close as u64, [fd, 0, 0, 0, 0, 0]) };
    result(e, v)
}

pub fn seek(fd: u64, offset: u64, whence: u64) -> SyscallResult {
    let (e, v) = unsafe { syscall(SyscallId::Seek as u64, [fd, offset, whence, 0, 0, 0]) };
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

pub fn tcb_set_ipc_buffer(tcb_slot: u8, frame_slot: u8, va: u64) -> SyscallResult {
    let (e, v) = unsafe {
        syscall(
            SyscallId::TcbSetIpcBuffer as u64,
            [tcb_slot as u64, frame_slot as u64, va, 0, 0, 0],
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
