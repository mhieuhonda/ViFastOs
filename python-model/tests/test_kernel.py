"""Bộ test hành vi kernel. Khi port sang Rust, mỗi test ở đây nên có test tương ứng
(chạy trên host hoặc QEMU) để chứng minh bản Rust giữ đúng ngữ nghĩa."""
import os
import sys
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

from vifast_kernel import IMAGES, BootError, build, load_manifest  # noqa: E402
from vifast_kernel.abi import (Call, CapCopy, CapDelete, CapMint, CapRevoke, CNodeMint,  # noqa: E402
                               DebugPrint, Fetch, FrameUnmap, IrqAck, Load, Recv, Reply,
                               Retype, Send, Signal, Store, TcbConfigure, TcbResume, Wait, Yield)
from vifast_kernel.hw import RAM_PHYS_BASE  # noqa: E402
from vifast_kernel.ktypes import KError, Message, ObjType, ThreadState  # noqa: E402
from vifast_kernel.objects import CNode, Untyped  # noqa: E402

MANIFESTS = os.path.join(ROOT, "manifests")


def boot(manifest, extra_images=None, **kw):
    images = dict(IMAGES)
    images.update(extra_images or {})
    return build(manifest, images, **kw)


def spin(label, n=3):
    def prog(env):
        for i in range(n):
            yield DebugPrint(f"{label}{i}")
    return prog


# ======================================================================
class DemoTests(unittest.TestCase):
    def test_space_profile(self):
        k = boot(load_manifest(os.path.join(MANIFESTS, "space.toml")))
        k.schedule_irq(5, at=2, period=8)
        k.run(1500)
        self.assertIn("hoàn thành điều khiển tư thế", k.lines("attitude"))
        self.assertEqual(sum("restart payload" in x for x in k.lines("supervisor")), 3)
        self.assertTrue(any("DEGRADED: payload" in x for x in k.lines("supervisor")))
        self.assertEqual(len(k.lines("consumer")), 3)
        samples = [int(x.split("=")[1]) for x in k.lines("attitude") if "mẫu IMU" in x]
        self.assertEqual(samples, sorted(samples))
        self.assertGreater(samples[-1], 10)

    def test_space_is_static(self):
        k = boot(load_manifest(os.path.join(MANIFESTS, "space.toml")))
        forbidden = (ObjType.UNTYPED, ObjType.IRQ_CONTROL, ObjType.SCHED_CONTROL)
        for _, obj in k.objs.items():
            if isinstance(obj, CNode):
                for s in obj.slots:
                    if s.cap is not None:
                        self.assertNotIn(s.cap.obj_type, forbidden)

    def test_space_rejects_untyped(self):
        m = {"profile": "space", "tasks": [
            {"name": "a", "image": "hello", "caps": [{"name": "mem", "slot": 1, "untyped_bits": 12}]}]}
        with self.assertRaises(BootError):
            boot(m)

    def test_desktop_profile(self):
        k = boot(load_manifest(os.path.join(MANIFESTS, "desktop.toml")))
        k.run(2000)
        self.assertEqual(len([x for x in k.lines("console") if x.startswith("[badge 7]")]), 3)
        self.assertEqual(sum("cấp trang" in x for x in k.lines("pager")), 3)
        self.assertIn("đọc lại: trang 0", k.lines("lazy"))
        self.assertEqual(k.lines("cam_driver"), [f"DMA xong (status=2): SENSOR-{i}" for i in (1, 2, 3)])
        self.assertTrue(k.phys.kernel_canary_intact())


# ======================================================================
class CapabilityTests(unittest.TestCase):
    def test_cap_transfer_and_revoke(self):
        def server(env):
            while True:
                yield Recv(env["svc"])
                yield Reply(Message(0, (42,)))

        def owner(env):
            yield Send(env["handoff"], Message(0, caps=(env["svc"],)))
            yield Wait(env["done"])
            yield CapRevoke(env["svc"])
            yield Signal(env["revoked"])

        def client(env):
            r = yield Recv(env["handoff"], recv_slot=5)
            slot = r.msg.caps[0]
            a = yield Call(slot, Message(1))
            yield DebugPrint(f"trước: {a.err.name} {a.msg.data}")
            yield Signal(env["done"])
            yield Wait(env["revoked"])
            b = yield Call(slot, Message(1))
            yield DebugPrint(f"sau: {b.err.name}")

        m = {"objects": {"svc": "endpoint", "handoff": "endpoint",
                         "done": "notification", "revoked": "notification"},
             "tasks": [
                 {"name": "server", "image": "server", "priority": 100,
                  "caps": [{"name": "svc", "slot": 1, "obj": "svc", "rights": "R"}]},
                 {"name": "owner", "image": "owner", "priority": 90, "caps": [
                     {"name": "svc", "slot": 1, "obj": "svc", "rights": "W"},
                     {"name": "handoff", "slot": 2, "obj": "handoff", "rights": "WG"},
                     {"name": "done", "slot": 3, "obj": "done", "rights": "R"},
                     {"name": "revoked", "slot": 4, "obj": "revoked", "rights": "W"}]},
                 {"name": "client", "image": "client", "priority": 80, "caps": [
                     {"name": "handoff", "slot": 1, "obj": "handoff", "rights": "R"},
                     {"name": "done", "slot": 2, "obj": "done", "rights": "W"},
                     {"name": "revoked", "slot": 3, "obj": "revoked", "rights": "R"}]}]}
        k = boot(m, {"server": server, "owner": owner, "client": client}).run(500)
        self.assertEqual(k.lines("client"), ["trước: OK (42,)", "sau: INVALID_CAP"])

    def test_untyped_exhaustion_and_reclaim(self):
        def hog(env):
            n = 0
            while (yield Retype(env["mem"], ObjType.FRAME, 0, 10 + n)).ok:
                n += 1
            yield DebugPrint(f"cấp được {n} trang")
            yield CapRevoke(env["mem"])
            r = yield Retype(env["mem"], ObjType.FRAME, 0, 10)
            yield DebugPrint(f"sau revoke: {r.err.name}")

        m = {"tasks": [
            {"name": "hog", "image": "hog", "priority": 100, "cnode_bits": 5,
             "caps": [{"name": "mem", "slot": 1, "untyped_bits": 14}]},
            {"name": "bystander", "image": "bystander", "priority": 100}]}
        k = boot(m, {"hog": hog, "bystander": spin("B")}).run(500)
        self.assertEqual(k.lines("hog"), ["cấp được 4 trang", "sau revoke: OK"])
        self.assertEqual(k.lines("bystander"), ["B0", "B1", "B2"])

    def test_retype_zeroes_memory(self):
        """Bộ nhớ thu hồi rồi cấp lại phải được xoá trắng (không rò dữ liệu cũ)."""
        def prog(env):
            from vifast_kernel.abi import FrameMap, PageTableMap
            yield Retype(env["mem"], ObjType.FRAME, 0, 10)
            yield Retype(env["mem"], ObjType.PAGE_TABLE, 0, 11)
            yield Retype(env["mem"], ObjType.PAGE_TABLE, 0, 12)
            yield PageTableMap(11, env["vs"], 0x4000_0000)
            yield PageTableMap(12, env["vs"], 0x4000_0000)
            yield FrameMap(10, env["vs"], 0x4000_0000)
            yield Store(0x4000_0000, b"BI-MAT")
            yield CapRevoke(env["mem"])
            yield Retype(env["mem"], ObjType.FRAME, 0, 10)
            yield Retype(env["mem"], ObjType.PAGE_TABLE, 0, 11)
            yield Retype(env["mem"], ObjType.PAGE_TABLE, 0, 12)
            yield PageTableMap(11, env["vs"], 0x4000_0000)
            yield PageTableMap(12, env["vs"], 0x4000_0000)
            yield FrameMap(10, env["vs"], 0x4000_0000)
            yield DebugPrint(repr((yield Load(0x4000_0000, 6)).data))

        m = {"tasks": [{"name": "p", "image": "p", "caps": [
            {"name": "mem", "slot": 1, "untyped_bits": 14},
            {"name": "vs", "slot": 2, "self": "vspace"}]}]}
        k = boot(m, {"p": prog}).run(200)
        self.assertEqual(k.lines("p"), [repr(bytes(6))])

    def test_multilevel_cspace_with_guard(self):
        """CSpace 2 cấp: gốc radix 4 + guard 52 bit, cấp 2 radix 8. cptr = (2 << 8) | 5."""
        def spawner(env):
            for slot, typ, bits in ((10, ObjType.CNODE, 4), (11, ObjType.CNODE, 8),
                                    (12, ObjType.TCB, 0), (13, ObjType.VSPACE, 0)):
                yield Retype(env["mem"], typ, bits, slot)
            yield CNodeMint(11, dest_index=2, guard=0, guard_bits=0, dest_cnode=10)
            yield CapMint(env["console"], dest_index=5, rights=0b0010, badge=9, dest_cnode=11)
            yield CNodeMint(10, dest_index=14, guard=0, guard_bits=52)
            r = yield TcbConfigure(12, cspace=14, vspace=13, priority=50, mcp=50, image="client",
                                   env={"ep": (2 << 8) | 5, "bad": 5})
            yield DebugPrint(f"configure {r.err.name}")
            yield TcbResume(12)

        def client(env):
            r = yield Call(env["bad"], Message(1))     # cptr 1 cấp không còn hợp lệ
            yield DebugPrint(f"cptr phẳng: {r.err.name}")
            r = yield Call(env["ep"], Message(1))
            yield DebugPrint(f"cptr 2 cấp: {r.err.name} {r.msg.data}")

        def server(env):
            r = yield Recv(env["ep"])
            yield Reply(Message(0, (r.badge,)))

        m = {"objects": {"ep": "endpoint"}, "tasks": [
            {"name": "server", "image": "server", "priority": 100,
             "caps": [{"name": "ep", "slot": 1, "obj": "ep", "rights": "R"}]},
            {"name": "spawner", "image": "spawner", "priority": 90, "caps": [
                {"name": "mem", "slot": 1, "untyped_bits": 18},
                {"name": "console", "slot": 2, "obj": "ep", "rights": "WG"}]}]}
        k = boot(m, {"spawner": spawner, "client": client, "server": server}).run(300)
        self.assertIn("configure OK", k.lines("spawner"))
        out = [x for x in k.lines() if x.startswith("cptr")]
        self.assertEqual(out, ["cptr phẳng: INVALID_CAP", "cptr 2 cấp: OK (9,)"])

    def test_revoke_is_preemptible(self):
        def prog(env):
            for i in range(40):
                yield CapCopy(env["ep"], dest_index=20 + i)
            r = yield CapRevoke(env["ep"])
            yield DebugPrint(f"revoke {r.err.name}")
            r = yield Call(20, Message())
            yield DebugPrint(f"bản copy: {r.err.name}")

        m = {"objects": {"ep": "endpoint"}, "tasks": [
            {"name": "p", "image": "p", "cnode_bits": 7,
             "caps": [{"name": "ep", "slot": 1, "obj": "ep", "rights": "RW"}]}]}
        k = boot(m, {"p": prog}).run(300)
        self.assertEqual(k.lines("p"), ["revoke OK", "bản copy: INVALID_CAP"])
        self.assertEqual(k.stats["revoke_preempt"], 4)       # 40 cap / 8 mỗi lượt

    def test_zombie_cnode_cycle_is_reclaimed(self):
        """CNode chứa cap trỏ về chính nó: xoá cap ngoài thì chưa giải phóng,
        revoke untyped thì vòng bị phá, CNode được dọn dần và bộ nhớ trở về."""
        def prog(env):
            yield Retype(env["mem"], ObjType.CNODE, 6, 10)
            yield CapCopy(10, dest_index=1, dest_cnode=10)
            yield CapDelete(10)
            yield DebugPrint("đã xoá cap ngoài")
            yield CapRevoke(env["mem"])
            for _ in range(10):
                yield Yield()
            r = yield Retype(env["mem"], ObjType.UNTYPED, 14, 11)
            yield DebugPrint(f"cấp lại toàn bộ: {r.err.name}")

        m = {"tasks": [{"name": "p", "image": "p",
                        "caps": [{"name": "mem", "slot": 1, "untyped_bits": 14}]}]}
        k = boot(m, {"p": prog}).run(300)
        self.assertEqual(k.lines("p"), ["đã xoá cap ngoài", "cấp lại toàn bộ: OK"])
        self.assertEqual(k.stats["zombie_reaped"], 1)


# ======================================================================
class SchedulingTests(unittest.TestCase):
    def test_priority(self):
        m = {"tasks": [{"name": "lo", "image": "lo", "priority": 10},
                       {"name": "hi", "image": "hi", "priority": 20}]}
        k = boot(m, {"lo": spin("L"), "hi": spin("H")}).run(100)
        order = [x for x in k.lines() if x[0] in "LH" and len(x) == 2]
        self.assertEqual(order, ["H0", "H1", "H2", "L0", "L1", "L2"])

    def test_timer_round_robin(self):
        m = {"tasks": [{"name": "a", "image": "a", "priority": 10},
                       {"name": "b", "image": "b", "priority": 10}]}
        k = boot(m, {"a": spin("A", 12), "b": spin("B", 12)}).run(200)
        seq = "".join(x[0] for x in k.lines() if x[0] in "AB")
        self.assertTrue(seq.startswith("AAAAABBBBB") or seq.startswith("BBBBBAAAAA"))
        self.assertGreater(k.harts[0].timer_irqs, 3)

    def test_mcp_limits_priority(self):
        def spawner(env):
            yield Retype(env["mem"], ObjType.TCB, 0, 10)
            yield Retype(env["mem"], ObjType.CNODE, 4, 11)
            yield Retype(env["mem"], ObjType.VSPACE, 0, 12)
            r = yield TcbConfigure(10, 11, 12, priority=200, mcp=0, image="hello")
            yield DebugPrint(r.err.name)

        m = {"tasks": [{"name": "s", "image": "spawner", "priority": 50, "mcp": 50,
                        "caps": [{"name": "mem", "slot": 1, "untyped_bits": 16}]}]}
        k = boot(m, {"spawner": spawner}).run(100)
        self.assertEqual(k.lines("s"), ["PRIO_TOO_HIGH"])

    def _hog_vs_victim(self, with_budget):
        def hog(env):
            while True:
                yield Load(env["ram_base"], 8)

        def victim(env):
            for i in range(1000):
                yield Load(env["ram_base"], 8)
                if i % 10 == 9:
                    yield DebugPrint("tiến độ")

        hog_spec = {"name": "hog", "image": "hog", "priority": 200}
        if with_budget:
            hog_spec.update(budget=2, period=10)
        m = {"tasks": [hog_spec, {"name": "victim", "image": "victim", "priority": 10}]}
        k = boot(m, {"hog": hog, "victim": victim}).run(300)
        return len(k.lines("victim")), k

    def test_budget_gives_temporal_isolation(self):
        without, _ = self._hog_vs_victim(False)
        with_sc, k = self._hog_vs_victim(True)
        self.assertEqual(without, 0)                 # ưu tiên cao lặp vô hạn: victim chết đói
        self.assertGreater(with_sc, 15)              # có budget 2/10: victim nhận ~80% CPU
        self.assertGreater(k.stats["throttle"], 20)

    def test_two_harts_run_in_parallel(self):
        def worker(env):
            for _ in range(20):
                yield Load(env["ram_base"], 8)       # lệnh máy, không vào kernel
            yield DebugPrint("xong")

        def tasks(n_harts):
            return {"harts": n_harts, "tasks": [
                {"name": "a", "image": "w", "core": 0},
                {"name": "b", "image": "w", "core": n_harts - 1}]}
        one = boot(tasks(1), {"w": worker}).run(500)
        two = boot(tasks(2), {"w": worker}).run(500)
        self.assertLess(two.ticks * 1.6, one.ticks)
        self.assertLessEqual(two.stats["lock_contention"], 1)   # chỉ DebugPrint cuối có thể trùng tick

    def test_big_kernel_lock_serializes_syscalls(self):
        m = {"harts": 2, "tasks": [{"name": "a", "image": "a", "core": 0},
                                   {"name": "b", "image": "b", "core": 1}]}
        k = boot(m, {"a": spin("A", 20), "b": spin("B", 20)}).run(500)
        self.assertGreater(k.stats["lock_contention"], 15)
        self.assertEqual(len(k.lines("a")) + len(k.lines("b")), 40)


# ======================================================================
class MemoryTests(unittest.TestCase):
    def test_sv39_page_tables_live_in_physical_memory(self):
        k = boot({"tasks": [{"name": "a", "image": "hello", "ram_pages": 2}]})
        _, t = k.find_tcb("a")
        vs = k.objs.get(t.vspace)
        pte = k.sv39_walk(vs.paddr, 0x10_1000)
        self.assertIsNotNone(pte)
        self.assertTrue(pte & 0x1 and pte & 0x10)                # V và U
        self.assertGreaterEqual((pte >> 10) << 12, RAM_PHYS_BASE)

    def test_wx_and_exec_permission(self):
        from vifast_kernel.abi import FrameMap, PageTableMap
        from vifast_kernel.ktypes import Rights

        def prog(env):
            yield Retype(env["mem"], ObjType.FRAME, 0, 10)
            r = yield FrameMap(10, env["vs"], 0x10_3000, Rights.READ | Rights.WRITE | Rights.EXEC)
            yield DebugPrint(f"RWX: {r.err.name}")
            r = yield FrameMap(10, env["vs"], 0x10_3000, Rights.READ | Rights.EXEC)
            yield DebugPrint(f"RX: {r.err.name}")
            yield Fetch(0x10_3000)
            yield DebugPrint("fetch trang RX được")
            yield Fetch(env["ram_base"])                         # trang RW: không được thực thi
            yield DebugPrint("không bao giờ tới đây")

        m = {"tasks": [{"name": "p", "image": "p", "caps": [
            {"name": "mem", "slot": 1, "untyped_bits": 14},
            {"name": "vs", "slot": 2, "self": "vspace"}]}]}
        k = boot(m, {"p": prog}).run(100)
        self.assertEqual(k.lines("p"), ["RWX: ILLEGAL_OPERATION", "RX: OK", "fetch trang RX được"])
        self.assertEqual(k.find_tcb("p")[1].state, ThreadState.FAULTED)

    def _shootdown_case(self, shootdown):
        def reader(env):
            while True:
                r = yield Load(env["buf_addr"], 4)
                yield DebugPrint(r.data.decode())

        def unmapper(env):
            yield Store(env["buf_addr"], b"DATA")
            for _ in range(6):
                yield Yield()
            yield FrameUnmap(env["buf"])
            yield DebugPrint("đã unmap")

        m = {"harts": 2, "objects": {"buf": "frame"}, "tasks": [
            {"name": "unmapper", "image": "unmapper", "core": 0, "priority": 100,
             "caps": [{"name": "buf", "slot": 1, "obj": "buf", "rights": "RW", "map": 0x200000}],
             "env": {"buf_addr": 0x200000}},
            {"name": "reader", "image": "reader", "core": 1, "priority": 100,
             "share_vspace": "unmapper", "env": {"buf_addr": 0x200000}}]}
        k = boot(m, {"reader": reader, "unmapper": unmapper})
        k.tlb_shootdown_enabled = shootdown
        k.run(60)
        return k

    def test_tlb_shootdown_across_harts(self):
        k = self._shootdown_case(True)
        self.assertEqual(k.find_tcb("reader")[1].state, ThreadState.FAULTED)
        self.assertGreater(k.stats["ipi"], 0)
        # Không shootdown: hart 1 vẫn đọc được qua TLB cũ sau khi đã unmap (lỗ hổng)
        bad = self._shootdown_case(False)
        t_unmap = next(t for t, s, x in bad.log if x == "đã unmap")
        stale = [x for t, s, x in bad.log if s == "reader" and t > t_unmap]
        self.assertTrue(stale)

    def test_vspace_destroy_frees_asid(self):
        def prog(env):
            for i in range(3):
                yield Retype(env["mem"], ObjType.VSPACE, 0, 10)
                yield CapDelete(10)
            yield DebugPrint("ok")
        k = boot({"tasks": [{"name": "p", "image": "p",
                             "caps": [{"name": "mem", "slot": 1, "untyped_bits": 14}]}]},
                 {"p": prog}).run(100)
        self.assertEqual(k.lines("p"), ["ok"])
        self.assertEqual(k.next_asid, 2)            # 1 cho task p, 1 được dùng lại


# ======================================================================
class DmaTests(unittest.TestCase):
    def _attack(self, iommu):
        m = {"iommu": iommu,
             "devices": [{"name": "camera", "kind": "dma_sensor", "mmio": 0x1000_0000, "irq": 7}],
             "objects": {"cam_ntfn": "notification"},
             "tasks": [{"name": "evil", "image": "dma_attacker", "env": {"target": RAM_PHYS_BASE},
                        "caps": [
                            {"name": "cam_regs", "slot": 1, "device": "camera", "rights": "RW",
                             "map": 0x5000_0000},
                            {"name": "cam_irq", "slot": 2, "irq": 7, "notification": "cam_ntfn"},
                            {"name": "cam_ntfn", "slot": 3, "obj": "cam_ntfn", "rights": "R"}]}]}
        return boot(m).run(100)

    def test_iommu_blocks_malicious_dma(self):
        k = self._attack(True)
        self.assertTrue(k.phys.kernel_canary_intact())
        self.assertEqual(k.lines("evil"), ["status=3"])
        self.assertEqual(k.stats["iommu_fault"], 1)

    def test_without_iommu_dma_corrupts_kernel(self):
        k = self._attack(False)
        self.assertFalse(k.phys.kernel_canary_intact())
        self.assertEqual(k.lines("evil"), ["status=2"])


# ======================================================================
class FaultTests(unittest.TestCase):
    def test_server_crash_cancels_waiting_client(self):
        def flaky(env):
            yield Recv(env["ep"])
            raise ValueError("server hỏng giữa chừng")

        def client(env):
            r = yield Call(env["ep"], Message(1))
            yield DebugPrint(r.err.name)

        m = {"objects": {"ep": "endpoint", "fault": "endpoint"}, "tasks": [
            {"name": "supervisor", "image": "supervisor", "priority": 250, "env": {"max_restarts": 1},
             "caps": [{"name": "fault_ep", "slot": 1, "obj": "fault", "rights": "R"},
                      {"name": "tcb_flaky", "slot": 10, "tcb": "flaky"}]},
            {"name": "flaky", "image": "flaky", "priority": 100, "fault": {"obj": "fault", "badge": 10},
             "caps": [{"name": "ep", "slot": 1, "obj": "ep", "rights": "R"}]},
            {"name": "client", "image": "client", "priority": 90,
             "caps": [{"name": "ep", "slot": 1, "obj": "ep", "rights": "W"}]}]}
        k = boot(m, {"flaky": flaky, "client": client}).run(300)
        self.assertEqual(k.lines("client"), ["CANCELLED"])

    def test_fault_without_handler_stops_only_that_thread(self):
        def bad(env):
            r = yield Call(99, Message())
            yield DebugPrint(f"cptr sai: {r.err.name}")
            raise RuntimeError("panic")

        m = {"tasks": [{"name": "bad", "image": "bad", "priority": 100},
                       {"name": "good", "image": "good", "priority": 10}]}
        k = boot(m, {"bad": bad, "good": spin("G")}).run(100)
        self.assertEqual(k.find_tcb("bad")[1].state, ThreadState.FAULTED)
        self.assertEqual(k.lines("bad"), ["cptr sai: INVALID_CAP"])
        self.assertEqual(k.lines("good"), ["G0", "G1", "G2"])

    def test_dev_mode_halts_everything(self):
        m = load_manifest(os.path.join(MANIFESTS, "space.toml"))
        m["tasks"][0]["env"]["dev_mode"] = 1
        k = boot(m)
        k.schedule_irq(5, at=2, period=8)
        k.run(1500)
        self.assertTrue(any("DEV MODE" in x for x in k.lines("supervisor")))
        self.assertNotIn("restart", " ".join(k.lines("supervisor")))

    def test_trap_frame_abi(self):
        def prog(env):
            yield Call(99, Message())
            yield DebugPrint("x")
        k = boot({"tasks": [{"name": "p", "image": "p"}]}, {"p": prog})
        k.run(3)
        t = k.find_tcb("p")[1]
        self.assertEqual(t.regs.pc, 8)                    # 2 syscall, mỗi lần pc += 4
        self.assertEqual(t.regs.a[0], int(KError.OK))      # kết quả DebugPrint


# ======================================================================
class IrqTests(unittest.TestCase):
    def test_irq_masked_until_ack(self):
        def drv(env):
            r = yield Wait(env["n"])
            yield DebugPrint(f"ngắt 1, value={r.value}")
            for _ in range(5):
                yield Yield()
            yield DebugPrint("ack")
            yield IrqAck(env["irq"])
            r = yield Wait(env["n"])
            yield DebugPrint(f"ngắt 2, value={r.value}")

        m = {"objects": {"n": "notification"}, "tasks": [
            {"name": "drv", "image": "drv", "priority": 100, "caps": [
                {"name": "n", "slot": 1, "obj": "n", "rights": "R"},
                {"name": "irq", "slot": 2, "irq": 3, "notification": "n"}]}]}
        k = boot(m, {"drv": drv})
        k.schedule_irq(3, at=1)
        k.schedule_irq(3, at=3)
        k.schedule_irq(3, at=4)
        k.run(100)
        self.assertEqual(k.lines("drv"), ["ngắt 1, value=1", "ack", "ngắt 2, value=1"])


if __name__ == "__main__":
    unittest.main()
