//! Capability space: phân giải cptr, tạo/sao chép/xoá/thu hồi cap, retype untyped.
//!
//! - Phân giải cptr kiểu seL4: mỗi cấp tiêu thụ `guard_bits` (phải khớp guard)
//!   rồi `radix` bit chỉ số, tổng cộng 64 bit.
//! - Cây dẫn xuất (CDT): delete nối con lên cha; revoke xoá toàn bộ con cháu.
//! - Revoke có preemption point: mỗi lần chỉ xoá `REVOKE_BATCH` cap rồi trả
//!   `Preempted`, thread chạy lại đúng syscall đó lượt sau.
//! - CNode hết cap trỏ tới thành zombie: dọn dần `REAP_BUDGET` slot mỗi tick, nên
//!   không đệ quy không giới hạn và xử lý được vòng CNode tự trỏ về chính nó.

use alloc::vec::Vec;

use vifast_abi::*;

use crate::kernel::{done, Kernel, Ret};
use crate::object::*;
use crate::platform::Platform;

/// Lấy `width` bit của `v` bắt đầu từ bit `shift` (tính từ bit thấp).
fn bits_at(v: u64, shift: u32, width: u32) -> u64 {
    if width == 0 {
        return 0;
    }
    let s = if shift >= 64 { 0 } else { v >> shift };
    if width >= 64 {
        s
    } else {
        s & ((1u64 << width) - 1)
    }
}

impl<P: Platform> Kernel<P> {
    // ------------------------------------------------------------------
    // truy cập slot
    // ------------------------------------------------------------------

    pub fn slot(&self, a: SlotAddr) -> KResult<&Slot> {
        match &self.objs.get(a.cnode)?.kind {
            Kind::CNode(cn) => cn.slots.get(a.index).ok_or(KError::InvalidArg),
            _ => Err(KError::WrongType),
        }
    }

    pub fn slot_mut(&mut self, a: SlotAddr) -> KResult<&mut Slot> {
        match &mut self.objs.get_mut(a.cnode)?.kind {
            Kind::CNode(cn) => cn.slots.get_mut(a.index).ok_or(KError::InvalidArg),
            _ => Err(KError::WrongType),
        }
    }

    pub fn cap_at(&self, a: SlotAddr) -> Option<Cap> {
        self.slot(a).ok().and_then(|s| s.cap)
    }

    pub(crate) fn set_cap_mapping(&mut self, a: SlotAddr, m: Option<Mapping>) {
        if let Ok(s) = self.slot_mut(a) {
            if let Some(c) = s.cap.as_mut() {
                c.mapped = m;
            }
        }
    }

    pub(crate) fn set_cap_io(&mut self, a: SlotAddr, m: Option<(ObjRef, u64)>) {
        if let Ok(s) = self.slot_mut(a) {
            if let Some(c) = s.cap.as_mut() {
                c.io_mapped = m;
            }
        }
    }

    /// CNode còn sống (không zombie).
    pub fn live_cnode(&self, r: ObjRef) -> KResult<&CNode> {
        match self.objs.try_get(r) {
            Some(o) if !o.hdr.dying => match &o.kind {
                Kind::CNode(cn) => Ok(cn),
                _ => Err(KError::InvalidCap),
            },
            _ => Err(KError::InvalidCap),
        }
    }

    // ------------------------------------------------------------------
    // phân giải
    // ------------------------------------------------------------------

    /// Phân giải cptr 64 bit qua CSpace nhiều cấp của thread `t`.
    pub fn resolve(&self, t: ObjRef, cptr: u64) -> KResult<(SlotAddr, Option<Cap>)> {
        let tcb = self.objs.tcb(t)?;
        let mut cn_ref = tcb.cspace.ok_or(KError::InvalidCap)?;
        let mut guard = tcb.cspace_guard;
        let mut gbits = tcb.cspace_guard_bits as u32;
        let mut bits: u32 = WORD_BITS;
        loop {
            let cn = self.live_cnode(cn_ref)?;
            let radix = cn.radix();
            if gbits + radix > bits {
                return Err(KError::InvalidCap);
            }
            if bits_at(cptr, bits - gbits, gbits) != guard {
                return Err(KError::InvalidCap);
            }
            bits -= gbits;
            let index = bits_at(cptr, bits - radix, radix) as usize;
            bits -= radix;
            let cap = cn.slots[index].cap;
            match cap {
                Some(c) if bits > 0 && c.obj_type == ObjType::CNode => {
                    cn_ref = c.obj.ok_or(KError::InvalidCap)?;
                    guard = c.guard;
                    gbits = c.guard_bits as u32;
                }
                _ => return Ok((SlotAddr { cnode: cn_ref, index }, cap)),
            }
        }
    }

    pub fn lookup_cap(&self, t: ObjRef, cptr: u64, ty: Option<ObjType>, rights: Rights) -> KResult<(SlotAddr, Cap)> {
        let (addr, cap) = self.resolve(t, cptr)?;
        let cap = cap.ok_or(KError::InvalidCap)?;
        if let Some(ty) = ty {
            if cap.obj_type != ty {
                return Err(KError::WrongType);
            }
        }
        if !cap.rights.contains(rights) {
            return Err(KError::NoRights);
        }
        if let Some(o) = cap.obj {
            if self.objs.try_get(o).is_none() {
                return Err(KError::ObjectDestroyed);
            }
        }
        Ok((addr, cap))
    }

    /// Slot đích còn trống. `dest_cnode = 0`: CNode gốc của người gọi.
    pub fn dest_slot(&self, t: ObjRef, dest_cnode: u64, index: u64) -> KResult<SlotAddr> {
        let r = if dest_cnode == 0 {
            self.objs.tcb(t)?.cspace.ok_or(KError::InvalidCap)?
        } else {
            let (_, cap) = self.lookup_cap(t, dest_cnode, Some(ObjType::CNode), Rights::WRITE)?;
            cap.obj.ok_or(KError::InvalidCap)?
        };
        let cn = self.live_cnode(r)?;
        let index = usize::try_from(index).map_err(|_| KError::InvalidArg)?;
        if index == 0 || index >= cn.slots.len() {
            return Err(KError::InvalidArg);
        }
        if cn.slots[index].cap.is_some() {
            return Err(KError::SlotOccupied);
        }
        Ok(SlotAddr { cnode: r, index })
    }

    // ------------------------------------------------------------------
    // thao tác lõi
    // ------------------------------------------------------------------

    pub fn insert_cap(&mut self, addr: SlotAddr, cap: Cap, parent: Option<SlotAddr>) -> KResult<()> {
        {
            let s = self.slot_mut(addr)?;
            if s.cap.is_some() {
                return Err(KError::SlotOccupied);
            }
            s.cap = Some(cap);
            s.parent = parent;
            s.children.clear();
        }
        if let Some(p) = parent {
            self.slot_mut(p)?.children.push(addr);
        }
        if let Some(o) = cap.obj {
            self.objs.get_mut(o)?.hdr.refs += 1;
        }
        Ok(())
    }

    /// Xoá một cap. Con của nó được nối lên cha (không mất quyền đã cấp).
    /// Object hết cap trỏ tới thì bị huỷ.
    pub fn delete_slot(&mut self, addr: SlotAddr) {
        let (cap, parent, children) = match self.slot_mut(addr) {
            Ok(s) => match s.cap.take() {
                Some(c) => (c, s.parent.take(), core::mem::take(&mut s.children)),
                None => return,
            },
            Err(_) => return,
        };
        if cap.mapped.is_some() {
            self.unmap_cap(&cap);
        }
        if cap.io_mapped.is_some() {
            self.io_unmap_cap(&cap);
        }
        if let Some(p) = parent {
            if let Ok(ps) = self.slot_mut(p) {
                if let Some(pos) = ps.children.iter().position(|c| *c == addr) {
                    ps.children.remove(pos);
                }
                ps.children.extend(children.iter().copied());
            }
        }
        for ch in &children {
            if let Ok(cs) = self.slot_mut(*ch) {
                cs.parent = parent;
            }
        }
        if let Some(o) = cap.obj {
            let destroy = match self.objs.get_mut(o) {
                Ok(obj) => {
                    obj.hdr.refs = obj.hdr.refs.saturating_sub(1);
                    obj.hdr.refs == 0 && !obj.hdr.dying
                }
                Err(_) => false,
            };
            if destroy {
                self.destroy_object(o);
            }
        }
    }

    /// Xoá cap con cháu từ lá lên, tối đa `budget` cap. `true` nếu đã xong.
    pub fn revoke(&mut self, addr: SlotAddr, mut budget: usize) -> bool {
        while budget > 0 {
            if self.objs.try_get(addr.cnode).is_none() {
                return true;
            }
            let mut cur = match self.slot(addr) {
                Ok(s) => match s.children.last() {
                    Some(c) => *c,
                    None => return true,
                },
                Err(_) => return true,
            };
            while let Some(next) = self.slot(cur).ok().and_then(|s| s.children.last().copied()) {
                cur = next;
            }
            self.delete_slot(cur);
            budget -= 1;
        }
        self.slot(addr).map_or(true, |s| s.children.is_empty())
    }

    pub fn destroy_object(&mut self, r: ObjRef) {
        let ty = match self.objs.get_mut(r) {
            Ok(o) => {
                o.hdr.dying = true;
                o.kind.obj_type()
            }
            Err(_) => return,
        };
        match ty {
            ObjType::Tcb => self.tcb_teardown(r),
            ObjType::Endpoint => loop {
                let next = match self.objs.endpoint_mut(r) {
                    Ok(ep) => ep.send_queue.pop_front().or_else(|| ep.recv_queue.pop_front()),
                    Err(_) => None,
                };
                match next {
                    Some(w) => self.abort_blocked(w, KError::ObjectDestroyed),
                    None => break,
                }
            },
            ObjType::Notification => {
                loop {
                    let w = self.objs.notification_mut(r).ok().and_then(|n| n.waiters.pop_front());
                    match w {
                        Some(w) => self.abort_blocked(w, KError::ObjectDestroyed),
                        None => break,
                    }
                }
                let bound = self.objs.notification(r).ok().and_then(|n| n.bound_tcb);
                if let Some(b) = bound {
                    if let Ok(t) = self.objs.tcb_mut(b) {
                        t.bound_ntfn = None;
                    }
                }
                for st in self.irqs.values_mut() {
                    if st.ntfn == Some(r) {
                        st.ntfn = None;
                    }
                }
            }
            ObjType::CNode => {
                // zombie: dọn dần ở các tick sau
                self.reap_queue.push_back(r);
                return;
            }
            ObjType::VSpace => self.vspace_teardown(r),
            ObjType::IoSpace => {
                let pages: Vec<IoMapping> = match self.objs.iospace_mut(r) {
                    Ok(io) => core::mem::take(&mut io.pages).into_values().collect(),
                    Err(_) => Vec::new(),
                };
                for m in pages {
                    self.set_cap_io(m.slot, None);
                }
            }
            ObjType::IrqHandler => {
                let irq = match self.objs.get(r) {
                    Ok(o) => match &o.kind {
                        Kind::IrqHandler { irq } => Some(*irq),
                        _ => None,
                    },
                    Err(_) => None,
                };
                if let Some(irq) = irq {
                    self.irqs.remove(&irq);
                }
            }
            ObjType::SchedContext => {
                let bound = self.objs.sched_context(r).ok().and_then(|s| s.bound_tcb);
                if let Some(b) = bound {
                    self.unbind_sc(b);
                    let ready = self.objs.tcb(b).map_or(false, |t| t.state == ThreadState::Ready);
                    if ready && !self.sched_queued(b) {
                        self.sched_enqueue(b, false);
                    }
                }
            }
            ObjType::Untyped => {
                let children: Vec<ObjRef> = self
                    .objs
                    .untyped(r)
                    .map(|u| u.children.iter().copied().collect())
                    .unwrap_or_default();
                for c in children {
                    if let Ok(o) = self.objs.get_mut(c) {
                        o.hdr.parent_untyped = None;
                    }
                }
            }
            _ => {}
        }
        self.finalize_object(r);
    }

    pub(crate) fn finalize_object(&mut self, r: ObjRef) {
        let parent = self.objs.try_get(r).and_then(|o| o.hdr.parent_untyped);
        if let Some(p) = parent {
            if let Ok(u) = self.objs.untyped_mut(p) {
                u.children.remove(&r);
                if u.children.is_empty() {
                    u.watermark = 0; // toàn bộ vùng nhớ được thu hồi
                }
            }
        }
        self.objs.remove(r);
    }

    /// Dọn CNode zombie, tối đa `budget` slot mỗi lần gọi.
    pub fn reap(&mut self, mut budget: usize) {
        while budget > 0 {
            let Some(&r) = self.reap_queue.front() else { return };
            let len = match self.objs.cnode(r) {
                Ok(cn) => cn.slots.len(),
                Err(_) => {
                    self.reap_queue.pop_front();
                    continue;
                }
            };
            loop {
                let i = self.objs.cnode(r).map_or(len, |cn| cn.reap_cursor);
                if budget == 0 || i >= len {
                    break;
                }
                let has_cap = self.objs.cnode(r).map_or(false, |cn| cn.slots[i].cap.is_some());
                if has_cap {
                    self.delete_slot(SlotAddr { cnode: r, index: i });
                    budget -= 1;
                }
                if let Ok(cn) = self.objs.cnode_mut(r) {
                    cn.reap_cursor += 1;
                }
            }
            let finished = self.objs.cnode(r).map_or(true, |cn| cn.reap_cursor >= len);
            if !finished {
                return;
            }
            self.reap_queue.pop_front();
            if self.objs.try_get(r).is_some() {
                self.finalize_object(r);
                self.stats.zombie_reaped += 1;
            }
        }
    }

    // ------------------------------------------------------------------
    // retype
    // ------------------------------------------------------------------

    /// Tạo `dests.len()` object loại `ty` từ untyped ở `ua`.
    /// Bộ nhớ mới luôn được xoá trắng (không rò dữ liệu của chủ cũ).
    pub fn retype_objects(&mut self, ua: SlotAddr, ucap: Cap, ty: ObjType, size_bits: u32, dests: &[SlotAddr]) -> KResult<()> {
        let size = object_size(ty, size_bits)?;
        let ut_ref = ucap.obj.ok_or(KError::InvalidCap)?;
        let ut_pa = self.objs.get(ut_ref)?.hdr.paddr;
        let (ut_size, wm) = {
            let u = self.objs.untyped(ut_ref)?;
            (u.size, u.watermark)
        };
        let n = dests.len() as u64;
        let align = if size.is_power_of_two() { size } else { 16 };
        let start = wm.div_ceil(align) * align;
        let end = start.checked_add(size.checked_mul(n).ok_or(KError::OutOfMemory)?).ok_or(KError::OutOfMemory)?;
        if end > ut_size {
            return Err(KError::OutOfMemory);
        }
        if ty == ObjType::VSpace {
            let available = self.asid_free.len() as u64 + MAX_ASID.saturating_sub(self.next_asid) as u64;
            if available < n {
                return Err(KError::OutOfMemory);
            }
        }
        self.objs.untyped_mut(ut_ref)?.watermark = end;
        let zeros = [0u8; 4096];
        for (i, d) in dests.iter().enumerate() {
            let paddr = ut_pa + start + i as u64 * size;
            let mut off = 0;
            while off < size {
                let chunk = core::cmp::min(4096, size - off) as usize;
                if !self.plat.phys_write(paddr + off, &zeros[..chunk]) {
                    return Err(KError::InvalidArg);
                }
                off += chunk as u64;
            }
            let mut kind = new_kind(ty, size_bits)?;
            if let Kind::VSpace(v) = &mut kind {
                v.asid = self.alloc_asid();
            }
            let obj = Object {
                hdr: Header { refs: 0, parent_untyped: Some(ut_ref), dying: false, paddr },
                kind,
            };
            let r = self.objs.insert(obj);
            self.objs.untyped_mut(ut_ref)?.children.insert(r);
            self.insert_cap(*d, Cap::new(ty, r, Rights::ALL), Some(ua))?;
        }
        Ok(())
    }

    pub(crate) fn alloc_asid(&mut self) -> u32 {
        if let Some(a) = self.asid_free.pop() {
            return a;
        }
        self.next_asid += 1;
        self.next_asid
    }

    // ------------------------------------------------------------------
    // syscall
    // ------------------------------------------------------------------

    pub(crate) fn sys_retype(&mut self, t: ObjRef, untyped: u64, ty: ObjType, size_bits: u32, dest_index: u64, count: u64, dest_cnode: u64) -> KResult<Ret> {
        let (ua, ucap) = self.lookup_cap(t, untyped, Some(ObjType::Untyped), Rights::WRITE)?;
        if !(1..=64).contains(&count) {
            return Err(KError::InvalidArg);
        }
        let mut dests = Vec::new();
        for i in 0..count {
            let idx = dest_index.checked_add(i).ok_or(KError::InvalidArg)?;
            dests.push(self.dest_slot(t, dest_cnode, idx)?);
        }
        self.retype_objects(ua, ucap, ty, size_bits, &dests)?;
        done()
    }

    pub(crate) fn sys_cap_copy(&mut self, t: ObjRef, src: u64, dest_index: u64, rights: Rights, dest_cnode: u64) -> KResult<Ret> {
        let (sa, cap) = self.lookup_cap(t, src, None, Rights::NONE)?;
        let dest = self.dest_slot(t, dest_cnode, dest_index)?;
        self.insert_cap(dest, cap.derive(rights, 0)?, Some(sa))?;
        done()
    }

    pub(crate) fn sys_cap_mint(&mut self, t: ObjRef, src: u64, dest_index: u64, rights: Rights, badge: u64, dest_cnode: u64) -> KResult<Ret> {
        let (sa, cap) = self.lookup_cap(t, src, None, Rights::NONE)?;
        let dest = self.dest_slot(t, dest_cnode, dest_index)?;
        self.insert_cap(dest, cap.derive(rights, badge)?, Some(sa))?;
        done()
    }

    pub(crate) fn sys_cnode_mint(&mut self, t: ObjRef, src: u64, dest_index: u64, guard: u64, guard_bits: u32, dest_cnode: u64) -> KResult<Ret> {
        let (sa, cap) = self.lookup_cap(t, src, Some(ObjType::CNode), Rights::NONE)?;
        if guard_bits > WORD_BITS || (guard_bits < 64 && guard >> guard_bits != 0) {
            return Err(KError::InvalidArg);
        }
        let dest = self.dest_slot(t, dest_cnode, dest_index)?;
        let mut new = cap.derive(Rights::ALL, 0)?;
        new.guard = guard;
        new.guard_bits = guard_bits as u8;
        self.insert_cap(dest, new, Some(sa))?;
        done()
    }

    pub(crate) fn sys_cap_delete(&mut self, t: ObjRef, slot: u64) -> KResult<Ret> {
        let (addr, _) = self.lookup_cap(t, slot, None, Rights::NONE)?;
        self.delete_slot(addr);
        done()
    }

    pub(crate) fn sys_cap_revoke(&mut self, t: ObjRef, slot: u64) -> KResult<Ret> {
        let (addr, _) = self.lookup_cap(t, slot, None, Rights::NONE)?;
        if self.revoke(addr, REVOKE_BATCH) {
            done()
        } else {
            Ok(Ret::Preempted)
        }
    }
}
