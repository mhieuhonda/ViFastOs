# ViFastOs — Báo cáo Agent 2, vòng 03

**Ngày:** 2026-11-05  
**Người thực hiện:** Agent 2  
**Tệp quyết định:** `ViFastOs-decision-agent2-03.md`  
**Baseline đầu vòng:** `4d44b60` (code), `541cc4f` (báo cáo vòng 02)  
**Code cuối vòng:** `74b7d75` (`feat: per-cpu security and per-hart current groundwork`)  
**Trạng thái:** Task 1–4 xong; **Task 5 chưa làm**; Task 6 docs đã cập nhật.

> Ghi chú lệch quy trình: Task 1–4 có phụ thuộc cấu trúc vào cùng trap entry,
> BKL và per-CPU state nên được gộp trong một commit tích hợp `74b7d75`
> thay vì bốn commit riêng. Từng phần đều có test riêng và baseline vẫn xanh.

---

## 1. Tổng hợp task

| Task | Nội dung | Trạng thái | Commit |
|---|---|---|---|
| 1 | Per-CPU an toàn | ✅ | `74b7d75` (gộp) |
| 2 | Ticket lock + maintenance poll | ✅ | `74b7d75` (gộp) |
| 3 | Per-hart current trong kernel | ✅ | `74b7d75` (gộp) |
| 4 | RISC-V remote I-cache | ✅ | `74b7d75` (gộp) |
| 5 | Hart phụ chạy scheduler | ❌ Chưa làm | — |
| 6 | Tài liệu | ✅ | docs cuối vòng |

---

## 2. Test cuối vòng

```text
./scripts/build.sh                 PASS, không warning
vifast-kernel                      84/84 PASS
vifast-arch                         4/4 PASS
vifast-pte                         11/11 PASS
vifast-elf                         17/17 PASS
cargo fmt --all -- --check          PASS
python3 scripts/test-qemu.py        3/3 PASS
  lặp 3 lần                         3/3 PASS cả 3 lần
python3 scripts/test-persist.py     RISC-V/AArch64/x86_64 PASS
```

QEMU cuối:

```text
RISC-V : SMP OK (harts=2, ipi=1), BLOCK OK, FS DISK OK, PERCPU CLOBBER OK
AArch64: SMP OK (harts=2, psci=0, ipi=1), BLOCK OK, FS DISK OK, PERCPU CLOBBER OK
x86_64 : SMP SKIP, BLOCK OK, FS DISK OK, PERCPU CLOBBER OK
```

`PERCPU CLOBBER BAD` không xuất hiện; harness bắt buộc đúng 1
`PERCPU CLOBBER OK`, vẫn đúng 4 `USER FAULT`.

---

## 3. Task 1 — Per-CPU an toàn

### RISC-V

Trước đây `tp` giữ CpuState; user có thể ghi `tp`. Nay:

- User: `sscratch` giữ con trỏ CpuState; `tp` là giá trị user.
- Trap entry (trích `trap.S`):

```asm
csrrw tp, sscratch, tp      # tp = CpuState, sscratch = user tp
beqz tp, 9f                 # trap từ kernel
sd sp, 24(tp)               # CpuState.user_sp = user sp
ld sp, 16(tp)               # trap_stack_top
...
csrr t0, sscratch
sd t0, 32(sp)               # lưu user tp vào frame
mv s0, tp                   # giữ CpuState pointer qua C call
csrw sscratch, zero
...
csrw sscratch, s0           # kernel-only pointer cho trap sau
sret
```

- `s0` là callee-saved nên Rust handler giữ nguyên CpuState pointer.
- User clobber `tp = 0xdeadbeef...`, gọi `GetTime`, rồi kiểm tra `tp` được
  khôi phục đúng; in `PERCPU CLOBBER OK`.

### x86_64

- Dùng chuẩn `swapgs`:
  - user: `GS_BASE = 0`, `IA32_KERNEL_GS_BASE = CpuState`.
  - trap từ ring 3: `swapgs`; trước `iretq` về ring 3: `swapgs`.
  - exception/page-fault kiểm tra CS để chỉ swapgs khi từ user.
  - `enter_user` đọc CpuState qua GS trước khi `swapgs`.
- Không bật `CR4.FSGSBASE`.
- Phát hiện `x86_reload_segments` làm mất GS base; `prepare_user` gọi lại
  `init_cpu(0)` sau reload.
- User `mov gs, 0`, gọi syscall; kernel vẫn đúng và user thấy selector 0;
  in `PERCPU CLOBBER OK`.

### AArch64

- `TPIDR_EL1` là kernel-only; user chỉ có `TPIDR_EL0`.
- Test ghi rác `TPIDR_EL0`, gọi syscall; giá trị user được giữ; in
  `PERCPU CLOBBER OK`.
- Không cần đổi cơ chế trap stack.

### current_tcb và ACTIVE_ROOT/ASID

- `CpuState` mở rộng:
  - `current_tcb`
  - `active_root`
  - `active_asid`
  - `user_sp` (RISC-V)
- Runtime gọi `set_current_tcb` sau mỗi `pick`/switch.
- Trap entry assert `CpuState.current_tcb` khớp `kernel.current_on(hart)`.
- `ACTIVE_ROOT`/`ACTIVE_ASID` đã bỏ khỏi static global, nằm trong CpuState.

---

## 4. Task 2 — Ticket BKL + maintenance poll

- `BigKernelLock` đổi sang ticket lock:
  - `next: AtomicUsize`
  - `serving: AtomicUsize`
  - Acquire/Release ordering.
  - RAII `BigKernelGuard`; `is_held()`.
- `lock_with(poll)` gọi hook khi spin.
- Platform `poll_maintenance()`:
  - RISC-V/AArch64: nếu hart != 0 và có IPI pending thì xử lý
    `sfence.vma`/`fence.i` hoặc `dsb ish`/`isb`, rồi ACK.
  - x86: no-op (chưa có AP).
- Assert IRQ tắt được chuyển **trước** `BKL.lock_with`.
- Không bật IRQ trong lúc spin.
- Host test `vifast-arch`:
  - 2 test cũ (re-entry, release).
  - FIFO: 4 thread, tối đa 1 thread trong critical section.
  - Mô phỏng deadlock: holder giữ khóa, spinner `lock_with` set ACK;
    test có timeout 5s và PASS.

---

## 5. Task 3 — Per-hart current trong kernel

- `Kernel` có:

```rust
pub const MAX_HARTS: usize = 4;
current: [Option<ObjRef>; MAX_HARTS]
```

- API mới:
  - `current_on(hart)`, `pick_on(hart)`
  - `yield_current_on(hart)`, `fault_current_on(hart)`
  - wrapper `current()/pick()/...` vẫn giữ cho hart 0/test cũ.
- Runtime lấy `A::hart_id()` và truyền hart vào scheduler.
- Runqueue vẫn **một global** dưới BKL.
- `check_invariants()`:
  - mỗi current TCB phải `Running`, không queued;
  - không TCB nào là current trên 2 hart.
- Revoke/destroy TCB:
  - Nếu TCB là current trên **bất kỳ hart nào**, trả `IllegalOperation`.
  - Chưa có IPI kick; chọn từ chối để an toàn, ghi nợ.
- `pick_on(hart)` trả `None` là idle; runtime hiện vẫn exit/không bật hart phụ.
- Host test mới:
  - 2 hart pick 2 TCB khác nhau;
  - hart 2 idle;
  - destroy TCB đang current bị từ chối.
- QEMU output giữ nguyên; hart1 vẫn maintenance.

---

## 6. Task 4 — RISC-V remote I-cache

- `PageOps::sync_icache`:
  - `fence.i` local;
  - nếu đang ở hart 0 và hart phụ online, gọi `request_ipi()`.
- Secondary handler đã chạy `fence.i` + `sfence.vma` và ACK.
- AArch64: chuỗi hiện tại `dc cvau` + `dsb ish` + `ic ivau` + `dsb ish`
  broadcast inner-shareable; không cần sửa.
- x86: I/D coherent; không cần.

---

## 7. Task 5 — Hart phụ chạy scheduler

**Chưa làm trong vòng này.**

Lý do: cần thay đổi lớn đồng thời:
- timer per-hart và test-exit chỉ do hart 0;
- secondary entry vào scheduler, context switch và root switch per-hart;
- xử lý IPI phần mềm trên RISC-V trước BKL;
- đảm bảo output/serial và 10 lần QEMU ổn định.

Không có commit Task 5; không tuyên bố `SMP SCHED OK`. Đây là phần còn lại
lớn nhất trước khi có SMP scheduler thật.

---

## 8. Headroom và nợ

```text
RISC-V kernel_end = 0x80390110
headroom          = 458480 byte (0x6fef0)
```

Nợ mới/ còn lại:

- Task 5 chưa làm: hart phụ vẫn maintenance, chưa in `SMP SCHED OK`.
- `current_tcb` per-CPU đã cập nhật nhưng chưa có scheduler hart phụ.
- Revoke TCB đang chạy trên hart khác bị từ chối, chưa có IPI kick.
- BKL ticket đã fair nhưng vẫn global.
- x86 AP/MADT/SIPI vẫn chưa làm.
- Server/VirtIO static mut vẫn chưa có khóa riêng.
- FS v2, per-VA TLB, hardware IOMMU, net/input, compositor còn ngoài vòng.

---

## 9. SHA-256 và trạng thái cuối

```text
ViFastOs-architecture-summary.txt:
d0d671f9cbb3fd468ad9ff53ec0629deee22c730c7b9ebc34c73cf76ed637d64
```

Code baseline cuối:

```text
74b7d75 feat: per-cpu security and per-hart current groundwork
```

Docs cuối vòng được commit sau code; không đổi code baseline.
