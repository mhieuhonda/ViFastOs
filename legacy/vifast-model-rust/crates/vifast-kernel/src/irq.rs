//! Chuyển tiếp ngắt phần cứng thành notification cho driver ở user space.
//!
//! Ngắt bị che (mask) sau khi giao cho driver, tới khi driver `IrqAck`.
//! Ngắt tới lúc đang che được ghi nhận pending (gộp lại), giao ngay khi ack.
//! Bản phần cứng: phần claim/complete PLIC nằm trong `vifast-core`.

use vifast_abi::*;

use crate::kernel::{done, Kernel, Ret};
use crate::object::*;
use crate::platform::Platform;

pub const MAX_IRQ: u32 = 256;

#[derive(Clone, Copy, Debug)]
pub struct IrqState {
    pub handler: ObjRef,
    pub ntfn: Option<ObjRef>,
    pub badge: u64,
    pub masked: bool,
    pub pending: bool,
}

impl<P: Platform> Kernel<P> {
    /// Phần cứng báo ngắt `irq`.
    pub fn deliver_irq(&mut self, irq: u32) {
        let (ntfn, badge) = match self.irqs.get_mut(&irq) {
            None => {
                self.stats.spurious_irq += 1;
                return;
            }
            Some(st) => {
                let alive = st.ntfn.map_or(false, |n| self.objs.try_get(n).is_some());
                if st.masked || !alive {
                    st.pending = true;
                    return;
                }
                st.masked = true;
                (st.ntfn, st.badge)
            }
        };
        if let Some(n) = ntfn {
            let _ = self.ntfn_signal(n, badge);
        }
    }

    pub fn create_irq_handler(&mut self, parent: SlotAddr, irq: u32, dest: SlotAddr) -> KResult<()> {
        if irq >= MAX_IRQ {
            return Err(KError::InvalidArg);
        }
        if self.irqs.contains_key(&irq) {
            return Err(KError::IrqInUse);
        }
        let h = self.objs.insert(Object::new(Kind::IrqHandler { irq }));
        self.irqs.insert(irq, IrqState { handler: h, ntfn: None, badge: 1, masked: false, pending: false });
        let mut cap = Cap::new(ObjType::IrqHandler, h, Rights::ALL);
        cap.irq = Some(irq);
        self.insert_cap(dest, cap, Some(parent))
    }

    pub fn bind_irq(&mut self, irq: u32, ntfn_cap: Cap) -> KResult<()> {
        let redeliver = {
            let st = self.irqs.get_mut(&irq).ok_or(KError::InvalidArg)?;
            st.ntfn = ntfn_cap.obj;
            st.badge = if ntfn_cap.badge != 0 { ntfn_cap.badge } else { 1 };
            if st.pending && !st.masked {
                st.pending = false;
                true
            } else {
                false
            }
        };
        if redeliver {
            self.deliver_irq(irq);
        }
        Ok(())
    }

    pub(crate) fn sys_irq_get(&mut self, t: ObjRef, control: u64, irq: u32, dest_index: u64, dest_cnode: u64) -> KResult<Ret> {
        let (ca, _) = self.lookup_cap(t, control, Some(ObjType::IrqControl), Rights::NONE)?;
        let dest = self.dest_slot(t, dest_cnode, dest_index)?;
        self.create_irq_handler(ca, irq, dest)?;
        done()
    }

    pub(crate) fn sys_irq_set_notification(&mut self, t: ObjRef, handler: u64, ntfn: u64) -> KResult<Ret> {
        let (_, hcap) = self.lookup_cap(t, handler, Some(ObjType::IrqHandler), Rights::NONE)?;
        let (_, ncap) = self.lookup_cap(t, ntfn, Some(ObjType::Notification), Rights::WRITE)?;
        self.bind_irq(hcap.irq.ok_or(KError::InvalidCap)?, ncap)?;
        done()
    }

    pub(crate) fn sys_irq_ack(&mut self, t: ObjRef, handler: u64) -> KResult<Ret> {
        let (_, hcap) = self.lookup_cap(t, handler, Some(ObjType::IrqHandler), Rights::NONE)?;
        let irq = hcap.irq.ok_or(KError::InvalidCap)?;
        let redeliver = {
            let st = self.irqs.get_mut(&irq).ok_or(KError::InvalidCap)?;
            st.masked = false;
            if st.pending {
                st.pending = false;
                true
            } else {
                false
            }
        };
        if redeliver {
            self.deliver_irq(irq);
        }
        done()
    }
}
