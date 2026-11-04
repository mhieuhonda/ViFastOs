"""Chương trình user space mẫu (tương đương các file ELF trong image hệ thống).

Mỗi chương trình là generator nhận `env` (tên cap -> slot, cộng tham số).
Đây là code user, không phải kernel: Rust port thành các binary riêng dùng crate `vifast-user`.
"""
from __future__ import annotations

from .abi import (Call, CapMint, CNodeMint, DebugPrint, FrameMap, GetTime, IoMap, IrqAck,
                  Load, PageTableMap, Recv, Reply, Retype, Signal, Store, TcbConfigure,
                  TcbRestart, TcbResume, TcbSuspend, Wait, Yield)
from .ktypes import (LABEL_FAULT, LABEL_NOTIFICATION, PAGE_SIZE, FaultKind, KError, Message,
                     ObjType, Rights)

GET = 1
PRINT = 2


def pack_text(text: str) -> tuple:
    raw = text.encode()[:63]
    raw = bytes([len(raw)]) + raw
    raw += b"\0" * (-len(raw) % 8)
    return tuple(int.from_bytes(raw[i:i + 8], "little") for i in range(0, len(raw), 8))


def unpack_text(words: tuple) -> str:
    raw = b"".join(w.to_bytes(8, "little") for w in words)
    return raw[1:1 + raw[0]].decode(errors="replace") if raw else ""


def u64(v: int) -> bytes:
    return v.to_bytes(8, "little")


# ---------------- hệ thống chung ----------------
def supervisor(env):
    """Chính sách phục hồi nằm ở user space: restart có giới hạn, quá thì DEGRADED."""
    max_restarts = env.get("max_restarts", 3)
    window = env.get("window", 500)
    dev_mode = env.get("dev_mode", 0)
    names = {v: k[4:] for k, v in env.items() if k.startswith("tcb_")}
    history: dict[int, list[int]] = {}
    while True:
        r = yield Recv(env["fault_ep"])
        if not r.ok or r.msg.label != LABEL_FAULT:
            continue
        kind, slot = FaultKind(r.msg.data[0]), r.badge
        name = names.get(slot, f"slot {slot}")
        if kind == FaultKind.EXITED:
            yield DebugPrint(f"{name} đã kết thúc")
            continue
        if dev_mode:
            yield DebugPrint(f"DEV MODE: {name} lỗi {kind.name}, dừng toàn hệ thống")
            for s in names:
                yield TcbSuspend(s)
            return
        now = (yield GetTime()).value
        recent = [x for x in history.get(slot, []) if now - x < window]
        if len(recent) >= max_restarts:
            history[slot] = recent
            yield DebugPrint(f"DEGRADED: {name} lỗi quá {max_restarts} lần, không restart")
            continue
        recent.append(now)
        history[slot] = recent
        yield TcbRestart(slot)
        yield DebugPrint(f"restart {name} ({kind.name}) lần {len(recent)}")


# ---------------- profile space ----------------
def imu_driver(env):
    """Driver ở user space: ngắt đến qua bound notification, phục vụ request qua endpoint."""
    sample = 0
    while True:
        r = yield Recv(env["imu_ep"])
        if r.msg.label == LABEL_NOTIFICATION:
            sample += 1                       # giả lập đọc thanh ghi cảm biến
            yield IrqAck(env["imu_irq"])
        elif r.msg.label == GET:
            yield Reply(Message(GET, (sample,)))


def attitude_ctrl(env):
    iterations = env.get("iterations", 40)
    for i in range(iterations):
        r = yield Call(env["imu"], Message(GET))
        if not r.ok:
            yield DebugPrint(f"lỗi gọi IMU: {r.err.name}")
            continue
        sample = r.msg.data[0]
        yield Store(env["ram_base"], u64(sample))
        if i % 10 == 0:
            yield DebugPrint(f"vòng {i}: mẫu IMU = {sample}")
        yield Yield()
    yield DebugPrint("hoàn thành điều khiển tư thế")


def payload(env):
    """Tác vụ nặng, ưu tiên thấp, bị giới hạn ngân sách CPU, lỗi sau vài vòng."""
    for _ in range(env.get("crash_after", 6)):
        yield Load(env["ram_base"], 8)
    yield DebugPrint("xử lý ảnh... gặp lỗi")
    raise RuntimeError("lỗi giả lập trong payload")


def producer(env):
    for i in range(3):
        text = f"khung ảnh {i}".encode()
        yield Store(env["buf_addr"], len(text).to_bytes(2, "little") + text)
        yield Signal(env["data_ready"])
        yield Wait(env["data_ack"])


def consumer(env):
    for _ in range(3):
        yield Wait(env["data_ready"])
        n = int.from_bytes((yield Load(env["buf_addr"], 2)).data, "little")
        body = (yield Load(env["buf_addr"] + 2, n)).data
        yield DebugPrint(f"nhận qua bộ nhớ chia sẻ: {body.decode()}")
        yield Signal(env["data_ack"])


# ---------------- driver thiết bị DMA ----------------
def cam_driver(env):
    """Driver camera: cấp vùng đệm cho thiết bị qua IOMMU rồi lập trình thanh ghi MMIO."""
    regs, buf, iova = env["cam_regs_addr"], env["dmabuf_addr"], 0x1000
    r = yield IoMap(env["dmabuf"], env["cam_io"], iova, Rights.READ | Rights.WRITE)
    if not r.ok:
        yield DebugPrint(f"IoMap lỗi: {r.err.name}")
        return
    for _ in range(env.get("frames", 3)):
        yield Store(regs + 0x00, u64(iova))
        yield Store(regs + 0x08, u64(16))
        yield Store(regs + 0x10, u64(1))
        yield Wait(env["cam_ntfn"])
        status = int.from_bytes((yield Load(regs + 0x18, 8)).data, "little")
        data = (yield Load(buf, 16)).data.rstrip(b"\0").decode()
        yield DebugPrint(f"DMA xong (status={status}): {data}")
        yield IrqAck(env["cam_irq"])


def dma_attacker(env):
    """Driver độc hại: ra lệnh cho thiết bị ghi DMA thẳng vào địa chỉ vật lý của kernel."""
    regs = env["cam_regs_addr"]
    yield Store(regs + 0x00, u64(env["target"]))
    yield Store(regs + 0x08, u64(16))
    yield Store(regs + 0x10, u64(1))
    yield Wait(env["cam_ntfn"])
    status = int.from_bytes((yield Load(regs + 0x18, 8)).data, "little")
    yield DebugPrint(f"status={status}")


# ---------------- profile desktop ----------------
def console(env):
    while True:
        r = yield Recv(env["console_ep"])
        yield DebugPrint(f"[badge {r.badge}] {unpack_text(r.msg.data)}")
        yield Reply(Message(0))


def hello(env):
    for i in range(3):
        r = yield Call(env["console"], Message(PRINT, pack_text(f"xin chào lần {i}")))
        if not r.ok:
            yield DebugPrint(f"console lỗi: {r.err.name}")


def init(env):
    """Tạo tiến trình mới lúc chạy: TCB + CNode (có guard) + VSpace + page table + 1 trang RAM."""
    S_TCB, S_CN, S_VS, S_FR, S_PT1, S_PT2, S_CNG = 20, 21, 22, 23, 24, 25, 26
    for slot, typ, bits in ((S_TCB, ObjType.TCB, 0), (S_CN, ObjType.CNODE, 4),
                            (S_VS, ObjType.VSPACE, 0), (S_FR, ObjType.FRAME, 0),
                            (S_PT1, ObjType.PAGE_TABLE, 0), (S_PT2, ObjType.PAGE_TABLE, 0)):
        r = yield Retype(env["mem"], typ, bits, slot)
        if not r.ok:
            yield DebugPrint(f"retype lỗi: {r.err.name}")
            return
    yield CNodeMint(S_CN, S_CNG, guard=0, guard_bits=64 - 4)   # CSpace 1 cấp, radix 4
    yield CapMint(env["console"], dest_index=1, rights=Rights.WRITE, badge=7, dest_cnode=S_CN)
    yield PageTableMap(S_PT1, S_VS, 0x10_0000)
    yield PageTableMap(S_PT2, S_VS, 0x10_0000)
    r = yield FrameMap(S_FR, S_VS, 0x10_0000)
    if not r.ok:
        yield DebugPrint(f"map lỗi: {r.err.name}")
        return
    r = yield TcbConfigure(S_TCB, cspace=S_CNG, vspace=S_VS, priority=50, mcp=50,
                           image="hello", env={"console": 1, "ram_base": 0x10_0000})
    if not r.ok:
        yield DebugPrint(f"configure lỗi: {r.err.name}")
        return
    yield TcbResume(S_TCB)
    yield DebugPrint("đã tạo tiến trình hello lúc chạy")


def pager(env):
    """Pager ở user space: nhận VM fault, cấp trang (và page table nếu thiếu) rồi reply."""
    next_slot, pt_slot = 30, 50
    while True:
        r = yield Recv(env["pager_ep"])
        kind, addr = FaultKind(r.msg.data[0]), r.msg.data[1]
        if kind != FaultKind.VM_FAULT:
            yield DebugPrint(f"bỏ qua fault {kind.name}")
            continue
        page = addr & ~(PAGE_SIZE - 1)
        rr = yield Retype(env["mem"], ObjType.FRAME, 0, next_slot)
        if not rr.ok:
            yield DebugPrint(f"hết bộ nhớ: {rr.err.name}")
            continue
        while True:
            m = yield FrameMap(next_slot, env["lazy_vs"], page)
            if m.err != KError.MISSING_PAGE_TABLE:
                break
            yield Retype(env["mem"], ObjType.PAGE_TABLE, 0, pt_slot)
            yield PageTableMap(pt_slot, env["lazy_vs"], page)
            pt_slot += 1
        next_slot += 1
        yield DebugPrint(f"cấp trang {page:#x} theo yêu cầu")
        yield Reply(Message(0))


def lazy(env):
    base = 0x4000_0000
    for i in range(3):
        yield Store(base + i * PAGE_SIZE + 8, f"trang {i}".encode())
    data = (yield Load(base + 8, 7)).data
    yield DebugPrint(f"đọc lại: {data.decode()}")


IMAGES = {f.__name__: f for f in (supervisor, imu_driver, attitude_ctrl, payload, producer,
                                   consumer, cam_driver, dma_attacker, console, hello, init,
                                   pager, lazy)}
