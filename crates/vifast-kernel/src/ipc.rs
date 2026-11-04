//! Synchronous IPC and notifications for the portable kernel.

use vifast_abi::*;

use crate::object::*;
use crate::Kernel;

impl Kernel {
    pub(crate) fn make_ready(&mut self, tcb: ObjRef) {
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
    ) -> KResult<()> {
        let Some(raw_slot) = msg.cap_slot() else {
            return Ok(());
        };
        let src_slot = u8::try_from(raw_slot).map_err(|_| KError::InvalidCap)?;
        let source = self.lookup_cap(sender, src_slot, None, Rights::NONE)?;
        if !source.rights.contains(Rights::GRANT) {
            return Err(KError::NoRights);
        }
        if hint == 0 || (hint as usize) >= MAX_CAPS {
            return Err(KError::InvalidArg);
        }
        let table = self.tcb(receiver).ok_or(KError::InvalidCap)?;
        if table.cspace[hint as usize].is_some() {
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
        let can_grant = if msg.cap_slot().is_some() {
            let raw_slot = msg.cap_slot().ok_or(KError::InvalidCap)?;
            let slot = u8::try_from(raw_slot).map_err(|_| KError::InvalidCap)?;
            self.lookup_cap(sender, slot, None, Rights::GRANT)?;
            true
        } else {
            false
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
                if let Err(error) = self.grant_message_cap(sender, receiver, hint, &mut msg) {
                    /* Put the receiver back so a failed transfer does not
                     * consume its receive. */
                    if let Some(ep) = self.endpoint_mut(endpoint) {
                        let _ = ep.recv.push(receiver);
                    }
                    return Err(error);
                }
            }
            if let Some(table) = self.tcb_mut(receiver) {
                table.resume = Some(msg);
            } else {
                return Err(KError::ObjectDestroyed);
            }
            self.make_ready(receiver);
            if is_call {
                if let Some(table) = self.tcb_mut(receiver) {
                    table.caller = Some((sender, epoch));
                }
                if let Some(table) = self.tcb_mut(sender) {
                    table.state = ThreadState::BlockedReply;
                    table.pending = None;
                }
            } else if self
                .tcb(sender)
                .is_some_and(|t| t.state != ThreadState::Running)
            {
                self.make_ready(sender);
            }
            self.stats.ipc_sent += 1;
            Ok(true)
        } else {
            let pending = PendingSend {
                badge,
                msg,
                is_call,
                can_grant,
            };
            {
                let table = self.tcb_mut(sender).ok_or(KError::InvalidCap)?;
                table.pending = Some(pending);
                table.state = ThreadState::BlockedSend;
            }
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
        if self
            .tcb(receiver)
            .ok_or(KError::InvalidCap)?
            .caller
            .is_some()
        {
            return Err(KError::WouldBlock);
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
            if pending.can_grant {
                if let Err(error) =
                    self.grant_message_cap(sender, receiver, recv_slot, &mut pending.msg)
                {
                    /* Keep the sender queued so a transient cspace failure
                     * does not strand it in BlockedSend forever. */
                    if let Some(table) = self.tcb_mut(sender) {
                        table.pending = Some(pending);
                    }
                    if let Some(ep) = self.endpoint_mut(endpoint) {
                        let _ = ep.send.push(sender);
                    }
                    return Err(error);
                }
            }
            if pending.is_call {
                let epoch = self.tcb(sender).ok_or(KError::ObjectDestroyed)?.reply_epoch;
                let table = self.tcb_mut(receiver).ok_or(KError::InvalidCap)?;
                table.caller = Some((sender, epoch));
                let sender_table = self.tcb_mut(sender).ok_or(KError::ObjectDestroyed)?;
                sender_table.state = ThreadState::BlockedReply;
            } else {
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
        if let Some(table) = self.tcb_mut(client) {
            table.resume = Some(msg);
            table.state = ThreadState::Ready;
        }
        self.make_ready(client);
        Ok(())
    }

    pub fn signal(&mut self, notification: ObjRef, bits: u64) -> KResult<()> {
        let bits = if bits == 0 { 1 } else { bits };
        let waiter = {
            let ntfn = self
                .notification_mut(notification)
                .ok_or(KError::InvalidCap)?;
            let waiter = ntfn.waiter.take();
            let value = ntfn.word | bits;
            ntfn.word = if waiter.is_some() { 0 } else { value };
            waiter.map(|w| (w, value))
        };
        if let Some((waiter, value)) = waiter {
            if let Ok(msg) = Message::new(LABEL_NOTIFICATION, &[value]) {
                if let Some(table) = self.tcb_mut(waiter) {
                    table.resume = Some(msg);
                }
            }
            self.make_ready(waiter);
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
        {
            let table = self.tcb_mut(tcb).ok_or(KError::InvalidCap)?;
            table.state = ThreadState::BlockedNotification;
        }
        let ntfn = self
            .notification_mut(notification)
            .ok_or(KError::InvalidCap)?;
        ntfn.waiter = Some(tcb);
        Ok(None)
    }

    pub fn poll(&mut self, notification: ObjRef) -> KResult<u64> {
        let ntfn = self
            .notification_mut(notification)
            .ok_or(KError::InvalidCap)?;
        let word = ntfn.word;
        ntfn.word = 0;
        Ok(word)
    }

    pub fn bind_notification(&mut self, tcb: ObjRef, notification: ObjRef) -> KResult<()> {
        if self.tcb(tcb).and_then(|t| t.bound_ntfn).is_some() {
            return Err(KError::AlreadyMapped);
        }
        if self
            .notification(notification)
            .and_then(|n| n.waiter)
            .is_some()
        {
            return Err(KError::AlreadyMapped);
        }
        if let Some(table) = self.tcb_mut(tcb) {
            table.bound_ntfn = Some(notification);
        }
        Ok(())
    }
}
