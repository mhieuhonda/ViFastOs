//! Bộ nhớ ảo Sv39 (RISC-V), shootdown TLB giữa các hart, IOMMU cho DMA.
//!
//! Page table là cấu trúc thật nằm trong bộ nhớ vật lý: 3 cấp, 512 PTE 8 byte mỗi
//! bảng, `PTE = PPN << 10 | U X W R V`. Phần cứng (MMU) tự đi bảng khi TLB miss.
//! Kernel không tự cấp page table: user retype PageTable rồi PageTableMap (như seL4).
//!
//! Các hàm tự do ở đầu module (`walk`, `entry_pa`...) là thuần tuý, dùng chung cho
//! kernel (khi map/unmap) và cho MMU giả lập trong `vifast-sim`.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use vifast_abi::*;

use crate::kernel::{done, Kernel, Ret};
use crate::object::*;
use crate::platform::Platform;

pub const PTE_V: u64 = 1;
pub const PTE_R: u64 = 2;
pub const PTE_W: u64 = 4;
pub const PTE_X: u64 = 8;
pub const PTE_U: u64 = 16;
/// Giới hạn địa chỉ ảo user (nửa thấp của Sv39).
pub const USER_VA_LIMIT: u64 = 1 << 38;
/// Kích thước tối đa một lần Load/Store trong simulator.
pub const MAX_ACCESS: u64 = PAGE_SIZE * 16;

pub fn vpn_index(va: u64, level: u32) -> u64 {
    (va >> (PAGE_SHIFT + 9 * level)) & 511
}

pub fn pte_pa(pte: u64) -> u64 {
    (pte >> 10) << PAGE_SHIFT
}

pub fn make_pte(pa: u64, flags: u64) -> u64 {
    ((pa >> PAGE_SHIFT) << 10) | flags
}

pub fn is_leaf(pte: u64) -> bool {
    pte & (PTE_R | PTE_W | PTE_X) != 0
}

/// Địa chỉ vật lý của PTE ở `level` cho `va`. `None` nếu thiếu bảng trung gian.
pub fn entry_pa<F: Fn(u64) -> u64>(read: &F, root: u64, va: u64, level: u32) -> Option<u64> {
    let mut table = root;
    let mut lv = 2;
    while lv > level {
        let pte = read(table + vpn_index(va, lv) * 8);
        if pte & PTE_V == 0 || is_leaf(pte) {
            return None;
        }
        table = pte_pa(pte);
        lv -= 1;
    }
    Some(table + vpn_index(va, level) * 8)
}

/// Đi bảng như phần cứng khi TLB miss. Trả PTE lá (chỉ hỗ trợ trang 4 KB).
pub fn walk<F: Fn(u64) -> u64>(read: F, root: u64, va: u64) -> Option<u64> {
    let ea = entry_pa(&read, root, va, 0)?;
    let pte = read(ea);
    if pte & PTE_V != 0 && is_leaf(pte) {
        Some(pte)
    } else {
        None
    }
}

/// Cờ PTE lá từ quyền. Áp chính sách W^X.
pub fn leaf_flags(rights: Rights) -> KResult<u64> {
    let mut flags = PTE_V | PTE_U;
    if rights.contains(Rights::READ) {
        flags |= PTE_R;
    }
    if rights.contains(Rights::WRITE) {
        flags |= PTE_W | PTE_R; // RISC-V: W mà không R là tổ hợp dành riêng
    }
    if rights.contains(Rights::EXEC) {
        flags |= PTE_X;
    }
    if flags & (PTE_R | PTE_X) == 0 {
        return Err(KError::InvalidArg);
    }
    if flags & PTE_W != 0 && flags & PTE_X != 0 {
        return Err(KError::IllegalOperation);
    }
    Ok(flags)
}

impl<P: Platform> Kernel<P> {
    /// (page table gốc, asid) của một VSpace còn sống.
    pub fn vspace_info(&self, vs: ObjRef) -> Option<(u64, u32)> {
        let o = self.objs.try_get(vs)?;
        if o.hdr.dying {
            return None;
        }
        match &o.kind {
            Kind::VSpace(v) => Some((o.hdr.paddr, v.asid)),
            _ => None,
        }
    }

    /// Giá trị "satp" của thread: MMU dùng để dịch địa chỉ.
    pub fn address_space(&self, t: ObjRef) -> Option<(u64, u32)> {
        let vs = self.objs.tcb(t).ok()?.vspace?;
        self.vspace_info(vs)
    }

    fn entry_of(&self, root: u64, va: u64, level: u32) -> Option<u64> {
        entry_pa(&|pa: u64| self.read_u64(pa), root, va, level)
    }

    // ------------------------------------------------------------------
    // TLB shootdown
    // ------------------------------------------------------------------

    pub(crate) fn shootdown(&mut self, asid: u32, vpn: Option<u64>) {
        self.stats.shootdown += 1;
        if !self.tlb_shootdown_enabled {
            return;
        }
        for h in 0..self.harts.len() {
            self.plat.tlb_flush(h, Some(asid), vpn);
            if Some(h) != self.cur_hart {
                self.stats.ipi += 1;
            }
        }
    }

    // ------------------------------------------------------------------
    // map / unmap
    // ------------------------------------------------------------------

    /// Gắn page table vào cấp trống đầu tiên trên đường tới `vaddr`. Trả về cấp đã gắn.
    pub fn map_page_table(&mut self, pt_addr: SlotAddr, pcap: Cap, vs: ObjRef, vaddr: u64) -> KResult<u8> {
        if pcap.mapped.is_some() {
            return Err(KError::AlreadyMapped);
        }
        if vaddr >= USER_VA_LIMIT {
            return Err(KError::InvalidArg);
        }
        let (root, _) = self.vspace_info(vs).ok_or(KError::InvalidCap)?;
        let pt = pcap.obj.ok_or(KError::InvalidCap)?;
        let pt_pa = self.objs.get(pt)?.hdr.paddr;
        for level in [2u32, 1] {
            let ea = self.entry_of(root, vaddr, level).ok_or(KError::InvalidArg)?;
            if self.read_u64(ea) & PTE_V == 0 {
                self.write_u64(ea, make_pte(pt_pa, PTE_V));
                let shift = PAGE_SHIFT + 9 * level;
                let base = vaddr & !((1u64 << shift) - 1);
                self.set_cap_mapping(pt_addr, Some(Mapping::Table { vspace: vs, base, level: level as u8 }));
                self.objs.vspace_mut(vs)?.tables.insert((base, level as u8), pt_addr);
                return Ok(level as u8);
            }
        }
        Err(KError::AlreadyMapped)
    }

    pub fn map_frame(&mut self, faddr: SlotAddr, fcap: Cap, vs: ObjRef, vaddr: u64, rights: Rights) -> KResult<()> {
        if vaddr % PAGE_SIZE != 0 {
            return Err(KError::Alignment);
        }
        if vaddr >= USER_VA_LIMIT {
            return Err(KError::InvalidArg);
        }
        if fcap.mapped.is_some() {
            return Err(KError::AlreadyMapped);
        }
        let (root, _) = self.vspace_info(vs).ok_or(KError::InvalidCap)?;
        let fr = fcap.obj.ok_or(KError::InvalidCap)?;
        self.objs.frame(fr)?;
        let fpa = self.objs.get(fr)?.hdr.paddr;
        let flags = leaf_flags(rights & fcap.rights)?;
        let ea = self.entry_of(root, vaddr, 0).ok_or(KError::MissingPageTable)?;
        if self.read_u64(ea) & PTE_V != 0 {
            return Err(KError::AlreadyMapped);
        }
        self.write_u64(ea, make_pte(fpa, flags));
        self.set_cap_mapping(faddr, Some(Mapping::Frame { vspace: vs, vaddr }));
        self.objs.vspace_mut(vs)?.frames.insert(vaddr >> PAGE_SHIFT, faddr);
        Ok(())
    }

    /// Gỡ ánh xạ của một cap (đã lấy ra khỏi slot hoặc sắp được cập nhật).
    pub(crate) fn unmap_cap(&mut self, cap: &Cap) {
        let Some(m) = cap.mapped else { return };
        let obj_pa = cap.obj.and_then(|o| self.objs.try_get(o)).map(|o| o.hdr.paddr);
        match m {
            Mapping::Frame { vspace, vaddr } => {
                let Some((root, asid)) = self.vspace_info(vspace) else { return };
                if let (Some(ea), Some(opa)) = (self.entry_of(root, vaddr, 0), obj_pa) {
                    if pte_pa(self.read_u64(ea)) == opa {
                        self.write_u64(ea, 0);
                    }
                }
                if let Ok(v) = self.objs.vspace_mut(vspace) {
                    v.frames.remove(&(vaddr >> PAGE_SHIFT));
                }
                self.shootdown(asid, Some(vaddr >> PAGE_SHIFT));
            }
            Mapping::Table { vspace, base, level } => {
                let Some((root, asid)) = self.vspace_info(vspace) else { return };
                if let (Some(ea), Some(opa)) = (self.entry_of(root, base, level as u32), obj_pa) {
                    if pte_pa(self.read_u64(ea)) == opa {
                        self.write_u64(ea, 0);
                    }
                }
                if let Ok(v) = self.objs.vspace_mut(vspace) {
                    v.tables.remove(&(base, level));
                }
                self.shootdown(asid, None);
            }
        }
    }

    pub(crate) fn vspace_teardown(&mut self, r: ObjRef) {
        let (frames, tables, asid) = match self.objs.vspace_mut(r) {
            Ok(v) => (core::mem::take(&mut v.frames), core::mem::take(&mut v.tables), v.asid),
            Err(_) => return,
        };
        for a in frames.values().chain(tables.values()) {
            self.set_cap_mapping(*a, None);
        }
        self.shootdown(asid, None);
        self.asid_free.push(asid);
    }

    // ------------------------------------------------------------------
    // IOMMU
    // ------------------------------------------------------------------

    pub(crate) fn io_unmap_cap(&mut self, cap: &Cap) {
        let Some((io, iova)) = cap.io_mapped else { return };
        if let Ok(s) = self.objs.iospace_mut(io) {
            s.pages.remove(&(iova >> PAGE_SHIFT));
        }
    }

    /// IOMMU dịch một yêu cầu DMA của thiết bị. Trả danh sách (pa, offset, len)
    /// hoặc `None` nếu bị chặn. Không có IOMMU: địa chỉ thiết bị = địa chỉ vật lý.
    pub fn dma_plan(&mut self, iospace: Option<ObjRef>, dev_name: &str, addr: u64, len: usize, write: bool) -> Option<Vec<(u64, usize, usize)>> {
        if !self.iommu_enabled {
            return Some(vec![(addr, 0, len)]);
        }
        let need = if write { Rights::WRITE } else { Rights::READ };
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos < len {
            let iova = addr + pos as u64;
            let n = core::cmp::min(PAGE_SIZE - (iova & (PAGE_SIZE - 1)), (len - pos) as u64) as usize;
            let m = iospace
                .and_then(|io| self.objs.iospace(io).ok())
                .and_then(|io| io.pages.get(&(iova >> PAGE_SHIFT)).copied());
            match m {
                Some(m) if m.rights.contains(need) => {
                    out.push((m.paddr | (iova & (PAGE_SIZE - 1)), pos, n));
                }
                _ => {
                    self.stats.iommu_fault += 1;
                    let text = format!("chặn DMA của '{}' tới iova={:#x}", dev_name, iova);
                    self.klog("iommu", &text);
                    return None;
                }
            }
            pos += n;
        }
        Some(out)
    }

    // ------------------------------------------------------------------
    // syscall
    // ------------------------------------------------------------------

    pub(crate) fn sys_page_table_map(&mut self, t: ObjRef, page_table: u64, vspace: u64, vaddr: u64) -> KResult<Ret> {
        let (paddr, pcap) = self.lookup_cap(t, page_table, Some(ObjType::PageTable), Rights::NONE)?;
        let (_, vcap) = self.lookup_cap(t, vspace, Some(ObjType::VSpace), Rights::WRITE)?;
        let level = self.map_page_table(paddr, pcap, vcap.obj.ok_or(KError::InvalidCap)?, vaddr)?;
        Ok(Ret::Done(SysResult::value(level as u64)))
    }

    pub(crate) fn sys_frame_map(&mut self, t: ObjRef, frame: u64, vspace: u64, vaddr: u64, rights: Rights) -> KResult<Ret> {
        let (faddr, fcap) = self.lookup_cap(t, frame, Some(ObjType::Frame), Rights::NONE)?;
        let (_, vcap) = self.lookup_cap(t, vspace, Some(ObjType::VSpace), Rights::WRITE)?;
        self.map_frame(faddr, fcap, vcap.obj.ok_or(KError::InvalidCap)?, vaddr, rights)?;
        done()
    }

    pub(crate) fn sys_frame_unmap(&mut self, t: ObjRef, frame: u64) -> KResult<Ret> {
        let (faddr, fcap) = self.lookup_cap(t, frame, Some(ObjType::Frame), Rights::NONE)?;
        if fcap.mapped.is_none() {
            return Err(KError::NotMapped);
        }
        self.unmap_cap(&fcap);
        self.set_cap_mapping(faddr, None);
        done()
    }

    pub(crate) fn sys_io_map(&mut self, t: ObjRef, frame: u64, iospace: u64, iova: u64, rights: Rights) -> KResult<Ret> {
        let (faddr, fcap) = self.lookup_cap(t, frame, Some(ObjType::Frame), Rights::NONE)?;
        let (_, icap) = self.lookup_cap(t, iospace, Some(ObjType::IoSpace), Rights::WRITE)?;
        let fr = fcap.obj.ok_or(KError::InvalidCap)?;
        let device = self.objs.frame(fr)?.device;
        if device || iova % PAGE_SIZE != 0 {
            return Err(KError::InvalidArg);
        }
        if fcap.io_mapped.is_some() {
            return Err(KError::AlreadyMapped);
        }
        let io = icap.obj.ok_or(KError::InvalidCap)?;
        let paddr = self.objs.get(fr)?.hdr.paddr;
        {
            let space = self.objs.iospace_mut(io)?;
            let vpn = iova >> PAGE_SHIFT;
            if space.pages.contains_key(&vpn) {
                return Err(KError::AlreadyMapped);
            }
            space.pages.insert(vpn, IoMapping { paddr, rights: rights & fcap.rights, slot: faddr });
        }
        self.set_cap_io(faddr, Some((io, iova)));
        done()
    }

    pub(crate) fn sys_io_unmap(&mut self, t: ObjRef, frame: u64) -> KResult<Ret> {
        let (faddr, fcap) = self.lookup_cap(t, frame, Some(ObjType::Frame), Rights::NONE)?;
        if fcap.io_mapped.is_none() {
            return Err(KError::NotMapped);
        }
        self.io_unmap_cap(&fcap);
        self.set_cap_io(faddr, None);
        done()
    }
}
