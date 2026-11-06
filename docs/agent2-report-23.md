# ViFastOs — Báo cáo vòng 23

**Ngày:** 2026-11-06  
**Baseline:** `c86de49` (vòng 22.2)  
**Chế độ:** chạy liền mạch theo quyết định agent0 (`ViFastOs-agent0-decision-23`).
**Trạng thái:** baseline xanh; 51/51 syscall; QEMU 3/3 × 3 kiến trúc.

## 0. Tóm tắt

1. Pha A hoàn tất: 0 warning, docs testing/crate map, wording 6 USER FAULT,
   summary có nhãn HISTORICAL, SHA + bundle 21/22.
2. Pha B hoàn tất: `ObjectDestroy` (syscall 50) + reclaim Untyped
   (watermark + free list 16 + `leaked`) + demo `DESTROY OK` + harness.
3. RISC-V boot stack giảm 256→192 KiB để giữ headroom sau khi thêm code
   (261.456 byte, trước vòng 200.016).
4. Pha C: C1 `-smp 4`, C2 soak `--repeat 10`, C3 thiết kế pager v2, C4 khảo
   sát `static mut`; kết quả ở mục 6.

## 1. Bảng task

| ID | Trạng thái | Commit | Ghi chú |
|---|---|---|---|
| A1 | DONE | `21063cf` | 4 warning `function_casts_as_integer` → 0 warning |
| A2 | DONE | `f39f817` | viết lại `docs/11-hien-tai/09-testing.md` |
| A3 | DONE | `f39f817` | thêm crate còn thiếu vào `00-index.md` |
| A4 | DONE | `cfc7b50` | NUMERIC mô tả 5→6 USER FAULT, `accept()` giữ 6 |
| A5 | DONE | `f09dae8` | HISTORICAL cho mục lịch sử, sửa số liệu hiện tại |
| A6 | DONE | `36a4e34` | SHA log + bundle 21, 22 |
| B1 | DONE | `ba2e753` | ABI + kernel `ObjectDestroy` + 10 host test |
| B2 | DONE | `d499677` | reclaim Untyped + 5 host test |
| B3 | DONE | `4584536` | demo root + harness `DESTROY OK`/`DESTROY BAD`; sửa `check-all.sh` |
| B4 | DONE | (commit docs cuối) | docs/00, ABI, object docs, report, bundle 23 |
| C1 | DONE | — | `-smp 4` chạy thủ công 3 kiến trúc |
| C2 | DONE | — | soak `--repeat 10` 3 kiến trúc |
| C3 | DONE | (commit docs cuối) | `docs/design/pager-v2.md` |
| C4 | DONE | — | khảo sát `static mut` AArch64 (mục 6.4) |

## 2. Số liệu

| Hạng mục | Giá trị |
|---|---|
| Host test `vifast-kernel` | **127/127** (+1 fuzz `--ignored`) |
| `vifast-arch` / `pte` / `elf` / `acpi` / `fs` | 7 / 11 / 17 / 8 / 22 |
| Tổng host test | **192** |
| Harness âm | **435 case** |
| QEMU `--repeat 3` | riscv64 3/3, aarch64 3/3, x86_64 3/3 |
| Persistence | 7 phase × 3 PASS |
| Warning | **0** |
| `Kernel` / `Object` / `Tcb` / `Cap` | 710.248 / 4.752 / 4.696 / 64 byte |
| RISC-V headroom | **261.456 byte** (`kernel_end=0x803d02b0`) |
| Object table | 147 object, 18 TCB, 96/64 cap, 64 mapping, 8 fd |
| `SYSCALL_STATUS` | 51/51 implemented, 0 placeholder |
| Root ELF stripped | RISC-V 22.960 B, AArch64 26.784 B, x86 29.440 B |

## 3. Output QEMU rút gọn (RISC-V)

```text
BOOT UNTYPED pages=24/32 objects=24/32 free_pages=8 free_objects=8
DATA UNTYPED pages=20/76 objects=20/76
VIRTIO OK base=0x10008000 magic=0x74726976 version=2 device=2
entering user mode at 0x1000000000
ROOT RUST
BOOTINFO OK
IOMMU OK
RUST MODULES OK
CLIENT VSPACE OK
SCHED CTX OK
DESTROY OK (frames=100, vspaces=10, reused=100, leaked=0)
ROOT READY
...
VFS SERVER OK
VFS OK
VFS STAT OK
IRQ OK
shell> PREEMPT OK (ticks=..., switches=...)
SMP IDLE OK (...)
SMP TIMER OK (hart1_ticks=...)
SMP SCHED OK (hart0=..., hart1=..., ...)
TLB STATS (va_flush=..., ipi=..., skipped=...)
ROOT SWITCH 1...
DATA UNTYPED pages=74/76 objects=70/76
```

## 4. Quyết định thiết kế khi đặc tả mơ hồ

1. **Cap gốc** xác định bằng `cap.parent == object.creator` và `cap.root ==
   false`; người giữ bản copy/mint nhận `NoRights`. Không dùng quy ước
   "slot == object.creator" vì `creator` là cap Untyped, không phải cap object.
2. **Hai mã lỗi mới** thay vì dùng `IllegalOperation` chung: `Busy = 20`
   (object đang dùng) và `HasDependents = 21` (còn cap khác). Thêm vào cuối
   enum, không đổi số cũ.
3. **Free list 16 entry** mỗi Untyped; hết chỗ thì vẫn destroy nhưng cộng
   `leaked` (đúng tinh thần đặc tả), invariant kiểm free list không chồng.
4. **Watermark + coalesce**: vùng sát `bump` hạ watermark và gộp tiếp; vùng
   khác gộp hai phía rồi mới chèn.
5. **Zero khi retype** giữ nguyên đường cũ (`zero_frame` cho
   Frame/PageTable/VSpace); test `reused_frame_is_zeroed_and_keeps_pa`.
6. **Demo đặt trước `spawn_idle`** thay vì trước `ROOT_READY`: tránh 200
   map/unmap sinh IPI TLB khi hart phụ đã load root VSpace (đây là nguyên
   nhân flaky/timeout ở lần chạy đầu). VA demo `+0x3f_f000` nằm trong span
   2 MiB của root ELF/stack nên không tạo page table mới.
7. **Giảm boot stack RISC-V 256→192 KiB**: code mới làm headroom còn 195.920
   (dưới guard 192 KiB). Boot stack chỉ BSP dùng, trap stack nằm trong
   `KERNEL_STORAGE`; sau khi giảm, headroom 261.456. Đây là thay đổi có chủ
   đích, đã ghi docs.
8. **SlotAllocator có free list nhỏ**: demo trả 2 slot sau khi destroy để
   worker E (spawn muộn trên x86) không bị "root slot range exhausted".

## 5. Task bị kẹt

Không có task BLOCKED. Hai lỗi tự gây và đã sửa:

- `vspace_in_use` ban đầu kiểm mapping của **mọi** VSpace thay vì VSpace đích
  → mọi ObjectDestroy VSpace trả `Busy`. Sửa và có test demo.
- x86 hết slot cspace sau khi demo → thêm free list cho SlotAllocator.

## 6. Pha C

### 6.1 C1 — `-smp 4` (không sửa harness)

Chạy bằng bản copy tạm của `test-qemu.py` chỉ đổi `-smp 2` → `-smp 4`
(không sửa harness, không commit script).

- **RISC-V: PASS.** Boot đầy đủ, `SMP OK (harts=2, ipi=1)`, `DESTROY OK`,
  `shell>`. Platform chỉ start hart 1 qua SBI HSM; hart 2/3 vẫn ở firmware.
  Harness vẫn xanh vì hợp đồng đang là 2 hart.
- **AArch64: PASS.** `SMP OK (harts=2, psci=0, ipi=1)`, đầy đủ demo; PSCI chỉ
  start CPU1, CPU2/3 offline.
- **x86_64: FAIL (dự kiến).** OVMF boot tới ACPI MADT (cpus=4) nhưng khi AP
  thứ 2/3 vào trampoline thì kernel báo `AP EXCEPTION`/`KERNEL EXCEPTION` và
  QEMU thoát mã 7. Hiện chỉ có trampoline INIT-SIPI-SIPI cho **một** AP;
  multi-AP cần thêm bảng/trampoline riêng.

Kết luận: `-smp 4` không hồi quy trên RISC-V/AArch64; x86 cần việc riêng
(multi-AP trampoline + LAPIC/GS/IST cho từng AP) trước khi đưa vào harness.

### 6.2 C2 — soak `--repeat 10`

`python3 scripts/test-qemu.py --repeat 10` (2 hart, cấu hình chuẩn):

```text
riscv64  10/10 PASS
aarch64  10/10 PASS
x86_64   10/10 PASS
real 2m20.760s
```

0 flaky, 0 timeout. Sau soak đã chạy lại `--repeat 3` + `gen-docs.py` để docs
khớp (nằm trong cổng `check-all.sh` cuối).

### 6.3 C3 — pager protocol v2

Đã viết `docs/design/pager-v2.md`: message MAP/UNMAP/EVICT/FAULT_REPLY, cap
VSpace/PageTable/TCB, luồng fault từ file qua VFS, evict = unmap +
ObjectDestroy, test plan 4 client và `PAGER3 OK`.

### 6.4 C4 — `static mut` ở platform AArch64

| Biến | Ai ghi | Khi nào | Bảo vệ hiện tại |
|---|---|---|---|
| `L0/L1/L2_LOW/L2_KERNEL` | BSP | `init_platform` (trước AP) | bất biến sau boot; MMU đọc |
| `USER_DATA` (304 KiB) | kernel qua identity map | FrameWrite/zero/copy | BKL cho đường kernel; user map đọc/ghi qua MMU |
| `METADATA` (80 KiB) | kernel | retype/zero | BKL |
| `BOOT_POOL` (128 KiB) | runtime | boot handoff | chỉ boot BSP |
| `CONTEXTS[128]` | mỗi hart | trap entry/exit, switch | mỗi TCB chỉ chạy 1 hart; per-context single writer |
| `TIMER_INTERVAL` | BSP | `init_timer` một lần | bất biến sau boot; cả 2 hart đọc |
| `TRAP_STACKS[hart]` | hart đó | `init_cpu` | per-hart độc quyền |
| `CPU_STATES[hart]` | hart đó | `init_cpu`, `publish_current_tcb`, `switch_root` | per-hart; địa chỉ trong TPIDR_EL1 |
| `KERNEL_PTR` | BSP | boot | bất biến sau boot |
| `KERNEL_STORAGE` | mọi hart | kernel entry | BKL (trừ GetTime fast path đọc atomic tick) |
| `SEMIHOST_EXIT_BLOCK` | BSP | shutdown | một lần |

Kết luận: hiện không có data race **nếu** các bất biến trên giữ; rủi ro chính
là `CONTEXTS` nếu một TCB bị chạy đồng thời 2 hart (scheduler hiện cấm) và
`TIMER_INTERVAL` nếu timer được cấu hình lại runtime. Đề xuất vòng sau thay
`static mut` bằng `UnsafeCell` wrapper có safety comment + `AtomicU64` cho
`TIMER_INTERVAL`; không cần khóa mới.

## 7. Sai lệch docs/code mới phát hiện

- `docs/11-hien-tai/02-objects-capabilities.md` trước vòng 23 ghi
  `MAX_OBJECTS=128`, `MAX_TCBS=16`, retype Tcb 1024 byte — đã sửa theo code.
- `docs/11-hien-tai/01-abi.md` bảng syscall cũ chỉ liệt kê một phần và ghi
  "27–31 chưa triển khai" — đã thay bằng bảng đủ 0..50.
- Summary mục 15 là changelog lịch sử; đã gắn nhãn HISTORICAL (A5).
- Số liệu root ELF trong summary/status từng cũ (12.880/12.736/16.744) — đã
  cập nhật theo build mới.
- `check-all.sh` chạy `check-docs.py` sau QEMU nhưng không `gen-docs.py` trước,
  nên lần chạy đầu báo stale dù mọi test xanh. Đã thêm `gen-docs.py` vào cổng
  (commit B3).

## 8. Câu hỏi cho agent0

1. Chấp nhận giảm boot stack RISC-V 256→192 KiB làm giải pháp headroom dài
   hạn, hay muốn giữ 256 KiB và cắt code khác?
2. `ObjectDestroy` có nên mở cho `Irq`/`Iommu`/`BlockDevice` ở vòng 24 không?
3. Pager v2 nên cấp PageTable cap cho pager hay thêm flag để kernel tự tạo
   bảng lá?
4. Có cần đưa `-smp 4` vào harness (kèm `cpus=4`) ngay vòng 24, hay để sau
   khi x2APIC/khóa mịn xong?
5. Bundle vòng 23 có cần kèm `docs/design/pager-v2.md` không?

## 9. Commit cuối

```text
$ git log --oneline c86de49..HEAD
4584536 feat: add object destroy demo and harness markers
d499677 feat: reclaim untyped memory on destroy
ba2e753 feat: add ObjectDestroy syscall
2d7db0e chore: reduce riscv boot stack to restore headroom
9519051 docs: refresh agent handoff for round 22
36a4e34 docs: sync log sha and add bundles 21-22
f09dae8 docs: mark historical summary sections
cfc7b50 docs: fix user fault count wording
f39f817 docs: refresh testing guide and crate map
21063cf fix: remove fn-to-int cast warnings in root
```

Cổng cuối: `./scripts/check-all.sh` → **ALL CHECKS PASS** (QEMU 3/3 ×3,
persistence 7×3, harness 435, docs). Bundle: `docs/bundles/ViFastOs-round-23.md`.
