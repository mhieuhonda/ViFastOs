"""Bộ nhớ ảo Sv39 (RISC-V), TLB có ASID, TLB shootdown giữa các hart, IOMMU cho DMA.

Page table là cấu trúc thật nằm trong PhysMem: 3 cấp, 512 PTE 8 byte mỗi bảng,
PTE = PPN << 10 | U X W R V. Phần cứng (MMU giả lập) tự đi bảng khi TLB miss.
Kernel không tự cấp page table: user retype PageTable rồi PageTableMap (như seL4).
Rust: phần đọc/ghi PTE và sfence.vma nằm trong crate lõi unsafe, logic còn lại port 1-1.
"""
from __future__ import annotations

from .abi import (Fetch, FrameMap, FrameUnmap, IoMap, IoUnmap, Load, PageTableMap,
                  Store, SysResult)
from .hw import BusError
from .ktypes import (PAGE_SHIFT, PAGE_SIZE, KError, ObjType, Rights, VmFault, fail)
from .objects import Cap, Frame, IoMapping, IoSpace, ObjRef, PageTable, SlotAddr, Tcb, VSpace

PTE_V, PTE_R, PTE_W, PTE_X, PTE_U = 1, 2, 4, 8, 16
USER_VA_LIMIT = 1 << 38
MAX_ACCESS = PAGE_SIZE * 16


def _idx(va: int, level: int) -> int:
    return (va >> (PAGE_SHIFT + 9 * level)) & 511


def _pte_pa(pte: int) -> int:
    return (pte >> 10) << PAGE_SHIFT


def _make_pte(pa: int, flags: int) -> int:
    return ((pa >> PAGE_SHIFT) << 10) | flags


def _is_leaf(pte: int) -> bool:
    return bool(pte & (PTE_R | PTE_W | PTE_X))


def _leaf_flags(rights: Rights) -> int:
    flags = PTE_V | PTE_U
    if rights & Rights.READ:
        flags |= PTE_R
    if rights & Rights.WRITE:
        flags |= PTE_W | PTE_R            # RISC-V: W mà không R là tổ hợp dành riêng
    if rights & Rights.EXEC:
        flags |= PTE_X
    if not flags & (PTE_R | PTE_X):
        fail(KError.INVALID_ARG)
    if flags & PTE_W and flags & PTE_X:
        fail(KError.ILLEGAL_OPERATION)    # chính sách W^X
    return flags


_NEED = {Rights.READ: PTE_R, Rights.WRITE: PTE_W, Rights.EXEC: PTE_X}


class VmMixin:
    # ---------- đi bảng ----------
    def _entry_pa(self, root: int, va: int, level: int) -> int | None:
        """Địa chỉ vật lý của PTE ở `level` cho `va`; None nếu thiếu bảng trung gian."""
        table = root
        for lv in range(2, level, -1):
            pte = self.phys.read_u64(table + _idx(va, lv) * 8)
            if not pte & PTE_V or _is_leaf(pte):
                return None
            table = _pte_pa(pte)
        return table + _idx(va, level) * 8

    def sv39_walk(self, root: int, va: int) -> int | None:
        """Phần cứng đi bảng khi TLB miss. Trả PTE lá (chỉ hỗ trợ trang 4 KB)."""
        ea = self._entry_pa(root, va, 0)
        if ea is None:
            return None
        pte = self.phys.read_u64(ea)
        return pte if pte & PTE_V and _is_leaf(pte) else None

    def translate(self, t: Tcb, va: int, need: Rights) -> int:
        vs = self.objs.try_get(t.vspace)
        if not isinstance(vs, VSpace) or vs.dying or not 0 <= va < USER_VA_LIMIT:
            raise VmFault(va)
        tlb = self.cur_hart.tlb
        vpn = va >> PAGE_SHIFT
        pte = tlb.get(vs.asid, vpn)
        if pte is None:
            pte = self.sv39_walk(vs.paddr, va)
            if pte is None:
                raise VmFault(va)
            tlb.put(vs.asid, vpn, pte)
        if not pte & PTE_U or not pte & _NEED[need]:
            raise VmFault(va)
        return _pte_pa(pte) | (va & (PAGE_SIZE - 1))

    def _chunks(self, t: Tcb, va: int, length: int, need: Rights):
        addr, end = va, va + length
        while addr < end:
            n = min(PAGE_SIZE - (addr & (PAGE_SIZE - 1)), end - addr)
            yield self.translate(t, addr, need), addr - va, n
            addr += n

    # ---------- TLB shootdown ----------
    def shootdown(self, asid: int, vpn: int | None = None) -> None:
        self.stats["shootdown"] += 1
        if not self.tlb_shootdown_enabled:
            return                         # chỉ để minh hoạ lỗi khi quên shootdown
        for h in self.harts:
            h.tlb.flush(asid, vpn)
            if h is not self.cur_hart:
                self.stats["ipi"] += 1

    # ---------- map / unmap ----------
    def map_page_table(self, paddr_slot: SlotAddr, pcap: Cap, vs_ref: ObjRef, vaddr: int) -> int:
        if pcap.mapped is not None:
            fail(KError.ALREADY_MAPPED)
        if not 0 <= vaddr < USER_VA_LIMIT:
            fail(KError.INVALID_ARG)
        vs: VSpace = self.objs.get(vs_ref)
        pt: PageTable = self.objs.get(pcap.obj)
        for level in (2, 1):
            ea = self._entry_pa(vs.paddr, vaddr, level)
            pte = self.phys.read_u64(ea)
            if not pte & PTE_V:
                self.phys.write_u64(ea, _make_pte(pt.paddr, PTE_V))
                base = vaddr & ~((1 << (PAGE_SHIFT + 9 * level)) - 1)
                pcap.mapped = (vs_ref, base, level)
                vs.tables[(base, level)] = paddr_slot
                return level
        fail(KError.ALREADY_MAPPED)

    def map_frame(self, faddr: SlotAddr, fcap: Cap, vs_ref: ObjRef, vaddr: int, rights: Rights) -> None:
        if vaddr % PAGE_SIZE:
            fail(KError.ALIGNMENT)
        if not 0 <= vaddr < USER_VA_LIMIT:
            fail(KError.INVALID_ARG)
        if fcap.mapped is not None:
            fail(KError.ALREADY_MAPPED)
        vs: VSpace = self.objs.get(vs_ref)
        frame: Frame = self.objs.get(fcap.obj)
        flags = _leaf_flags(rights & fcap.rights)
        ea = self._entry_pa(vs.paddr, vaddr, 0)
        if ea is None:
            fail(KError.MISSING_PAGE_TABLE)
        if self.phys.read_u64(ea) & PTE_V:
            fail(KError.ALREADY_MAPPED)
        self.phys.write_u64(ea, _make_pte(frame.paddr, flags))
        fcap.mapped = (vs_ref, vaddr)
        vs.frames[vaddr >> PAGE_SHIFT] = faddr

    def unmap_cap(self, cap: Cap) -> None:
        info, cap.mapped = cap.mapped, None
        vs = self.objs.try_get(info[0])
        if not isinstance(vs, VSpace) or vs.dying:
            return
        obj = self.objs.try_get(cap.obj)
        if cap.obj_type == ObjType.FRAME:
            vaddr = info[1]
            ea = self._entry_pa(vs.paddr, vaddr, 0)
            if ea is not None and obj is not None and _pte_pa(self.phys.read_u64(ea)) == obj.paddr:
                self.phys.write_u64(ea, 0)
            vs.frames.pop(vaddr >> PAGE_SHIFT, None)
            self.shootdown(vs.asid, vaddr >> PAGE_SHIFT)
        else:
            base, level = info[1], info[2]
            ea = self._entry_pa(vs.paddr, base, level)
            if ea is not None and obj is not None and _pte_pa(self.phys.read_u64(ea)) == obj.paddr:
                self.phys.write_u64(ea, 0)
            vs.tables.pop((base, level), None)
            self.shootdown(vs.asid)

    def vspace_teardown(self, vs: VSpace) -> None:
        for addr in list(vs.frames.values()) + list(vs.tables.values()):
            c = self._cap_at(addr)
            if c is not None:
                c.mapped = None
        vs.frames.clear()
        vs.tables.clear()
        self.shootdown(vs.asid)
        self.asid_free.append(vs.asid)

    # ---------- IOMMU ----------
    def io_unmap_cap(self, cap: Cap) -> None:
        info, cap.io_mapped = cap.io_mapped, None
        io = self.objs.try_get(info[0])
        if isinstance(io, IoSpace):
            io.pages.pop(info[1] >> PAGE_SHIFT, None)

    def dma_write(self, dev, addr: int, data: bytes) -> bool:
        """Thiết bị ghi DMA. Có IOMMU: dịch qua IoSpace của thiết bị; không có: ghi thẳng RAM."""
        if not self.iommu_enabled:
            try:
                self.phys.write(addr, data)
                return True
            except BusError:
                return False
        io = self.objs.try_get(dev.iospace)
        writes, pos = [], 0
        while pos < len(data):
            iova = addr + pos
            n = min(PAGE_SIZE - (iova & (PAGE_SIZE - 1)), len(data) - pos)
            m = io.pages.get(iova >> PAGE_SHIFT) if isinstance(io, IoSpace) else None
            if m is None or not m.rights & Rights.WRITE:
                self.stats["iommu_fault"] += 1
                self.klog("iommu", f"chặn DMA của '{dev.name}' tới iova={iova:#x}")
                return False
            writes.append((m.paddr | (iova & (PAGE_SIZE - 1)), data[pos:pos + n]))
            pos += n
        for pa, chunk in writes:
            self.phys.write(pa, chunk)
        return True

    # ---------- lệnh máy ----------
    def sys_load(self, ref, t, op: Load):
        if not 0 <= op.length <= MAX_ACCESS:
            fail(KError.INVALID_ARG)
        buf = bytearray()
        for pa, _, n in list(self._chunks(t, op.vaddr, op.length, Rights.READ)):
            buf += self.phys.read(pa, n)
        return SysResult(data=bytes(buf))

    def sys_store(self, ref, t, op: Store):
        if len(op.data) > MAX_ACCESS:
            fail(KError.INVALID_ARG)
        for pa, off, n in list(self._chunks(t, op.vaddr, len(op.data), Rights.WRITE)):
            self.phys.write(pa, op.data[off:off + n])
        return SysResult()

    def sys_fetch(self, ref, t, op: Fetch):
        pa = self.translate(t, op.vaddr, Rights.EXEC)
        return SysResult(data=self.phys.read(pa, 4))

    # ---------- syscall ----------
    def sys_page_table_map(self, ref, t, op: PageTableMap):
        paddr, pcap = self.lookup_cap(t, op.page_table, ObjType.PAGE_TABLE)
        _, vcap = self.lookup_cap(t, op.vspace, ObjType.VSPACE, Rights.WRITE)
        return SysResult(value=self.map_page_table(paddr, pcap, vcap.obj, op.vaddr))

    def sys_frame_map(self, ref, t, op: FrameMap):
        faddr, fcap = self.lookup_cap(t, op.frame, ObjType.FRAME)
        _, vcap = self.lookup_cap(t, op.vspace, ObjType.VSPACE, Rights.WRITE)
        self.map_frame(faddr, fcap, vcap.obj, op.vaddr, op.rights)
        return SysResult()

    def sys_frame_unmap(self, ref, t, op: FrameUnmap):
        _, fcap = self.lookup_cap(t, op.frame, ObjType.FRAME)
        if fcap.mapped is None:
            fail(KError.NOT_MAPPED)
        self.unmap_cap(fcap)
        return SysResult()

    def sys_io_map(self, ref, t, op: IoMap):
        faddr, fcap = self.lookup_cap(t, op.frame, ObjType.FRAME)
        _, icap = self.lookup_cap(t, op.iospace, ObjType.IO_SPACE, Rights.WRITE)
        frame: Frame = self.objs.get(fcap.obj)
        if frame.device or op.iova % PAGE_SIZE or op.iova < 0:
            fail(KError.INVALID_ARG)
        if fcap.io_mapped is not None:
            fail(KError.ALREADY_MAPPED)
        io: IoSpace = self.objs.get(icap.obj)
        vpn = op.iova >> PAGE_SHIFT
        if vpn in io.pages:
            fail(KError.ALREADY_MAPPED)
        io.pages[vpn] = IoMapping(frame.paddr, op.rights & fcap.rights, faddr)
        fcap.io_mapped = (icap.obj, op.iova)
        return SysResult()

    def sys_io_unmap(self, ref, t, op: IoUnmap):
        _, fcap = self.lookup_cap(t, op.frame, ObjType.FRAME)
        if fcap.io_mapped is None:
            fail(KError.NOT_MAPPED)
        self.io_unmap_cap(fcap)
        return SysResult()
