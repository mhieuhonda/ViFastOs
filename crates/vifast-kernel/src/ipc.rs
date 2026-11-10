//! Synchronous IPC and notifications for the portable kernel.

use vifast_abi::*;

use crate::object::*;
use crate::Kernel;

impl Kernel {
    pub(crate) fn make_ready(&mut self, tcb: ObjRef) {
        let Some(state) = self.tcb(tcb).map(|table| table.state) else {
            return;
        };
        // Vòng 99 (soi chay K4): chỉ đánh thức task đang thật sự chờ. Trước đây
        // set Ready mù quáng nên có thể:
        //  - đặt Ready cho task đang Running trên hart khác (vừa Running vừa
        //    queued → có thể chạy hai lần, cùng bệnh K1),
        //  - tự gỡ treo một task Suspended khi có send tới,
        //  - cho task BlockedBudget chạy lố budget.
        // `Inactive` vẫn được phép (enqueue sẽ chuyển Ready) để không đổi hành
        // vi khởi động.
        // Vòng 70: BlockedBudget (hết ngân sách sched_context) cũng wakeable
        // như mọi Blocked* khác — cùng họ lỗi vòng 65 (wake_error bỏ sót).
        // Nếu không, send tới task hết budget không đánh thức, message kẹt.
        let wakeable = matches!(
            state,
            ThreadState::Inactive
                // `reply()` set Ready trước khi gọi make_ready; vẫn phải
                // enqueue (enqueue bỏ qua nếu đã queued).
                | ThreadState::Ready
                | ThreadState::BlockedSend
                | ThreadState::BlockedRecv
                | ThreadState::BlockedReply
                | ThreadState::BlockedNotification
                | ThreadState::BlockedSleep
                | ThreadState::BlockedFault
                | ThreadState::BlockedBudget
        );
        if !wakeable {
            return;
        }
        // (#2) Keep the sleep/budget active-counters in sync when IPC wakes
        // a sleeper/budgeter through this path instead of tick/suspend.
        // `tick()` self-heals a drifted counter, but an exact count keeps
        // the idle fast-path (`tick` early-return) correct immediately.
        if matches!(state, ThreadState::BlockedSleep) {
            self.leave_sleep(tcb);
        } else if matches!(state, ThreadState::BlockedBudget) {
            self.leave_budget(tcb);
        }
        if let Some(table) = self.tcb_mut(tcb) {
            table.state = ThreadState::Ready;
        }
        self.enqueue(tcb);
    }

    /// Install a transferred capability into the receiver's cspace.  The
    /// sender must hold GRANT on the source slot; the receiver gets a copy
    /// without GRANT, so a grant right cannot be laundered through IPC.
    fn grant_message_cap(
        &mut self,
        sender: ObjRef,
        receiver: ObjRef,
        hint: u8,
        msg: &mut Message,
        pending: &PendingSend,
    ) -> KResult<()> {
        let Some(raw_slot) = msg.cap_slot() else {
            return Ok(());
        };
        let src_slot = u8::try_from(raw_slot).map_err(|_| KError::InvalidCap)?;
        let source = self.lookup_cap(sender, src_slot, None, Rights::NONE)?;
        if !source.rights.contains(Rights::GRANT) {
            return Err(KError::NoRights);
        }
        // Vòng 101 (K49): verify snapshot GRANT lúc queue — slot của sender
        // đang BlockedSend mà bị CapCopy/revoke đổi object/id thì từ chối
        // thay vì chuyển nhầm vé mới cho receiver.
        if !pending.grant_key_matches(src_slot, source.id, source.obj) {
            return Err(KError::NoRights);
        }
        if hint == 0 || (hint as usize) >= self.cspace_capacity(receiver) {
            return Err(KError::InvalidArg);
        }
        if self
            .cspace(receiver)
            .and_then(|slots| slots.get(hint as usize))
            .is_some_and(Option::is_some)
        {
            return Err(KError::SlotOccupied);
        }
        let rights = source.rights.without(Rights::GRANT);
        self.copy_cap(sender, src_slot, receiver, hint, rights, 0)?;
        msg.cap = hint as u64;
        Ok(())
    }

    /// Send a message.  Returns `true` if delivered immediately, `false` if
    /// the sender was queued.  A `Call` leaves the sender in `BlockedReply`
    /// when delivered.
    pub fn send(
        &mut self,
        sender: ObjRef,
        endpoint: ObjRef,
        badge: u64,
        msg: Message,
        is_call: bool,
    ) -> KResult<bool> {
        // Vòng 101 (K49): snapshot vé GRANT (slot, id, object, rights) ngay
        // lúc queue để recv verify lúc dequeue — chống TOCTOU đổi cspace
        // giữa chừng.
        let (can_grant, grant_key) = if msg.cap_slot().is_some() {
            let raw_slot = msg.cap_slot().ok_or(KError::InvalidCap)?;
            let slot = u8::try_from(raw_slot).map_err(|_| KError::InvalidCap)?;
            let source = self.lookup_cap(sender, slot, None, Rights::GRANT)?;
            (
                true,
                PendingSend::grant_key_for(slot, source.id, source.obj),
            )
        } else {
            (false, 0)
        };
        if is_call {
            // A server that already holds a caller must reply before it can
            // accept another Call.  Otherwise the single caller slot would be
            // overwritten and the first client would wait forever.
            let front = self.endpoint(endpoint).and_then(|ep| ep.recv.front());
            if let Some(receiver) = front {
                if self.tcb(receiver).is_some_and(|t| t.caller.is_some()) {
                    return Err(KError::WouldBlock);
                }
            }
        }
        let receiver = {
            let ep = self.endpoint_mut(endpoint).ok_or(KError::InvalidCap)?;
            ep.recv.pop()
        };
        if let Some(receiver) = receiver {
            let epoch = self.tcb(sender).ok_or(KError::InvalidCap)?.reply_epoch;
            let hint = self.tcb(receiver).map(|table| table.recv_slot).unwrap_or(0);
            let mut msg = msg;
            if can_grant {
                // Giao ngay (không qua hàng): snapshot vừa lấy ở trên chính
                // là vé hiện tại — gói thành PendingSend tạm để tái dùng
                // cùng đường verify.
                let probe = PendingSend {
                    badge,
                    msg,
                    is_call,
                    can_grant,
                    grant_key,
                };
                if let Err(error) = self.grant_message_cap(sender, receiver, hint, &mut msg, &probe)
                {
                    /* Vòng 101 (K48): grant fail phải trả receiver về ĐẦU hàng
                     * (push_front) để giữ FIFO — receiver chưa nhận gì nên
                     * không được tụt xuống cuối. */
                    if let Some(ep) = self.endpoint_mut(endpoint) {
                        let _ = ep.recv.push_front(receiver);
                    }
                    return Err(error);
                }
            }
            self.copy_ipc_buffers(sender, receiver);
            msg.badge = badge;
            if let Some(table) = self.tcb_mut(receiver) {
                table.resume = Some(msg);
                table.resume_error = None;
            } else {
                return Err(KError::ObjectDestroyed);
            }
            self.make_ready(receiver);
            if is_call {
                // 290.4 (r290): cấp epoch MỚI cho mỗi lần call. Trước đây
                // `reply_epoch` không bao giờ tăng nên guard K49 luôn là
                // `0 == 0` (rỗng): record `caller` cũ vẫn "khớp" và `reply`
                // giao nhầm cho client đã gọi lần khác.
                let _ = epoch;
                let fresh_epoch = match self.tcb_mut(sender) {
                    Some(table) => {
                        table.reply_epoch = table.reply_epoch.wrapping_add(1);
                        table.reply_epoch
                    }
                    None => 0,
                };
                if let Some(table) = self.tcb_mut(receiver) {
                    table.caller = Some((sender, fresh_epoch));
                }
                if let Some(table) = self.tcb_mut(sender) {
                    table.state = ThreadState::BlockedReply;
                    table.pending = None;
                }
                // Mốc 5 (vòng 11): call giao ngay cũng park sender — sender
                // có thể đang Ready+queued từ lần wake trước (vd. reply 1
                // vừa make_ready client, client call tiếp). Không gỡ thì
                // BlockedReply+queued, scheduler có thể pick nhầm task đang
                // chờ (cùng họ bug K32/K35 như park path).
                self.remove_from_runq(sender);
            } else if self
                .tcb(sender)
                .is_some_and(|t| t.state != ThreadState::Running)
            {
                self.make_ready(sender);
            }
            self.stats.ipc_sent += 1;
            Ok(true)
        } else {
            let mut msg = msg;
            // The badge is kernel-owned metadata and must be fixed before a
            // sender can park in the queue.
            msg.badge = badge;
            let pending = PendingSend {
                badge,
                msg,
                is_call,
                can_grant,
                grant_key,
            };
            {
                let table = self.tcb_mut(sender).ok_or(KError::InvalidCap)?;
                table.pending = Some(pending);
                table.state = ThreadState::BlockedSend;
            }
            // Mốc 5 (vòng 11): park phải rời runqueue trước — sender có thể
            // đang Ready+queued (vd. make_ready từ lần giao ngay trước rồi
            // send tiếp mà không có receiver). Không gỡ thì thành viên kép
            // Blocked+queued (cùng họ K32/K35).
            self.remove_from_runq(sender);
            let pushed = {
                let ep = self.endpoint_mut(endpoint).ok_or(KError::InvalidCap)?;
                ep.send.push(sender)
            };
            if !pushed {
                if let Some(table) = self.tcb_mut(sender) {
                    table.pending = None;
                    table.state = ThreadState::Ready;
                }
                return Err(KError::OutOfMemory);
            }
            self.stats.ipc_sent += 1;
            Ok(false)
        }
    }

    /// Receive a message.  Returns `None` if the caller was queued.
    pub fn recv(
        &mut self,
        receiver: ObjRef,
        endpoint: ObjRef,
        recv_slot: u8,
    ) -> KResult<Option<Message>> {
        let bound = self.tcb(receiver).ok_or(KError::InvalidCap)?;
        if bound.caller.is_some() {
            return Err(KError::WouldBlock);
        }
        // K42 (r260, red-first): `bound_ntfn` trước đây chỉ được GHI và KIỂM —
        // `recv` không bao giờ hỏi tới, nên Notification bind vào TCB là một
        // trường chết. Nay `recv` poll nó trước khi chặn: có bit chờ thì trả
        // ngay event (kiểu bound notification của seL4), nhờ vậy task vừa
        // chờ IPC vừa chờ ngắt không bị kẹt.
        let wake_word = bound
            .bound_ntfn
            .and_then(|notification| {
                self.notification_mut(notification).map(|ntfn| {
                    let word = ntfn.word;
                    ntfn.word = 0;
                    word
                })
            })
            .filter(|word| *word != 0);
        if let Some(word) = wake_word {
            let message = Message::new(LABEL_NOTIFICATION, &[word])?;
            return Ok(Some(message));
        }
        let sender = {
            let ep = self.endpoint_mut(endpoint).ok_or(KError::InvalidCap)?;
            ep.send.pop()
        };
        if let Some(sender) = sender {
            let mut pending = self
                .tcb_mut(sender)
                .ok_or(KError::ObjectDestroyed)?
                .pending
                .take()
                .ok_or(KError::InvalidArg)?;
            pending.msg.badge = pending.badge;
            if pending.can_grant {
                // Copy snapshot ra trước vì grant_message_cap mượn &mut self
                // trong khi pending.msg cũng mượn &mut.
                let snapshot = pending;
                if let Err(error) =
                    self.grant_message_cap(sender, receiver, recv_slot, &mut pending.msg, &snapshot)
                {
                    /* Vòng 101 (K48): grant fail phải trả sender về ĐẦU hàng
                     * (push_front) chứ không phải đuôi (push). Trước đây FIFO
                     * bị đảo: đầu hàng lỗi SlotOccupied thì tụt xuống cuối,
                     * người sau vượt lên; lặp lại thì đói. */
                    if let Some(table) = self.tcb_mut(sender) {
                        table.pending = Some(pending);
                    }
                    if let Some(ep) = self.endpoint_mut(endpoint) {
                        let _ = ep.send.push_front(sender);
                    }
                    return Err(error);
                }
            }
            self.copy_ipc_buffers(sender, receiver);
            if pending.is_call {
                // 290.4: cấp epoch MỚI cho mỗi lần call (xem chú thích ở nhánh
                // giao ngay phía trên).
                let epoch = match self.tcb_mut(sender) {
                    Some(table) => {
                        table.reply_epoch = table.reply_epoch.wrapping_add(1);
                        table.reply_epoch
                    }
                    None => return Err(KError::ObjectDestroyed),
                };
                let table = self.tcb_mut(receiver).ok_or(KError::InvalidCap)?;
                table.caller = Some((sender, epoch));
                let sender_table = self.tcb_mut(sender).ok_or(KError::ObjectDestroyed)?;
                sender_table.state = ThreadState::BlockedReply;
            } else if self
                .tcb(sender)
                .is_some_and(|table| table.state != ThreadState::BlockedFault)
            {
                self.make_ready(sender);
            }
            self.stats.ipc_received += 1;
            Ok(Some(pending.msg))
        } else {
            {
                let table = self.tcb_mut(receiver).ok_or(KError::InvalidCap)?;
                table.state = ThreadState::BlockedRecv;
                table.recv_slot = recv_slot;
            }
            // Mốc 5 (vòng 11): như send — receiver có thể đang Ready+queued
            // (make_ready từ lần giao ngay trước) rồi recv tiếp mà hàng
            // trống. Park mà không gỡ runq thì BlockedRecv+queued.
            self.remove_from_runq(receiver);
            let pushed = {
                let ep = self.endpoint_mut(endpoint).ok_or(KError::InvalidCap)?;
                ep.recv.push(receiver)
            };
            if !pushed {
                if let Some(table) = self.tcb_mut(receiver) {
                    table.state = ThreadState::Ready;
                }
                return Err(KError::OutOfMemory);
            }
            Ok(None)
        }
    }

    /// Reply to the current caller (if any), then receive the next message.
    /// With no caller this is exactly `recv`; an invalid caller is reported
    /// before the server is queued.
    pub fn reply_recv(
        &mut self,
        server: ObjRef,
        endpoint: ObjRef,
        recv_slot: u8,
        msg: Message,
    ) -> KResult<Option<Message>> {
        if self.endpoint(endpoint).is_none() {
            return Err(KError::InvalidCap);
        }
        if self.tcb(server).ok_or(KError::InvalidCap)?.caller.is_some() {
            self.reply(server, msg)?;
        }
        self.recv(server, endpoint, recv_slot)
    }

    pub fn reply(&mut self, server: ObjRef, msg: Message) -> KResult<()> {
        let (client, epoch) = self
            .tcb_mut(server)
            .ok_or(KError::InvalidCap)?
            .caller
            .take()
            .ok_or(KError::NoCaller)?;
        let valid = self.tcb(client).is_some_and(|table| {
            table.state == ThreadState::BlockedReply && table.reply_epoch == epoch
        });
        if !valid {
            return Err(KError::NoCaller);
        }
        // Mốc 5 (vòng 11): Reply mang cap — server GRANT vé cho client qua
        // recv_slot của client (đường grant_message_cap chung với send, có
        // verify TOCTOU K49). Trước đây msg.cap trên reply bị bỏ qua thầm lặng.
        let mut msg = msg;
        if msg.cap_slot().is_some() {
            // Vòng 225 (P0 reply-orphan, hoàn thiện): MỌI lỗi sau take()
            // mà client vẫn còn chờ (BlockedReply + epoch khớp) đều phải
            // trả caller về chỗ cũ — kể cả lỗi lookup_cap/convert ở dưới.
            // Chỉ stale caller (client đã hết chờ) mới được drop.
            let restore = |kernel: &mut Self| {
                let still_waiting = kernel.tcb(client).is_some_and(|table| {
                    table.state == ThreadState::BlockedReply && table.reply_epoch == epoch
                });
                if still_waiting {
                    if let Some(table) = kernel.tcb_mut(server) {
                        if table.caller.is_none() {
                            table.caller = Some((client, epoch));
                        }
                    }
                }
            };
            let raw_slot = match msg.cap_slot().ok_or(KError::InvalidCap) {
                Ok(v) => v,
                Err(e) => {
                    restore(&mut *self);
                    return Err(e);
                }
            };
            let slot = match u8::try_from(raw_slot).map_err(|_| KError::InvalidCap) {
                Ok(v) => v,
                Err(e) => {
                    restore(&mut *self);
                    return Err(e);
                }
            };
            let source = match self.lookup_cap(server, slot, None, Rights::GRANT) {
                Ok(v) => v,
                Err(e) => {
                    restore(&mut *self);
                    return Err(e);
                }
            };
            let hint = self.tcb(client).map(|table| table.recv_slot).unwrap_or(0);
            let probe = PendingSend {
                badge: 0,
                msg,
                is_call: false,
                can_grant: true,
                grant_key: PendingSend::grant_key_for(slot, source.id, source.obj),
            };
            if let Err(error) = self.grant_message_cap(server, client, hint, &mut msg, &probe) {
                // Vòng 225 (P0 reply-orphan): grant lỗi phải trả caller về chỗ
                // cũ — trước đây caller.take() ở trên đã xóa mà lỗi grant trả
                // về ngay khiến client kẹt BlockedReply vĩnh viễn không ai
                // reply được nữa. Khôi phục để server retry sau khi client
                // dọn recv_slot.
                restore(&mut *self);
                return Err(error);
            }
        }
        self.copy_ipc_buffers(server, client);
        if let Some(table) = self.tcb_mut(client) {
            table.resume = Some(msg);
            table.resume_error = None;
            table.state = ThreadState::Ready;
        }
        self.make_ready(client);
        Ok(())
    }

    /// Parent chờ con exit (mốc 1): target đã Exited thì xong ngay; nếu
    /// không, waiter park BlockedReply và mượn `target.caller` làm chỗ chờ
    /// (exit_current_on wake caller này — cùng đường dọn caller ma K36).
    /// Chỉ một waiter tại một thời điểm (target đã có caller thì WouldBlock).
    /// Waiter đang BlockedReply (chờ IPC nơi khác) thì WouldBlock.
    pub fn wait_tcb(&mut self, waiter: ObjRef, target: ObjRef) -> KResult<bool> {
        if waiter == target {
            return Err(KError::IllegalOperation);
        }
        let target_table = self.tcb(target).ok_or(KError::WrongType)?;
        if target_table.state == ThreadState::Exited {
            return Ok(true);
        }
        if target_table.state == ThreadState::Inactive {
            return Err(KError::IllegalOperation);
        }
        if target_table.caller.is_some() {
            return Err(KError::WouldBlock);
        }
        let waiter_state = self.tcb(waiter).ok_or(KError::InvalidCap)?.state;
        if waiter_state == ThreadState::BlockedReply {
            return Err(KError::WouldBlock);
        }
        let epoch = self.tcb(target).ok_or(KError::ObjectDestroyed)?.reply_epoch;
        if let Some(table) = self.tcb_mut(target) {
            table.caller = Some((waiter, epoch));
        }
        if let Some(table) = self.tcb_mut(waiter) {
            table.state = ThreadState::BlockedReply;
            table.pending = None;
        }
        self.remove_from_runq(waiter);
        Ok(false)
    }

    pub fn signal(&mut self, notification: ObjRef, bits: u64) -> KResult<()> {
        let bits = if bits == 0 { 1 } else { bits };
        // Vòng 88 (audit #4 P0): đánh thức TẤT CẢ waiter còn sống trong hàng
        // đợi, chỉ những task đang thật sự BlockedNotification. Bit không bị
        // mất khi không có waiter nào.
        let pending = {
            let ntfn = self.notification(notification).ok_or(KError::InvalidCap)?;
            ntfn.waiters
        };
        let value = {
            let ntfn = self
                .notification_mut(notification)
                .ok_or(KError::InvalidCap)?;
            let value = ntfn.word | bits;
            ntfn.waiters = [None; NOTIFICATION_WAITERS];
            ntfn.word = 0;
            value
        };
        let mut delivered = false;
        for waiter in pending.into_iter().flatten() {
            let blocked = self
                .tcb(waiter)
                .map(|table| table.state == ThreadState::BlockedNotification)
                .unwrap_or(false);
            if !blocked {
                continue;
            }
            if let Ok(msg) = Message::new(LABEL_NOTIFICATION, &[value]) {
                if let Some(table) = self.tcb_mut(waiter) {
                    table.resume = Some(msg);
                    table.resume_error = None;
                }
            }
            self.make_ready(waiter);
            delivered = true;
        }
        if !delivered {
            if let Some(ntfn) = self.notification_mut(notification) {
                ntfn.word = value;
            }
        }
        self.stats.notifications += 1;
        Ok(())
    }

    pub fn wait_as(&mut self, tcb: ObjRef, notification: ObjRef) -> KResult<Option<u64>> {
        let word = {
            let ntfn = self
                .notification_mut(notification)
                .ok_or(KError::InvalidCap)?;
            if ntfn.word != 0 {
                let word = ntfn.word;
                ntfn.word = 0;
                Some(word)
            } else {
                None
            }
        };
        if let Some(word) = word {
            return Ok(Some(word));
        }
        // Vòng 88 (audit #4 P0): giành một slot TRƯỚC khi đổi state. Đăng ký
        // lại (cùng tcb) chỉ refresh. Hàng đợi đầy -> WouldBlock và state giữ
        // nguyên (không kẹt BlockedNotification như bản một-slot trước đây).
        {
            let ntfn = self
                .notification_mut(notification)
                .ok_or(KError::InvalidCap)?;
            let mut slot = None;
            let mut occupied = 0;
            for (index, entry) in ntfn.waiters.iter().enumerate() {
                if *entry == Some(tcb) {
                    slot = Some(index);
                    break;
                }
                if entry.is_some() {
                    occupied += 1;
                } else if slot.is_none() {
                    slot = Some(index);
                }
            }
            match slot {
                Some(index) => {
                    ntfn.waiters[index] = Some(tcb);
                }
                None => {
                    let _ = occupied;
                    return Err(KError::WouldBlock);
                }
            }
        }
        {
            let table = self.tcb_mut(tcb).ok_or(KError::InvalidCap)?;
            table.state = ThreadState::BlockedNotification;
        }
        Ok(None)
    }

    pub fn poll(&mut self, notification: ObjRef) -> KResult<u64> {
        let ntfn = self
            .notification_mut(notification)
            .ok_or(KError::InvalidCap)?;
        let word = ntfn.word;
        ntfn.word = 0;
        // K46 (r260, red-first): tiêu thụ word cũng có nghĩa "đã nhận ngắt",
        // nên phải hạ `pending` của mọi IRQ đang trỏ tới notification này.
        // Trước đây `poll` chỉ xoá word: driver đọc sự kiện rồi re-arm thiết
        // bị mà không `IrqAck`, cạnh kế tiếp bị coalesce (`irq_raise` thấy
        // `pending == true` nên không signal) ⇒ MẤT NGẮT.
        if word != 0 {
            for index in 0..crate::MAX_OBJECTS {
                let reference = ObjRef {
                    index: index as u8,
                    gen: self.objs[index].gen,
                };
                if let Some(irq) = self.irq_mut(reference) {
                    if irq.notification == Some(notification) {
                        irq.pending = false;
                    }
                }
            }
        }
        Ok(word)
    }

    pub fn bind_notification(&mut self, tcb: ObjRef, notification: ObjRef) -> KResult<()> {
        // Vòng 101 (K44): liveness check ngay đầu — trước đây tcb/notification
        // chết vẫn rơi qua Ok vì dạng `if let Some → set; Ok(())`.
        self.tcb(tcb).ok_or(KError::WrongType)?;
        self.notification(notification).ok_or(KError::WrongType)?;
        if self.tcb(tcb).and_then(|t| t.bound_ntfn).is_some() {
            return Err(KError::AlreadyMapped);
        }
        // Vòng 101 (K43): một Notification chỉ bind cho đúng một TCB —
        // Notification không có back-pointer nên phải quét mọi TCB khác.
        for i in 0..crate::MAX_OBJECTS {
            let r = ObjRef {
                index: i as u8,
                gen: self.objs[i].gen,
            };
            if r != tcb
                && self
                    .tcb(r)
                    .is_some_and(|table| table.bound_ntfn == Some(notification))
            {
                return Err(KError::AlreadyMapped);
            }
        }
        // Vòng 101 (K51): bỏ nhánh cấm bind khi notification đang có waiter —
        // cấm đó ngăn rebind hợp lệ từ task cũ sang mới; signal đã xử lý hỗn
        // hợp word/waiter và destroy xóa cả hai, nên check waiter là thừa.
        if let Some(table) = self.tcb_mut(tcb) {
            table.bound_ntfn = Some(notification);
        }
        Ok(())
    }
}
