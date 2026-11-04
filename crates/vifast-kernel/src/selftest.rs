//! Portable kernel self-tests.
//!
//! These run in kernel mode on every architecture before any user thread is
//! started.  They are intentionally deterministic and allocation-free so the
//! same code can run on the host, in QEMU and on real hardware.

use crate::console::Console;
use vifast_abi::*;

use crate::object::{Cap, ObjRef};
use crate::{Kernel, MAX_CAPS};

pub fn run<A: Console>(k: &mut Kernel) -> bool {
    let mut passed = true;

    macro_rules! check {
        ($cond:expr, $name:expr) => {
            if $cond {
                crate::klog!(A, "[ok]   {}", $name);
            } else {
                crate::klog!(A, "[FAIL] {}", $name);
                passed = false;
            }
        };
    }

    let Some(root) = k.root() else {
        crate::klog!(A, "[FAIL] no root TCB");
        return false;
    };

    // 1. Capability derivation and revocation.
    let Some(endpoint) = k.create_endpoint() else {
        crate::klog!(A, "[FAIL] object table exhausted");
        return false;
    };
    let base = Cap::new(endpoint, ObjType::Endpoint, Rights::ALL);
    check!(
        k.install_cap(root, 1, base).is_ok(),
        "install endpoint capability"
    );
    check!(
        k.copy_cap(root, 1, root, 2, Rights::READ, 0).is_ok(),
        "copy capability with reduced rights"
    );
    check!(
        k.lookup_cap(root, 2, Some(ObjType::Endpoint), Rights::READ)
            .is_ok(),
        "lookup read-only copy"
    );
    check!(
        k.lookup_cap(root, 2, Some(ObjType::Endpoint), Rights::WRITE) == Err(KError::NoRights),
        "rights cannot increase"
    );
    check!(
        k.revoke_cap(root, 1).unwrap_or(0) == 1,
        "revoke removes derived capability"
    );
    check!(
        k.lookup_cap(root, 2, None, Rights::NONE).is_err(),
        "revoked capability is invalid"
    );
    check!(
        k.lookup_cap(root, 1, None, Rights::NONE).is_ok(),
        "revoke keeps the root capability"
    );
    let _ = k.delete_cap(root, 1);

    // 2. Synchronous send / receive.
    let Some(ep) = k.create_endpoint() else {
        return fail::<A>(passed, "object table exhausted");
    };
    let Some(worker) = k.create_tcb("worker", 5) else {
        return fail::<A>(passed, "object table exhausted");
    };
    let Ok(message) = Message::new(0x1234, &[11, 22, 33]) else {
        return fail::<A>(passed, "message construction");
    };
    check!(
        k.send(root, ep, 0x42, message, false) == Ok(false),
        "send blocks without receiver"
    );
    check!(
        k.tcb(root).map(|t| t.state) == Some(ThreadState::BlockedSend),
        "sender state is BlockedSend"
    );
    let received = k.recv(worker, ep, 0);
    check!(
        matches!(received, Ok(Some(ref message)) if message.words() == &[11, 22, 33][..]),
        "receiver gets the message"
    );
    check!(
        k.tcb(root).map(|t| t.state) == Some(ThreadState::Ready),
        "sender becomes Ready after delivery"
    );

    // 3. Call / Reply with a single caller slot.
    let Some(ep2) = k.create_endpoint() else {
        return fail::<A>(passed, "object table exhausted");
    };
    let call_msg = Message::empty(0xC0DE);
    check!(
        k.send(root, ep2, 0, call_msg, true) == Ok(false),
        "call blocks without receiver"
    );
    let call_received = k.recv(worker, ep2, 0);
    check!(matches!(call_received, Ok(Some(_))), "server receives call");
    check!(
        k.tcb(worker)
            .and_then(|t| t.caller)
            .map(|(client, _)| client)
            == Some(root),
        "server records caller"
    );
    check!(
        k.tcb(root).map(|t| t.state) == Some(ThreadState::BlockedReply),
        "client waits for reply"
    );
    check!(
        k.recv(worker, ep2, 0) == Err(KError::WouldBlock),
        "server must reply before receiving another call"
    );
    let reply = Message::new(0xBEEF, &[7]).unwrap_or_else(|_| Message::empty(0xBEEF));
    check!(k.reply(worker, reply).is_ok(), "reply succeeds");
    check!(
        k.tcb(root).map(|t| t.state) == Some(ThreadState::Ready),
        "client becomes Ready after reply"
    );
    check!(
        k.tcb(worker).and_then(|t| t.caller).is_none(),
        "caller slot is consumed by reply"
    );
    check!(
        k.reply(worker, reply) == Err(KError::NoCaller),
        "a second reply is rejected"
    );

    // 4. Notifications.
    let Some(ntfn) = k.create_notification() else {
        return fail::<A>(passed, "object table exhausted");
    };
    check!(
        k.wait_as(worker, ntfn) == Ok(None),
        "wait blocks on empty notification"
    );
    check!(
        k.tcb(worker).map(|t| t.state) == Some(ThreadState::BlockedNotification),
        "waiter state is BlockedNotification"
    );
    check!(k.signal(ntfn, 0x4).is_ok(), "signal wakes waiter");
    check!(
        k.tcb(worker).map(|t| t.state) == Some(ThreadState::Ready),
        "notification waiter becomes Ready"
    );
    check!(
        k.tcb(worker)
            .and_then(|t| t.resume)
            .map(|m| m.words() == &[4][..])
            .unwrap_or(false),
        "notification word is delivered"
    );
    check!(
        k.signal(ntfn, 0x8).is_ok(),
        "signal accumulates when nobody waits"
    );
    check!(
        k.poll(ntfn) == Ok(0x8),
        "poll consumes the notification word"
    );

    // 5. Capability transfer over IPC.
    let Some(xfer_ntfn) = k.create_notification() else {
        return fail::<A>(passed, "object table exhausted");
    };
    let Some(xfer_ep) = k.create_endpoint() else {
        return fail::<A>(passed, "object table exhausted");
    };
    check!(
        k.install_cap(
            root,
            3,
            Cap::new(xfer_ntfn, ObjType::Notification, Rights::ALL)
        )
        .is_ok(),
        "install grantable notification capability"
    );
    check!(
        k.recv(worker, xfer_ep, 6) == Ok(None),
        "receiver waits with a requested cap slot"
    );
    let transfer_msg = Message::new(0xCAFE, &[1])
        .unwrap_or_else(|_| Message::empty(0xCAFE))
        .with_cap(3);
    check!(
        k.send(root, xfer_ep, 0, transfer_msg, false) == Ok(true),
        "send delivers a message with a transferred capability"
    );
    check!(
        k.tcb(worker)
            .and_then(|table| table.resume)
            .is_some_and(|msg| msg.cap == 6),
        "receiver sees the installed capability slot"
    );
    check!(
        k.lookup_cap(worker, 6, Some(ObjType::Notification), Rights::READ)
            .is_ok(),
        "transferred capability is installed in the receiver"
    );
    check!(
        k.lookup_cap(worker, 6, Some(ObjType::Notification), Rights::GRANT)
            == Err(KError::NoRights),
        "transferred capability cannot be re-granted"
    );
    check!(
        k.install_cap(
            root,
            4,
            Cap::new(xfer_ntfn, ObjType::Notification, Rights::READ)
        )
        .is_ok(),
        "install a non-grantable capability"
    );
    let denied = Message::new(0xDEAD, &[0])
        .unwrap_or_else(|_| Message::empty(0xDEAD))
        .with_cap(4);
    check!(
        k.send(root, xfer_ep, 0, denied, false) == Err(KError::NoRights),
        "send refuses a capability without GRANT"
    );

    // 6. Tiny ramfs VFS.
    let mut file_buf = [0u8; 32];
    let fd = match k.open_path(root, b"/motd", O_RDONLY) {
        Ok(fd) => fd,
        Err(_) => return fail::<A>(passed, "open /motd"),
    };
    let read = match k.read_fd(root, fd, &mut file_buf) {
        Ok(n) => n,
        Err(_) => return fail::<A>(passed, "read /motd"),
    };
    check!(
        &file_buf[..read] == b"ViFastOS ramfs\n",
        "ramfs open/read returns /motd"
    );
    check!(
        k.open_path(root, b"/missing", O_RDONLY) == Err(KError::NotFound),
        "ramfs reports missing files"
    );
    check!(
        k.close_fd(root, fd).is_ok(),
        "ramfs closes a file descriptor"
    );
    let scratch = match k.open_path(root, b"/scratch", O_CREATE | O_RDWR) {
        Ok(fd) => fd,
        Err(_) => return fail::<A>(passed, "create /scratch"),
    };
    check!(
        k.write_fd(root, scratch, b"write-path\n") == Ok(11),
        "ramfs write/create path"
    );
    check!(
        k.stat_fd(root, scratch) == Ok(11),
        "ramfs stat reports file size"
    );
    check!(
        k.seek_fd(root, scratch, 0) == Ok(0),
        "ramfs seek resets offset"
    );
    let mut scratch_buf = [0u8; 16];
    let n = k.read_fd(root, scratch, &mut scratch_buf).unwrap_or(0);
    check!(
        &scratch_buf[..n] == b"write-path\n",
        "ramfs read after seek"
    );
    check!(k.close_fd(root, scratch).is_ok(), "ramfs closes scratch");

    // 7. VSpace/Frame generic mapping and W^X enforcement.
    let Some(vspace) = k.create_vspace(0x1000_0000, 0x4000_a000, 0x4000) else {
        return fail::<A>(passed, "object table exhausted");
    };
    let Some(untyped) = k.create_untyped(0x9000_0000, 0x2000) else {
        return fail::<A>(passed, "object table exhausted");
    };
    let frame = match k.retype(untyped, ObjType::Frame) {
        Ok(frame) => frame,
        Err(_) => return fail::<A>(passed, "retype frame"),
    };
    let page_table = match k.retype(untyped, ObjType::PageTable) {
        Ok(table) => table,
        Err(_) => return fail::<A>(passed, "retype page table"),
    };
    check!(
        k.frame(frame)
            .is_some_and(|frame| frame.pa == 0x9000_0000 && frame.size == 4096),
        "untyped retype carves an aligned frame"
    );
    check!(
        k.page_table(page_table)
            .is_some_and(|table| table.root == 0x9000_1000 && table.mapped.is_none()),
        "untyped retype carves a page table"
    );
    check!(
        k.retype(untyped, ObjType::Frame) == Err(KError::OutOfMemory),
        "untyped arena is exhausted after two pages"
    );
    let mapping = k.map_frame(vspace, frame, 0, Rights::READ | Rights::WRITE);
    check!(
        matches!(mapping, Ok(mapping) if mapping.va == 0x4000_a000 && mapping.pa == 0x9000_0000),
        "VSpace maps a frame at the next virtual page"
    );
    check!(
        k.map_frame(vspace, frame, 0, Rights::WRITE | Rights::EXEC) == Err(KError::NoRights),
        "VSpace rejects writable+executable mappings"
    );
    check!(
        k.unmap_frame(vspace, 0x4000_a000).is_ok(),
        "VSpace unmaps a tracked frame"
    );
    check!(
        k.map_frame(vspace, frame, 0x4000_a000, Rights::READ | Rights::EXEC)
            .is_ok(),
        "VSpace allows read+execute after unmap"
    );

    // 8. Priority scheduling with round-robin tie-breaking.
    // Reuse `k` so the self-test does not allocate another large Kernel on
    // the boot stack.  Drain any TCBs made Ready by the IPC tests first.
    while k.pick().is_some() {}
    let Some(low) = k.create_tcb("low", 3) else {
        return fail::<A>(passed, "object table exhausted");
    };
    let Some(high) = k.create_tcb("high", 9) else {
        return fail::<A>(passed, "object table exhausted");
    };
    let Some(mid) = k.create_tcb("mid", 5) else {
        return fail::<A>(passed, "object table exhausted");
    };
    k.enqueue(low);
    k.enqueue(high);
    k.enqueue(mid);
    check!(k.pick() == Some(high), "scheduler picks highest priority");
    if let Some(table) = k.tcb_mut(high) {
        table.priority = 0;
    }
    let _ = k.yield_current();
    check!(k.pick() == Some(mid), "scheduler picks next priority");
    if let Some(table) = k.tcb_mut(mid) {
        table.priority = 0;
    }
    let _ = k.yield_current();
    check!(
        k.pick() == Some(low),
        "scheduler eventually runs low priority"
    );

    // 9. Object generation rejects stale references.
    let stale = endpoint;
    check!(
        k.get(stale).is_some(),
        "live object reference remains valid"
    );
    check!(
        k.get(ObjRef {
            index: stale.index,
            gen: stale.gen.wrapping_add(1)
        })
        .is_none(),
        "generation mismatch is rejected"
    );

    // 10. Bounded capability table.
    check!(
        k.install_cap(
            root,
            MAX_CAPS as u8,
            Cap::new(endpoint, ObjType::Endpoint, Rights::ALL)
        ) == Err(KError::InvalidArg),
        "capability slot bounds are checked"
    );
    check!(
        k.check_invariants().is_ok(),
        "object/capability/scheduler/VSpace invariants hold"
    );

    passed
}

fn fail<A: Console>(_passed: bool, reason: &str) -> bool {
    crate::klog!(A, "[FAIL] {}", reason);
    false
}
