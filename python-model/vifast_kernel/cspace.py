"""Capability space: phân giải cptr nhiều cấp, tạo/sao chép/xoá/thu hồi cap, retype untyped.

- Phân giải cptr kiểu seL4: mỗi cấp tiêu thụ `guard_bits` (phải khớp guard) rồi `radix` bit chỉ số.
- Cây dẫn xuất (CDT): delete nối con lên cha; revoke xoá toàn bộ con cháu.
- Revoke có preemption point: mỗi lần chỉ xoá REVOKE_BATCH cap rồi trả PREEMPTED,
  thread chạy lại đúng syscall đó ở lượt sau -> thời gian mỗi lần vào kernel có giới hạn.
- CNode hết cap trỏ tới thành "zombie": được dọn dần REAP_BUDGET slot mỗi tick,
  tránh đệ quy không giới hạn và xử lý được vòng CNode chứa cap trỏ về chính nó.
"""
from __future__ import annotations

from .abi import CapCopy, CapDelete, CapMint, CapRevoke, CNodeMint, Retype, SysResult
from .hw import BusError
from .ktypes import (MAX_ASID, PREEMPTED, REVOKE_BATCH, WORD_BITS, WORD_MAX,
                     KError, ObjType, Rights, fail, object_size)
from .objects import (Cap, CNode, Endpoint, IoSpace, IrqHandler, Notification,
                      ObjRef, SchedContext, Slot, SlotAddr, Tcb, Untyped,
                      VSpace, new_object)


class CSpaceMixin:
    # ---------- phân giải ----------
    def slot(self, addr: SlotAddr) -> Slot:
        return self.objs.get(addr.cnode).slots[addr.index]

    def live_cnode(self, ref: ObjRef | None) -> CNode:
        cn = self.objs.try_get(ref)
        if not isinstance(cn, CNode) or cn.dying:
            fail(KError.INVALID_CAP)
        return cn

    def resolve(self, t: Tcb, cptr: int, depth: int = WORD_BITS) -> tuple[SlotAddr, Cap | None]:
        if not 0 <= cptr <= WORD_MAX:
            fail(KError.INVALID_CAP)
        cn_ref, guard, gbits, bits = t.cspace, t.cspace_guard, t.cspace_guard_bits, depth
        while True:
            cn = self.live_cnode(cn_ref)
            radix = cn.radix
            if gbits + radix > bits:
                fail(KError.INVALID_CAP)
            if (cptr >> (bits - gbits)) & ((1 << gbits) - 1) != guard:
                fail(KError.INVALID_CAP)
            bits -= gbits
            index = (cptr >> (bits - radix)) & ((1 << radix) - 1)
            bits -= radix
            cap = cn.slots[index].cap
            if bits == 0 or cap is None or cap.obj_type != ObjType.CNODE:
                return SlotAddr(cn_ref, index), cap
            cn_ref, guard, gbits = cap.obj, cap.guard, cap.guard_bits

    def lookup_cap(self, t: Tcb, cptr: int, obj_type: ObjType | None = None,
                   rights: Rights = Rights.NONE) -> tuple[SlotAddr, Cap]:
        addr, cap = self.resolve(t, cptr)
        if cap is None:
            fail(KError.INVALID_CAP)
        if obj_type is not None and cap.obj_type != obj_type:
            fail(KError.WRONG_TYPE)
        if (cap.rights & rights) != rights:
            fail(KError.NO_RIGHTS)
        if cap.obj is not None and self.objs.try_get(cap.obj) is None:
            fail(KError.OBJECT_DESTROYED)
        return addr, cap

    def dest_slot(self, t: Tcb, dest_cnode: int, index: int) -> SlotAddr:
        if dest_cnode == 0:
            ref = t.cspace
        else:
            _, cap = self.lookup_cap(t, dest_cnode, ObjType.CNODE, Rights.WRITE)
            ref = cap.obj
        cn = self.live_cnode(ref)
        if not 0 < index < len(cn.slots):
            fail(KError.INVALID_ARG)
        if cn.slots[index].cap is not None:
            fail(KError.SLOT_OCCUPIED)
        return SlotAddr(ref, index)

    # ---------- thao tác lõi ----------
    def insert_cap(self, addr: SlotAddr, cap: Cap, parent: SlotAddr | None) -> None:
        s = self.slot(addr)
        if s.cap is not None:
            fail(KError.SLOT_OCCUPIED)
        s.cap, s.parent, s.children = cap, parent, []
        if parent is not None:
            self.slot(parent).children.append(addr)
        if cap.obj is not None:
            self.objs.get(cap.obj).refs += 1

    def delete_slot(self, addr: SlotAddr) -> None:
        s = self.slot(addr)
        cap = s.cap
        if cap is None:
            return
        if cap.mapped is not None:
            self.unmap_cap(cap)
        if cap.io_mapped is not None:
            self.io_unmap_cap(cap)
        if s.parent is not None:
            p = self.slot(s.parent)
            p.children.remove(addr)
            p.children.extend(s.children)
        for ch in s.children:
            self.slot(ch).parent = s.parent
        s.cap, s.parent, s.children = None, None, []
        if cap.obj is not None:
            obj = self.objs.try_get(cap.obj)
            if obj is not None:
                obj.refs -= 1
                if obj.refs <= 0 and not obj.dying:
                    self.destroy_object(cap.obj)

    def revoke(self, addr: SlotAddr, budget: int) -> bool:
        """Xoá cap con cháu từ lá lên, tối đa `budget` cap. True nếu đã xong."""
        while budget > 0:
            if self.objs.try_get(addr.cnode) is None:
                return True
            root = self.slot(addr)
            if not root.children:
                return True
            cur = root.children[-1]
            while self.slot(cur).children:
                cur = self.slot(cur).children[-1]
            self.delete_slot(cur)
            budget -= 1
        return not self.slot(addr).children

    def destroy_object(self, ref: ObjRef) -> None:
        obj = self.objs.get(ref)
        obj.dying = True
        if isinstance(obj, Tcb):
            self.tcb_teardown(ref, obj)
        elif isinstance(obj, Endpoint):
            for q in (obj.send_queue, obj.recv_queue):
                while q:
                    self.abort_blocked(q.popleft(), KError.OBJECT_DESTROYED)
        elif isinstance(obj, Notification):
            while obj.waiters:
                self.abort_blocked(obj.waiters.popleft(), KError.OBJECT_DESTROYED)
            b = self.objs.try_get(obj.bound_tcb)
            if b is not None:
                b.bound_ntfn = None
            for st in self.irqs.values():
                if st.ntfn == ref:
                    st.ntfn = None
        elif isinstance(obj, CNode):
            self.reap_queue.append(ref)      # zombie: dọn dần ở các tick sau
            return
        elif isinstance(obj, VSpace):
            self.vspace_teardown(obj)
        elif isinstance(obj, IoSpace):
            for m in obj.pages.values():
                c = self._cap_at(m.slot)
                if c is not None:
                    c.io_mapped = None
            obj.pages.clear()
        elif isinstance(obj, IrqHandler):
            self.irqs.pop(obj.irq, None)
        elif isinstance(obj, SchedContext):
            t = self.objs.try_get(obj.bound_tcb)
            if isinstance(t, Tcb):
                self.unbind_sc(t)
                if t.state == t.state.READY and not self.sched_queued(obj.bound_tcb):
                    self.sched_enqueue(obj.bound_tcb, t)
        elif isinstance(obj, Untyped):
            for ch in obj.children:
                o = self.objs.try_get(ch)
                if o is not None:
                    o.parent_untyped = None
        self.finalize_object(ref, obj)

    def finalize_object(self, ref: ObjRef, obj) -> None:
        parent = self.objs.try_get(obj.parent_untyped)
        if isinstance(parent, Untyped):
            parent.children.discard(ref)
            if not parent.children:
                parent.watermark = 0          # toàn bộ vùng nhớ được thu hồi
        self.objs.remove(ref)

    def reap(self, budget: int) -> None:
        """Dọn CNode zombie, tối đa `budget` slot mỗi tick."""
        while budget > 0 and self.reap_queue:
            ref = self.reap_queue[0]
            cn = self.objs.try_get(ref)
            if cn is None:
                self.reap_queue.popleft()
                continue
            while budget > 0 and cn.reap_cursor < len(cn.slots):
                i = cn.reap_cursor
                if cn.slots[i].cap is not None:
                    self.delete_slot(SlotAddr(ref, i))
                    budget -= 1
                cn.reap_cursor += 1
            if cn.reap_cursor >= len(cn.slots):
                self.reap_queue.popleft()
                self.finalize_object(ref, cn)
                self.stats["zombie_reaped"] += 1

    def _cap_at(self, addr: SlotAddr) -> Cap | None:
        cn = self.objs.try_get(addr.cnode)
        return cn.slots[addr.index].cap if isinstance(cn, CNode) else None

    # ---------- retype ----------
    def retype_objects(self, ua: SlotAddr, ucap: Cap, obj_type: ObjType,
                       size_bits: int, dests: list[SlotAddr]) -> None:
        size = object_size(obj_type, size_bits)
        ut: Untyped = self.objs.get(ucap.obj)
        align = size if size & (size - 1) == 0 else 16
        start = -(-ut.watermark // align) * align
        end = start + size * len(dests)
        if end > ut.size:
            fail(KError.OUT_OF_MEMORY)
        if obj_type == ObjType.VSPACE and len(self.asid_free) + (MAX_ASID - self.next_asid) < len(dests):
            fail(KError.OUT_OF_MEMORY)
        ut.watermark = end
        for i, d in enumerate(dests):
            obj = new_object(obj_type, size_bits)
            obj.parent_untyped = ucap.obj
            obj.paddr = ut.paddr + start + i * size
            try:
                self.phys.write(obj.paddr, bytes(size))    # kernel luôn xoá trắng bộ nhớ mới
            except BusError:
                fail(KError.INVALID_ARG)
            if isinstance(obj, VSpace):
                obj.asid = self.alloc_asid()
            ref = self.objs.insert(obj)
            ut.children.add(ref)
            self.insert_cap(d, Cap(obj_type, ref, Rights.ALL), parent=ua)

    def alloc_asid(self) -> int:
        if self.asid_free:
            return self.asid_free.pop()
        self.next_asid += 1
        return self.next_asid

    # ---------- syscall ----------
    def sys_retype(self, ref, t, op: Retype):
        ua, ucap = self.lookup_cap(t, op.untyped, ObjType.UNTYPED, Rights.WRITE)
        if not 1 <= op.count <= 64:
            fail(KError.INVALID_ARG)
        dests = [self.dest_slot(t, op.dest_cnode, op.dest_index + i) for i in range(op.count)]
        self.retype_objects(ua, ucap, op.obj_type, op.size_bits, dests)
        return SysResult()

    def sys_cap_copy(self, ref, t, op: CapCopy):
        sa, cap = self.lookup_cap(t, op.src)
        dest = self.dest_slot(t, op.dest_cnode, op.dest_index)
        self.insert_cap(dest, cap.derive(op.rights), parent=sa)
        return SysResult()

    def sys_cap_mint(self, ref, t, op: CapMint):
        sa, cap = self.lookup_cap(t, op.src)
        dest = self.dest_slot(t, op.dest_cnode, op.dest_index)
        self.insert_cap(dest, cap.derive(op.rights, op.badge), parent=sa)
        return SysResult()

    def sys_cnode_mint(self, ref, t, op: CNodeMint):
        sa, cap = self.lookup_cap(t, op.src, ObjType.CNODE)
        if not 0 <= op.guard_bits <= WORD_BITS or op.guard >> op.guard_bits:
            fail(KError.INVALID_ARG)
        dest = self.dest_slot(t, op.dest_cnode, op.dest_index)
        new = cap.derive(Rights.ALL)
        new.guard, new.guard_bits = op.guard, op.guard_bits
        self.insert_cap(dest, new, parent=sa)
        return SysResult()

    def sys_cap_delete(self, ref, t, op: CapDelete):
        addr, _ = self.lookup_cap(t, op.slot)
        self.delete_slot(addr)
        return SysResult()

    def sys_cap_revoke(self, ref, t, op: CapRevoke):
        addr, _ = self.lookup_cap(t, op.slot)
        if self.revoke(addr, REVOKE_BATCH):
            return SysResult()
        self.stats["revoke_preempt"] += 1
        return PREEMPTED

