"""Dựng hệ thống từ manifest (TOML). Đóng vai trò "root task" / công cụ build image.

Hai profile dùng chung một kernel:
- desktop: task được phát untyped + IRQ control + sched control -> tạo tiến trình,
  nhận thiết bị, cấu hình ngân sách CPU lúc chạy.
- space:   cấm phát các quyền đó; boot xong xoá luôn untyped gốc, IRQ control, sched control
  -> không ai còn quyền tạo object mới: hệ thống "tĩnh" nhờ capability, kernel không có chế độ riêng.
Quy ước fault: badge của fault endpoint = slot chứa TCB cap của task đó trong supervisor.
"""
from __future__ import annotations

import tomllib

from .hw import DEVICE_KINDS, KERNEL_RESERVED, RAM_PHYS_BASE
from .kernel import Kernel
from .ktypes import PAGE_SIZE, WORD_BITS, KError, KernelError, ObjType, Rights
from .objects import (Cap, CNode, Frame, IoSpace, IrqControl, SchedControl, Slot, SlotAddr,
                      Untyped)

BOOT_CNODE_BITS = 12
RAM_BASE = 0x10_0000          # địa chỉ ảo vùng RAM riêng của mỗi task
OBJECT_KINDS = {"endpoint": ObjType.ENDPOINT, "notification": ObjType.NOTIFICATION,
                "frame": ObjType.FRAME}


class BootError(Exception):
    pass


def load_manifest(path: str) -> dict:
    with open(path, "rb") as f:
        return tomllib.load(f)


class _Builder:
    def __init__(self, k: Kernel, mem: int) -> None:
        self.k = k
        self.cn = k.objs.insert(CNode(slots=[Slot() for _ in range(1 << BOOT_CNODE_BITS)], refs=1))
        k.boot_cnode = self.cn
        self.next = 1
        ut = Untyped(size=mem, paddr=RAM_PHYS_BASE + KERNEL_RESERVED)
        self.ut = self.put(Cap(ObjType.UNTYPED, k.objs.insert(ut), Rights.ALL))
        self.irqc = self.put(Cap(ObjType.IRQ_CONTROL, k.objs.insert(IrqControl()), Rights.ALL))
        self.schedc = self.put(Cap(ObjType.SCHED_CONTROL, k.objs.insert(SchedControl()), Rights.ALL))
        self.dev_frame: dict[str, SlotAddr] = {}
        self.dev_io: dict[str, SlotAddr] = {}

    def fresh(self) -> SlotAddr:
        addr = SlotAddr(self.cn, self.next)
        self.next += 1
        return addr

    def put(self, cap: Cap) -> SlotAddr:
        addr = self.fresh()
        self.k.insert_cap(addr, cap, None)
        return addr

    def cap(self, addr: SlotAddr) -> Cap:
        return self.k.slot(addr).cap

    def new(self, obj_type: ObjType, bits: int = 0) -> SlotAddr:
        dest = self.fresh()
        self.k.retype_objects(self.ut, self.cap(self.ut), obj_type, bits, [dest])
        return dest

    def map(self, faddr: SlotAddr, vs_ref, vaddr: int, rights: Rights) -> None:
        """Map frame, tự tạo page table trung gian khi thiếu (việc của root task)."""
        while True:
            try:
                self.k.map_frame(faddr, self.k.slot(faddr).cap, vs_ref, vaddr, rights)
                return
            except KernelError as e:
                if e.code != KError.MISSING_PAGE_TABLE:
                    raise
            pt = self.new(ObjType.PAGE_TABLE)
            self.k.map_page_table(pt, self.cap(pt), vs_ref, vaddr)


def build(manifest: dict, images: dict, *, verbose: bool = False) -> Kernel:
    profile = manifest.get("profile", "desktop")
    if profile not in ("desktop", "space"):
        raise BootError(f"profile không hợp lệ: {profile}")
    mem = int(manifest.get("memory", 1 << 22))
    k = Kernel(images, ram_size=mem, harts=int(manifest.get("harts", 1)),
               iommu=bool(manifest.get("iommu", True)), verbose=verbose)
    b = _Builder(k, mem)
    try:
        _build(k, b, manifest, profile)
    except KernelError as e:
        raise BootError(f"lỗi kernel khi boot: {e.code.name}") from e
    except KeyError as e:
        raise BootError(f"manifest thiếu hoặc sai tên: {e}") from e
    k.klog("boot", f"profile={profile}, {len(manifest.get('tasks', []))} task, "
                   f"{len(k.harts)} hart, iommu={'bật' if k.iommu_enabled else 'tắt'}")
    return k


def _build(k: Kernel, b: _Builder, m: dict, profile: str) -> None:
    for d in m.get("devices", []):
        dev = DEVICE_KINDS[d["kind"]](d["name"], int(d["mmio"]), int(d["irq"]))
        k.attach_device(dev)
        b.dev_frame[d["name"]] = b.put(Cap(ObjType.FRAME, k.objs.insert(
            Frame(device=True, paddr=dev.mmio_base)), Rights.READ | Rights.WRITE))
        io_slot = b.put(Cap(ObjType.IO_SPACE, k.objs.insert(IoSpace(device=d["name"])), Rights.ALL))
        b.dev_io[d["name"]] = io_slot
        dev.iospace = b.cap(io_slot).obj

    objects = {name: b.new(OBJECT_KINDS[kind]) for name, kind in m.get("objects", {}).items()}
    tasks = {}
    for spec in m.get("tasks", []):
        name = spec["name"]
        if name in tasks or name in objects:
            raise BootError(f"trùng tên: {name}")
        tasks[name] = {"spec": spec,
                       "cnode": b.new(ObjType.CNODE, int(spec.get("cnode_bits", 6))),
                       "tcb": b.new(ObjType.TCB)}
    for name, info in tasks.items():
        shared = info["spec"].get("share_vspace")
        info["vspace"] = None if shared else b.new(ObjType.VSPACE)
    for name, info in tasks.items():
        shared = info["spec"].get("share_vspace")
        if shared:
            info["vspace"] = tasks[shared]["vspace"]

    for name, info in tasks.items():
        spec = info["spec"]
        cn_ref = b.cap(info["cnode"]).obj
        vs_ref = b.cap(info["vspace"]).obj
        pages = 0 if spec.get("share_vspace") else int(spec.get("ram_pages", 1))
        for i in range(pages):
            b.map(b.new(ObjType.FRAME), vs_ref, RAM_BASE + i * PAGE_SIZE, Rights.READ | Rights.WRITE)
        env = {"ram_base": RAM_BASE, "ram_pages": pages}
        used = set()
        for c in spec.get("caps", []):
            slot = int(c["slot"])
            if slot in used or slot == 0:
                raise BootError(f"{name}: slot {slot} trùng hoặc bằng 0")
            used.add(slot)
            _grant(k, b, profile, name, c, SlotAddr(cn_ref, slot), objects, tasks, info, vs_ref, env)
            env[c["name"]] = slot
        env.update(spec.get("env", {}))
        fault_ep, fault_badge = None, 0
        if "fault" in spec:
            fault_ep = b.cap(objects[spec["fault"]["obj"]]).obj
            fault_badge = int(spec["fault"].get("badge", 0))
        prio = int(spec.get("priority", 100))
        tref = b.cap(info["tcb"]).obj
        t = k.objs.get(tref)
        radix = k.objs.get(cn_ref).radix
        k.configure_tcb(t, name=name, image=spec["image"], priority=prio,
                        mcp=int(spec.get("mcp", prio)), cspace=cn_ref, guard=0,
                        guard_bits=WORD_BITS - radix, vspace=vs_ref, fault_ep=fault_ep,
                        fault_badge=fault_badge, env=env, affinity=int(spec.get("core", 0)))
        if "bind_notification" in spec:
            k.bind_notification(tref, t, b.cap(objects[spec["bind_notification"]]).obj)
        if "budget" in spec:
            sc = b.new(ObjType.SCHED_CONTEXT)
            k.configure_sc(b.cap(sc).obj, int(spec["budget"]), int(spec["period"]))
            k.bind_sc(b.cap(sc).obj, tref)

    for name, info in tasks.items():
        if info["spec"].get("start", True):
            tref = b.cap(info["tcb"]).obj
            k.make_ready(tref, k.objs.get(tref), None)

    if profile == "space":
        # Khoá hệ thống: không còn untyped, IRQ control hay sched control ở đâu cả.
        for slot in (b.schedc, b.irqc, b.ut):
            k.delete_slot(slot)


def _grant(k, b, profile, task, c, dest, objects, tasks, info, vs_ref, env) -> None:
    rights = Rights.parse(c.get("rights", "RWXG"))
    if "obj" in c:
        src = objects[c["obj"]]
        k.insert_cap(dest, b.cap(src).derive(rights, int(c.get("badge", 0))), src)
        if "map" in c:
            b.map(dest, vs_ref, int(c["map"]), rights)
            env[c["name"] + "_addr"] = int(c["map"])
    elif "device" in c:
        src = b.dev_frame[c["device"]]
        k.insert_cap(dest, b.cap(src).derive(rights), src)
        if "map" in c:
            b.map(dest, vs_ref, int(c["map"]), rights & (Rights.READ | Rights.WRITE))
            env[c["name"] + "_addr"] = int(c["map"])
    elif "iospace" in c:
        src = b.dev_io[c["iospace"]]
        k.insert_cap(dest, b.cap(src).derive(rights), src)
    elif any(key in c for key in ("tcb", "cnode", "vspace")):
        key = next(key for key in ("tcb", "cnode", "vspace") if key in c)
        src = tasks[c[key]][key]
        k.insert_cap(dest, b.cap(src).derive(rights), src)
    elif "self" in c:
        src = info[c["self"]]
        k.insert_cap(dest, b.cap(src).derive(rights), src)
    elif "irq" in c:
        irq_slot = b.fresh()
        k.create_irq_handler(b.irqc, int(c["irq"]), irq_slot)
        k.insert_cap(dest, b.cap(irq_slot).derive(Rights.ALL), irq_slot)
        if "notification" in c:
            k.bind_irq(int(c["irq"]), b.cap(objects[c["notification"]]))
    elif "untyped_bits" in c or "irq_control" in c or "sched_control" in c:
        if profile == "space":
            raise BootError(f"{task}: profile space không cho phép cấp untyped/irq_control/sched_control")
        if "untyped_bits" in c:
            src = b.new(ObjType.UNTYPED, int(c["untyped_bits"]))
        else:
            src = b.irqc if "irq_control" in c else b.schedc
        k.insert_cap(dest, b.cap(src).derive(Rights.ALL), src)
    else:
        raise BootError(f"{task}: không hiểu cap {c}")
