# ViFastOs — Báo cáo Agent 2, vòng 02

**Ngày:** 2026-11-05  
**Người thực hiện:** Agent 2  
**Tệp quyết định:** `ViFastOs-decision-agent2-02.md`  
**Baseline đầu vòng:** `4293406` (code), `ed0fe08` (báo cáo vòng 01)  
**Code cuối vòng:** `4d44b60` (`refactor: per-cpu trap stacks and current tcb`)  
**Trạng thái:** tất cả task xong; repo sạch sau commit docs cuối.

---

## 1. Tổng hợp task

| Task | Nội dung | Trạng thái | Commit |
|---|---|---|---|
| 1 | RISC-V VirtIO-MMIO block | ✅ Xong | `23d8dfb` |
| 2 | Đồng bộ summary + handoff | ✅ Xong | `f5d3256` |
| 3 | Audit/hardening BKL | ✅ Xong | `b51782b` |
| 4 | Per-CPU trap stacks/state | ✅ Xong | `4d44b60` |

---

## 2. Test cuối vòng

```text
./scripts/build.sh                 PASS, không warning
vifast-kernel                      83/83 PASS
vifast-arch                         2/2 PASS
vifast-pte                         11/11 PASS
vifast-elf                         17/17 PASS
cargo fmt --all -- --check          PASS
python3 scripts/test-qemu.py        3/3 PASS
  lặp 3 lần                         3/3 PASS cả 3 lần
python3 scripts/test-persist.py     RISC-V/AArch64/x86_64 PASS
```

QEMU cuối vòng:

```text
RISC-V : SMP OK (harts=2, ipi=1), BLOCK OK, FS DISK OK
AArch64: SMP OK (harts=2, psci=0, ipi=1), BLOCK OK, FS DISK OK
x86_64 : SMP SKIP, BLOCK OK, FS DISK OK
```

Persistence test kiểm tra đủ 3 phase cho cả 3 kiến trúc: disk trắng, nonce
`VF01`, và negative magic sai.

---

## 3. Task 1 — RISC-V VirtIO-MMIO block

Commit:

```text
23d8dfb feat: riscv virtio-mmio block backend
```

- OpenSBI Region07:

```text
Domain0 Region07 : 0x0-0xffffffffffffffff M:() S/U:(R,W,X)
```

  Không có PMP chặn. Giả thuyết PMP trong handoff cũ là sai.

- QEMU `virt` có 8 slot VirtIO-MMIO, mỗi slot cách `0x1000`:
  `0x10001000`, `0x10002000`, ..., `0x10008000`.
- `virtio-blk-device` nằm ở slot cuối **`0x10008000`**; với
  `-global virtio-mmio.force-legacy=false` đọc được:

```text
magic=0x74726976 version=2 device=2
```

- `vifast-virtio` có `init_range(start, end, stride)`; AArch64 giữ cửa sổ cũ
  `0x0a000000..0x0a004000`, stride `0x200`; RISC-V dùng
  `0x10001000..0x10009000`, stride `0x1000`.
- Legacy version 1 chỉ log `BLOCK SKIP (virtio legacy)`.
- RISC-V platform kiểm tra DMA range nằm trong `kernel_accessible_range()`;
  queue là static BSS identity-mapped.
- Harness thêm disk temp + `virtio-blk-device` + `force-legacy=false`;
  `require_block/require_disk=True`.
- `test-persist.py` bỏ skip RISC-V; cả 3 kiến trúc đều 3 phase PASS.

### Kích thước RISC-V

```text
kernel_end = 0x8038f0b0
limit      = 0x80400000
headroom   = 462672 byte
```

Việc thêm per-CPU stacks ở Task 4 làm headroom giảm từ `561072` xuống
`462672` byte; vẫn dưới giới hạn 4 MiB.

### Nợ

- Địa chỉ thiết bị vẫn scan cứng theo cửa sổ QEMU, chưa đọc DTB.
- Driver polling, một request/lần; chưa IRQ/queue đa request.

---

## 4. Task 2 — Đồng bộ tài liệu

Commit:

```text
f5d3256 docs: sync summary and handoff after riscv block
```

Đã sửa:

- Summary: commit list/latest, RISC-V block/FS, bỏ PMP, host test `vifast-arch`
  2/2, BKL/per-CPU, `test-persist.py`, headroom mới.
- Handoff: baseline mới, bảng tính năng RISC-V block/FS ✅, BKL/per-CPU ✅,
  xóa hướng A/B và giả thuyết PMP, sửa ghi chú x86 I-cache (I/D coherent,
  chỉ cần serializing; không `wbinvd`/clflush), thêm `test-persist.py` vào
  harness/DoD.
- SHA-256 summary cập nhật trong `docs/overnight-log.md`.

---

## 5. Task 3 — Audit và làm cứng BKL

Commit:

```text
b51782b fix: make bkl cover all kernel entries and assert release on user return
```

### 5.1 Bảng đường vào/ra kernel

| Kiến trúc | Đường vào | BKL | Đường ra / nhả |
|---|---|---|---|
| RISC-V | boot `rust_main` | Không | `enter_user` assert BKL free |
| RISC-V | timer trap | Có | handler `drop` + assert trước `sret` |
| RISC-V | syscall trap | Có | handler `drop` + assert trước `sret` |
| RISC-V | user fault | Có | `fault::suspend`; `drop` + assert trước `sret` |
| RISC-V | kernel trap | Không | `riscv_kernel_trap` exit, không return |
| RISC-V | IPI maintenance hart 1 | **Không** | loop maintenance; comment lý do |
| AArch64 | boot | Không | `enter_user` assert BKL free |
| AArch64 | sync lower syscall | Có | `drop` + assert trước `eret` |
| AArch64 | sync lower fault | Có | `drop` + assert trước `eret` |
| AArch64 | IRQ | Có | `drop` + assert trước `eret` |
| AArch64 | kernel fault vector | Không | `aarch64_kernel_fault` exit |
| AArch64 | IPI maintenance hart 1 | **Không** | loop maintenance |
| x86_64 | boot | Không | `enter_user` assert BKL free |
| x86_64 | int80 syscall | Có | `drop` + assert trước `iretq` |
| x86_64 | IRQ0 timer | Có | `drop` + assert trước `iretq` |
| x86_64 | exception page/GP/... | Có | fault suspend → `drop` + assert trước `enter_user` |
| x86_64 | page fault | Có | fault suspend → `drop` + assert trước `enter_user` |
| x86_64 | fatal kernel exception | Có | exit, không return |
| x86_64 | AP/IPI | Chưa có AP | — |

### 5.2 Cách xử lý x86 fault path

- `x86_exception_handler` và `x86_page_fault` đều lấy `BKL.lock()`.
- Sau `fault::suspend`, gọi `drop(bkl)` rồi assert `!BKL.is_held()` trước
  `enter_user`. Không dùng guard xuyên qua `enter_user` vì `enter_user` không
  bao giờ return.
- `enter_user` của cả 3 kiến trúc có assert BKL đã nhả.

### 5.3 IRQ khi giữ BKL

Mỗi trap entry assert IRQ đã tắt ngay sau khi lấy BKL:

- RISC-V: `sstatus.SIE == 0`.
- AArch64: `DAIF.I == 1`.
- x86_64: `RFLAGS.IF == 0`.

QEMU 3/3 PASS 3 lần chứng minh các assert không kích hoạt trên các đường
timer/syscall/fault hiện tại.

### 5.4 IPI path

- `rust_secondary` RISC-V và AArch64 **không** lấy BKL.
- Comment trong code nêu rõ: hart 0 giữ BKL khi chờ ACK TLB shootdown; nếu hart
  phụ lấy BKL sẽ deadlock.
- x86 chưa có AP/IPI nên không áp dụng.

### 5.5 Ordering

`BigKernelLock::try_lock` dùng `compare_exchange` với `Ordering::Acquire`;
`drop` dùng `Ordering::Release`. Chưa chuyển ticket lock; ghi nợ.

---

## 6. Task 4 — Per-CPU trap stacks và state

Commit:

```text
4d44b60 refactor: per-cpu trap stacks and current tcb
```

### 6.1 Đã chuyển thành mảng per-CPU

| Kiến trúc | Biến cũ | Mới |
|---|---|---|
| RISC-V | `TRAP_STACK` | `TRAP_STACKS[4]` |
| RISC-V | — | `CPU_STATES[4]`, truy cập qua `tp` |
| AArch64 | `TRAP_STACK` | `TRAP_STACKS[4]` |
| AArch64 | — | `CPU_STATES[4]`, truy cập qua `TPIDR_EL1` |
| x86_64 | `TRAP_STACK` | `TRAP_STACKS[4]` |
| x86_64 | `IST_STACK` | `IST_STACKS[4]` |
| x86_64 | `TSS` | `TSSS[4]`, GDT trỏ `TSSS[0]` |
| x86_64 | — | `CPU_STATES[4]`, truy cập qua `GS_BASE` |

`CpuState` có:

```rust
hart_id: usize
current_tcb: usize
trap_stack_top: u64
```

- RISC-V: `init_cpu` ghi `tp` trỏ CPU state; `enter_user` đọc trap stack top
  từ `tp`.
- AArch64: `init_cpu` ghi `TPIDR_EL1`; `enter_user` đọc từ đó.
- x86_64: `init_cpu` ghi `IA32_GS_BASE`; `enter_user` đọc `gs:[0]/gs:[16]`.
- Phát hiện và sửa lỗi: `x86_reload_segments` nạp GS selector base 0, làm mất
  `GS_BASE`; `prepare_user` gọi lại `init_cpu(0)` sau reload.
- Hart 0 chạy như cũ; hart 1 vẫn maintenance, chưa vào scheduler.
- QEMU output không đổi.

### 6.2 Biến còn global

- Page table/boot-only: `PML4/PDPT/PD`, `ROOT/KERNEL_L1/DEVICE_L1`, L0/L1/L2,
  GDT/IDT, split tables.
- Kernel state dưới BKL: `KERNEL_PTR`, `KERNEL_STORAGE`, `ACTIVE_ROOT`,
  `ACTIVE_ASID`, `CONTEXTS`, arena pools.
- Server: `FILES`, `FILE_COUNT`, `DISK_OK`, `PERSISTED`.
- VirtIO: `QUEUE`, `BASE`, `READY`, `LAST_*`.
- `current_tcb` trong `CpuState` hiện chưa được cập nhật theo scheduler; để
  dành cho vòng full SMP.

---

## 7. Nợ mới / còn lại

- BKL vẫn test-and-set, chưa fair/ticket; chưa có scheduler đa hart.
- `current_tcb` per-CPU chưa được cập nhật khi switch task.
- x86 AP/MADT/SIPI chưa làm; x86 IPI path chưa áp dụng.
- Server/VirtIO static mut chưa có khóa riêng cho SMP.
- RISC-V VirtIO scan cứng, chưa đọc DTB.
- FS v2, per-VA TLB, remote I-cache, hardware IOMMU, net/input/compositor
  vẫn ngoài vòng này.

---

## 8. SHA-256 và trạng thái cuối

```text
ViFastOs-architecture-summary.txt:
69ffa2fc99d70c44730d7be98bd3ed5a69d239bf32a2a287a7379a5d2f1e3268
```

Code baseline cuối:

```text
4d44b60 refactor: per-cpu trap stacks and current tcb
```

Docs audit cuối được commit sau code (không đổi code baseline).
