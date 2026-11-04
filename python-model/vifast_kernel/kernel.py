"""Lõi kernel ViFastOs: máy nhiều hart, trap, khoá kernel, phân phối syscall, xử lý fault.

Mô hình thực thi (mỗi tick):
1. Phần cứng: ngắt ngoài, thiết bị DMA, timer nạp lại ngân sách, dọn CNode zombie.
2. Mỗi hart (thứ tự xoay vòng để công bằng) lập lịch rồi chạy đúng MỘT lệnh của thread hiện tại:
   - Load/Store/Fetch: lệnh máy, đi qua MMU/TLB, không vào kernel.
   - Syscall: trap (ecall), cần khoá kernel lớn (BKL như seL4 SMP). Hart khác đang giữ khoá
     trong tick này thì hart này phải chờ (spin) -> thấy được chi phí tranh chấp.
Chương trình user là generator; Rust thay bằng TrapFrame + context switch thật (crate lõi unsafe).
"""
from __future__ import annotations

from collections import Counter, deque

from . import abi
from .cspace import CSpaceMixin
from .hw import PhysMem
from .ipc import IpcMixin
from .irq import IrqMixin
from .ktypes import (BLOCKED, LABEL_FAULT, PREEMPTED, REAP_BUDGET, WORD_MAX, FaultKind,
                     KernelError, Message, ThreadState, VmFault)
from .objects import Endpoint, ObjectTable, ObjRef, Tcb
from .sched import Hart, SchedMixin
from .tcb import TcbMixin
from .vm import VmMixin


class Kernel(CSpaceMixin, IpcMixin, VmMixin, IrqMixin, TcbMixin, SchedMixin):
    def __init__(self, images: dict, *, ram_size: int = 1 << 22, harts: int = 1,
                 iommu: bool = True, verbose: bool = False) -> None:
        self.objs = ObjectTable()
        self.phys = PhysMem(ram_size)
        self.harts = [Hart(i) for i in range(harts)]
        self.cur_hart: Hart | None = None
        self.images = images
        self.verbose = verbose
        self.iommu_enabled = iommu
        self.tlb_shootdown_enabled = True
        self.ticks = 0
        self.irqs: dict = {}
        self.irq_events: list = []
        self._irq_seq = 0
        self.devices: list = []
        self.releases: list = []
        self._release_seq = 0
        self.reap_queue: deque = deque()
        self.asid_free: list[int] = []
        self.next_asid = 0
        self.lock_holder: int | None = None
        self._tcb_counter = 0
        self.stats: Counter = Counter()
        self.log: list[tuple[int, str, str]] = []
        self.boot_cnode: ObjRef | None = None
        self._dispatch = {
            abi.Send: self.sys_send, abi.Call: self.sys_call, abi.Recv: self.sys_recv,
            abi.Reply: self.sys_reply, abi.ReplyRecv: self.sys_reply_recv,
            abi.Signal: self.sys_signal, abi.Wait: self.sys_wait, abi.Poll: self.sys_poll,
            abi.Yield: self.sys_yield,
            abi.Load: self.sys_load, abi.Store: self.sys_store, abi.Fetch: self.sys_fetch,
            abi.Retype: self.sys_retype, abi.CapCopy: self.sys_cap_copy,
            abi.CapMint: self.sys_cap_mint, abi.CNodeMint: self.sys_cnode_mint,
            abi.CapDelete: self.sys_cap_delete, abi.CapRevoke: self.sys_cap_revoke,
            abi.TcbConfigure: self.sys_tcb_configure, abi.TcbResume: self.sys_tcb_resume,
            abi.TcbSuspend: self.sys_tcb_suspend, abi.TcbRestart: self.sys_tcb_restart,
            abi.TcbSetPriority: self.sys_tcb_set_priority,
            abi.TcbSetAffinity: self.sys_tcb_set_affinity,
            abi.TcbBindNotification: self.sys_tcb_bind_notification,
            abi.SchedContextConfigure: self.sys_sc_configure,
            abi.SchedContextBind: self.sys_sc_bind,
            abi.PageTableMap: self.sys_page_table_map,
            abi.FrameMap: self.sys_frame_map, abi.FrameUnmap: self.sys_frame_unmap,
            abi.IoMap: self.sys_io_map, abi.IoUnmap: self.sys_io_unmap,
            abi.IrqGet: self.sys_irq_get, abi.IrqSetNotification: self.sys_irq_set_notification,
            abi.IrqAck: self.sys_irq_ack,
            abi.DebugPrint: self.sys_debug_print, abi.GetTime: self.sys_get_time,
        }

    # ---------- tiện ích ----------
    def klog(self, source: str, text: str) -> None:
        self.log.append((self.ticks, source, text))
        if self.verbose:
            print(f"[{self.ticks:6d}] {source:<14} {text}")

    def lines(self, source: str | None = None) -> list[str]:
        return [txt for _, src, txt in self.log if source is None or src == source]

    def find_tcb(self, name: str) -> tuple[ObjRef, Tcb] | None:
        for ref, obj in self.objs.items():
            if isinstance(obj, Tcb) and obj.name == name:
                return ref, obj
        return None

    def attach_device(self, dev) -> None:
        dev.kernel = self
        self.phys.devices.append(dev)
        self.devices.append(dev)

    def report(self) -> dict:
        r = dict(self.stats)
        r["ticks"] = self.ticks
        for h in self.harts:
            r[f"hart{h.id}"] = (f"busy={h.busy} idle={h.idle} ctx_switch={h.ctx_switches} "
                                f"traps={h.traps} timer_irq={h.timer_irqs} "
                                f"tlb_hit={h.tlb.hits} tlb_miss={h.tlb.misses}")
        return r

    # ---------- vòng lặp máy ----------
    def _hardware_pending(self) -> bool:
        return bool(self.irq_events or self.reap_queue or any(d.busy for d in self.devices)
                    or (self.releases and any(isinstance(o, Tcb) and o.throttled
                                              for _, o in self.objs.items())))

    def run(self, max_ticks: int = 10_000) -> "Kernel":
        for _ in range(max_ticks):
            if not self.step() and not self._hardware_pending():
                break
        return self

    def step(self) -> bool:
        self.ticks += 1
        self.poll_hardware()
        self.process_releases()
        self.reap(REAP_BUDGET)
        self.lock_holder = None
        busy = False
        n = len(self.harts)
        for i in range(n):
            busy |= self.run_hart(self.harts[(self.ticks + i) % n])
        return busy

    def run_hart(self, h: Hart) -> bool:
        ref = self.hart_schedule(h)
        if ref is None:
            h.idle += 1
            return False
        h.busy += 1
        t = self.objs.get(ref)
        self.cur_hart = h
        h.mode = "U"
        self._run_one(ref, t)
        h.mode = "S"
        self.cur_hart = None
        if self.objs.try_get(ref) is not t:
            h.current = None
        elif t.state == ThreadState.RUNNING:
            self.charge(h, ref, t)
        elif h.current == ref:
            h.current = None
        return True

    def _run_one(self, ref: ObjRef, t: Tcb) -> None:
        if t.retry_op is not None:
            op, t.retry_op = t.retry_op, None
        else:
            try:
                if t.program is None:
                    t.program = self.images[t.image](dict(t.env))
                    op = next(t.program)
                else:
                    value, t.resume_value = t.resume_value, None
                    op = t.program.send(value)
            except StopIteration:
                t.program = None
                self.raise_fault(ref, t, FaultKind.EXITED)
                return
            except Exception as e:          # panic trong user code
                t.program = None
                self.raise_fault(ref, t, FaultKind.USER_EXCEPTION, detail=f"{type(e).__name__}: {e}")
                return
        self._execute(ref, t, op)

    def _execute(self, ref: ObjRef, t: Tcb, op) -> None:
        h = self.cur_hart
        handler = self._dispatch.get(type(op))
        if handler is None:
            self.reset_context(t)
            self.raise_fault(ref, t, FaultKind.BAD_SYSCALL, detail=repr(op))
            return
        is_syscall = not isinstance(op, abi.USER_INSTRUCTIONS)
        if is_syscall:
            if self.lock_holder not in (None, h.id):
                t.retry_op = op                      # hart khác đang trong kernel: spin
                self.stats["lock_contention"] += 1
                return
            self.lock_holder = h.id
            h.traps += 1
            h.mode = "S"
            t.regs.a[7] = abi.SYSCALL_NUMBER[type(op)]
        try:
            result = handler(ref, t, op)
        except KernelError as e:
            result = abi.SysResult(err=e.code)
        except VmFault as f:
            t.retry_op = op
            self.raise_fault(ref, t, FaultKind.VM_FAULT, addr=f.addr)
            return
        if result is PREEMPTED:
            t.retry_op = op                          # chạy lại syscall ở lượt sau
            return
        if is_syscall:
            t.regs.pc += 4
        if result is not BLOCKED and self.objs.try_get(ref) is t:
            t.resume_value = result
            self.write_result_regs(t, result)

    @staticmethod
    def write_result_regs(t: Tcb, result) -> None:
        """Quy ước trả về: a0 = mã lỗi, a1 = value/badge. Rust: ghi vào TrapFrame."""
        if isinstance(result, abi.SysResult):
            t.regs.a[0] = int(result.err)
            t.regs.a[1] = (result.value or result.badge) & WORD_MAX

    def raise_fault(self, ref: ObjRef, t: Tcb, kind: FaultKind, addr: int = 0, detail: str = "") -> None:
        """Gửi fault tới fault handler như một Call; không có handler thì thread dừng ở FAULTED."""
        if kind == FaultKind.EXITED:
            self.klog("kernel", f"'{t.name}' kết thúc")
        else:
            self.klog("kernel", f"fault {kind.name} ở '{t.name}' addr={addr:#x} {detail}".rstrip())
        t.fault_kind = kind
        if not isinstance(self.objs.try_get(t.fault_ep), Endpoint):
            t.state = ThreadState.FAULTED
            return
        self.ipc_send(ref, t, t.fault_ep, t.fault_badge,
                      Message(LABEL_FAULT, (int(kind), addr)), is_call=True, can_grant=False)

    # ---------- syscall nhỏ ----------
    def sys_yield(self, ref, t, op):
        t.timeslice = 0
        self.cur_hart.resched = True
        return abi.SysResult()

    def sys_debug_print(self, ref, t, op: abi.DebugPrint):
        self.klog(t.name, op.text)
        return abi.SysResult()

    def sys_get_time(self, ref, t, op):
        return abi.SysResult(value=self.ticks)

