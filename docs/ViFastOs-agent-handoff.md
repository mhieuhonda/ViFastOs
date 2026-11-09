# ViFastOs — Báo cáo bàn giao cho agent0

**Phiên bản:** 2026-11-06, cập nhật sau vòng 23 (ObjectDestroy)  
**Repo:** `/Users/mps/dvchd/ViFastOs` (nhánh `main`)  
**HEAD:** `c86de49` — `docs: document VFS stat-by-path`  
**Mốc code gần nhất:** `13c26f8` — VFS `STAT_PATH`  
**Vòng gần nhất:** 22.2 (xem `docs/agent2-report-22.md`)  
**Trạng thái baseline:** xanh — build PASS (còn 4 warning ở `vifast-root`,
xem 1.4), QEMU 3/3 × 3 kiến trúc (repeat 3), persistence 7 phase × 3 PASS,
docs check PASS, fmt PASS.  
**Mức trưởng thành:** microkernel bring-up/nghiên cứu/giáo dục; **không phải**
hệ điều hành production.

> **CẬP NHẬT 2026-11-07 (vòng 79) — đọc mục này trước, phần dưới là lịch sử**
>
> - HEAD hiện tại: `69fe809` (xem `git log --oneline`); báo cáo từng vòng ở
>   `docs/agent2-report-<N>.md`, nhật ký ở `docs/overnight-log.md`.
> - Đã có thêm từ vòng 23: VirtIO-MMIO **net** (ARP/ICMP/UDP/DNS A, TCP echo
>   một kết nối), **gpu** (160x120, gradient + font 5x7 "ViFastOs"),
>   **input** (bàn phím → RxQueue UART → shell), **rng** (entropy thật),
>   demand-load pager nạp `/page.bin` từ VFS trên riscv64/aarch64.
> - Tài liệu driver mới: `docs/11-hien-tai/13-virtio.md`.
> - Cổng hiện tại (chạy bằng một lệnh `./scripts/gate.sh`): build 0 warning,
>   QEMU 3/3 × 3 kiến trúc, persist 7 × 3, harness 504 case, fmt, docs.
>   Thêm `--with-tcp` để chạy thêm kiểm tra TCP echo qua hostfwd.
> - Chốt chặn đã biết: x86_64 không có VFS demand-load vì root ELF/stack map
>   bằng trang 2 MiB (cần tách PMD) và data/meta Untyped đã cạn 76/76.
> - Việc nên làm tiếp: giải phóng Untyped cho x86 (staging frame hoặc tách
>   huge page), đưa TCP thành service thường trú, mở rộng thời gian sống của
>   guest để test gõ lệnh đầy đủ.

> **Cách đọc tài liệu này**
>
> 1. Mục 0 là tóm tắt đủ để ra quyết định trong 3 phút.
> 2. Mục 1 là bằng chứng kiểm chứng ngày 2026-11-06; nếu số liệu khác, chạy
>    lại lệnh ở đó.
> 3. Mục 7–9 là việc cần làm; mục 8 là "hợp đồng" không được phá.
> 4. Code + `scripts/harness_spec.py` là nguồn sự thật cuối cùng. Khi tài liệu
>    và code lệch nhau, **đọc code rồi sửa tài liệu**, không sửa test cho dễ
>    pass.

---

## 0. Tóm tắt cực ngắn cho agent0

### Đã có (đã kiểm chứng trên QEMU)

- Microkernel Rust `no_std`; `vifast-kernel`, `vifast-runtime`, `vifast-fs`
  đều `#![forbid(unsafe_code)]`.
- Boot 3 kiến trúc: RISC-V 64 (OpenSBI), AArch64 (QEMU `virt`, GICv2),
  x86_64 (OVMF/UEFI, ACPI MADT + LAPIC).
- User mode 3 kiến trúc (`U-mode`/`EL0`/`ring3`), root/server/client/fault
  TCB nạp từ Rust ELF. Root, server, client có VSpace riêng; fault1–3 dùng
  VSpace root, fault4 dùng VSpace client; các TCB demo (idle, CNode worker,
  pager) chạy trong VSpace root.
- **51/51 syscall number implemented, 0 placeholder** (bảng `SYSCALL_STATUS`),
  gồm `ObjectDestroy` (50) + reclaim Untyped (free list 16 vùng, `leaked`).
- Object/capability/Untyped/retype, IPC Call/Reply/ReplyRecv, notification,
  chuyển capability qua IPC, badge, revoke theo cây.
- SMP 2 hart cả 3 kiến trúc: scheduler trên hart phụ, ready queue theo hart,
  work stealing, BKL ticket, maintenance IPI, per-VA TLB shootdown.
- Timer preemption; `Sleep` deadline (`BlockedSleep`); SchedContext
  budget/period (`BlockedBudget`).
- VFS server chạy user-space; FS v4 (`VF04`) journal hóa: 64 inode, thư mục,
  permission theo badge, hard link, symlink, single + double indirect,
  `O_APPEND`/`O_EXCL`/`O_DIRECTORY`, `STAT_PATH`, block cache write-back cho
  data, write-through metadata/journal, `fsync` → `BlockFlush`.
- Persistence 7 phase trên cả 3 kiến trúc (blank format, two-way persist,
  migration, journal replay, torn journal, corrupt superblock, file lớn).
- Pager: fault endpoint → user-space pager server map Frame + `TcbResume`,
  phục vụ **hai** client (`PAGER OK`, `PAGER2 OK`); `TcbDestroy`/reap TCB.
- CNode phẳng 64 slot, `CNodeMint`/`CNodeInstall`, demo end-to-end
  (`CNODE OK`).
- IRQ object: timer IRQ + UART IRQ, console input IRQ, shell nhỏ
  (`ls`, `cat`, `stat`, `mkdir`, `rm`, `mv`, `ln`, `sync`, `truncate`, `fsck`).
- Software IOMMU domain + `IoMap`/`IoUnmap` (chưa có phần cứng).

### Chưa có / còn yếu

- Hardware IOMMU (VT-d/SMMU); DMA hiện chưa dùng thật vì block driver polling.
- Compositor, ViFastKey, focus/window nhiều client (net + GPU + input MVP
  ĐÃ có từ vòng 66–180; chỉ thiếu phần compositor/ViFastKey).
- Pager/mount tổng quát; page cache dùng chung; demand-load ELF theo nhu cầu.
- CNode guard, nhiều cấp, zombie reaping.
- Free/destroy object theo từng object ngoài đường revoke Untyped; allocator
  vật lý trả trang; object table 168 (TCB 24) nhưng chưa có free-page
  allocator toàn cục (G2).
- Khóa mịn (mới đo contention), x2APIC; mới test 2 hart (`MAX_HARTS=4`).
- GDT/TSS x86 chưa per-hart ⇒ không bật được hart 2/3 (P1, audit92).
- Barrier/atomicity của đĩa thật; driver QEMU-specific.
- Userland đầy đủ: libc, shell hệ thống, dynamic linking, ứng dụng thật.
- Formal verification; soak dài; CI hosted (đã bỏ theo yêu cầu).

### Ba việc nên làm tiếp (đề xuất, xem mục 9)

1. **Object lifecycle + reclaim**: destroy từng Frame/Endpoint/PageTable/
   VSpace/Notification, trả trang về Untyped, test double-free/generation.
2. **Pager tổng quát + demand-load**: pager protocol v2, cấp PageTable cho
   pager, nạp trang từ file qua VFS, test `PAGER3 OK`.
3. **VirtIO net + stack tối thiểu**: transport dùng lại, ARP/IPv4/ICMP/UDP,
   DMA qua software IOMMU; sau đó hardware IOMMU mới có đất diễn.

### Ba điều cấm

1. Không làm yếu harness (`scripts/test-qemu.py` + `harness_spec.py`) để pass.
2. Không thêm `unsafe` vào `vifast-kernel` / `vifast-runtime` / `vifast-fs`.
3. Không commit khi QEMU đỏ, khi vượt 4 MiB cửa sổ kernel RISC-V, hoặc khi
   `./scripts/check-all.sh` chưa xanh.

---

## 1. Baseline và bằng chứng kiểm chứng (2026-11-06)

### 1.1 Môi trường

| Mục | Giá trị |
|---|---|
| OS | macOS (Darwin), QEMU 11.x, OVMF edk2 |
| Rust | `rustc 1.99.1`, `cargo 1.99.1` |
| Target | `riscv64imac-unknown-none-elf`, `riscv64gc-unknown-none-elf`, `aarch64-unknown-none-softfloat`, `x86_64-unknown-none`, `x86_64-unknown-uefi` |
| QEMU | `qemu-system-riscv64`, `qemu-system-aarch64`, `qemu-system-x86_64` |
| Lưu ý PATH | Trong shell không login, `cargo` **không** có trên PATH; dùng `export PATH="$HOME/.cargo/bin:$PATH"` |

### 1.2 Lệnh kiểm tra và kết quả đã chạy

```bash
cd /Users/mps/dvchd/ViFastOs
export PATH="$HOME/.cargo/bin:$PATH"

./scripts/build.sh                      # host test + root ELF + 3 kernel
python3 scripts/test-qemu.py --repeat 3 # 3 lần × 3 kiến trúc
python3 scripts/test-persist.py         # 7 phase × 3 kiến trúc
python3 scripts/test-harness-spec.py    # test âm cho harness
python3 scripts/check-docs.py           # sau khi gen-docs
cargo fmt --all -- --check
```

Kết quả ngày 2026-11-06:

| Hạng mục | Kết quả |
|---|---|
| Host test `vifast-kernel` | **112/112 PASS**, 1 test fuzz `--ignored` |
| Host test `vifast-arch` | 7/7 PASS |
| Host test `vifast-pte` | 11/11 PASS |
| Host test `vifast-elf` | 17/17 PASS |
| Host test `vifast-acpi` | 8/8 PASS |
| Host test `vifast-fs` | 22/22 PASS |
| `vifast-runtime`, `vifast-user`, `vifast-fsutil` | 0 unit test (kiểm qua QEMU/build) |
| Tổng host test duy nhất | **192** |
| Harness âm | **435 case PASS** |
| QEMU `--repeat 3` | riscv64 **3/3**, aarch64 **3/3**, x86_64 **3/3** |
| Exit code QEMU | RISC-V 0, AArch64 0, x86_64 **3** (`isa-debug-exit`) |
| Persistence | **7 phase × 3 kiến trúc PASS** (24 dòng PASS, 0 FAIL) |
| `cargo fmt --check` | PASS |
| `check-docs.py` | PASS (sau `gen-docs.py`) |
| Warning build | **4 warning** ở `vifast-root` (xem 1.4) |

### 1.3 Output QEMU rút gọn (RISC-V, mẫu thật)

Số đếm thay đổi theo thời điểm; các marker là hợp đồng.

```text
ViFastOs booting on riscv64 (hart=0, dtb=0x87e00000)
SELFTEST PASS (ipc_tx=3 ipc_rx=2 ntfn=2 ctx=5 ticks=0)
SMP OK (harts=2, ipi=1)
ROOT ELF entry=0x1000200000 segments=2 stack=0x1000305000
BOOT UNTYPED pages=24/32 objects=24/32 free_pages=8 free_objects=8
DATA UNTYPED pages=20/76 objects=20/76
VIRTIO OK base=0x10008000 magic=0x74726976 version=2 device=2
BLOCK OK
entering user mode at 0x1000000000
ROOT RUST
BOOTINFO OK
IOMMU OK
RUST MODULES OK
CLIENT VSPACE OK
SCHED CTX OK
ROOT READY
TLB REMAP OK (seen_new=1)
USER FAULT ... addr=0x80000000
FAULT EP OK addr=0x0000000080000000
...
USER FAULT ... addr=0x1001000000
USER FAULT ... addr=0x1001001000
PAGER OK
PAGER2 OK
CNODE OK
TCB REAP OK
...
VFS SERVER OK
VFS OK
VFS STAT OK
IRQ OK
shell> PREEMPT OK (ticks=..., switches=...)
SMP IDLE OK (...)
SMP TIMER OK (hart1_ticks=...)
SMP PREEMPT OK (user_ticks=..., preempts=...)
SMP SCHED OK (hart0=..., hart1=..., resched_sent=..., resched_wake=..., idle_entries=...)
SMP WORKERS OK (a=50, b=50, c=1)
TLB STATS (va_flush=..., ipi=..., skipped=...)
ROOT SWITCH 1891
DATA UNTYPED pages=69/76 objects=66/76
```

### 1.4 Sai lệch / nợ đã phát hiện khi kiểm chứng

1. **4 warning build** trong `vifast-root`:
   `function_casts_as_integer` tại `crates/vifast-root/src/main.rs:504`,
   `:607`, `:997`, `:1009` (`cnode_worker as usize`, `pager_server as usize`,
   `pager_client as usize`, `pager2_client as usize`). Build vẫn exit 0.
   Tài liệu cũ ghi "0 warning" là **không còn đúng**. Sửa bằng cách cast qua
   function pointer (`as fn() -> ! as usize`) hoặc allow có chủ đích.
2. `docs/11-hien-tai/09-testing.md` **lỗi thời**: còn ghi "104 test", "3 USER
   FAULT", "x86 chấp nhận `SMP SKIP`", "chưa có persistence" — trái với hiện
   tại (112 test, 6 fault record, x86 SMP thật, persistence 7 phase).
3. `docs/01`–`docs/07`, `legacy/`, `python-model/` là **lịch sử thiết kế**;
   nhiều chi tiết (CNode nhiều cấp, IOMMU phần cứng, pager, `vifast-sim`) chưa
   được port sang bare-metal này. Không implement theo các tài liệu đó.
4. `ViFastOs-architecture-summary.txt` có các mục lịch sử đã cũ (ví dụ "object
   table 128", "chưa có cache block", "chưa có double indirect"). Phần
   `### BEGIN GENERATED ...` mới là số liệu sống.
5. **SHA trong `docs/overnight-log.md` lệch**: log ghi
   `6603365d...` (summary thời vòng 20), summary hiện tại
   `d1dc046d...`. `scripts/make-bundle.py` kiểm SHA này nên bundle vòng 22 sẽ
   **fail** cho tới khi cập nhật. Ngoài ra `docs/bundles/` mới có tới vòng 20.
6. `scripts/check-docs.py` phụ thuộc `target/vifast-qemu-last.json`. Nếu chạy
   QEMU với `--repeat` khác lần ghi chú trong summary, phải chạy lại
   `python3 scripts/gen-docs.py` rồi `check-docs.py`, nếu không sẽ báo
   "summary generated sections are stale".
7. `python3 scripts/test-persist.py --help` **không** in help — script không có
   argparse và sẽ chạy toàn bộ test. Đừng dùng `--help` để thăm dò.
8. Working tree có 2 đường dẫn untracked (không ảnh hưởng build):
   `content/`, `docs/08-giai-thich-cho-sinh-vien-nam-nhat.md`.

---

## 2. Đánh giá tiến độ và sức khỏe dự án

### 2.1 Cột mốc

| Vòng | Nội dung chính | Ý nghĩa |
|---|---|---|
| 19 | headroom RISC-V, TLB mutation, fuzz 100k, CapDelete, suspend/priority, FS single-indirect | hoàn tất hardening cơ bản |
| 20 | per-hart runqueue + work stealing; VFS bulk I/O; hard link; FS v3 bền vững | scheduler SMP thật + VFS dùng được |
| 21 | Chuẩn hóa tên; write-through cache; fsync/BlockFlush; VF04 double-indirect; Sleep deadline; TcbRestart/BindNotification; console input + shell; O_TRUNC/fsutil; tăng capacity | mở đường driver + userland |
| 22 | SchedContext budget; IRQ object + timer/UART IRQ; CNode phẳng; O_APPEND; write-back data; pager demo; TcbDestroy/reap; O_EXCL/O_DIRECTORY; pager server 2 client; STAT_PATH | **50/50 syscall, hết placeholder; user-space service thật** |
| 23 | ObjectDestroy + reclaim Untyped; demo DESTROY OK; 0 warning; docs/bundle 21-22; -smp 4 thử; pager v2 design | **51/51 syscall; vòng đời object khép kín** |

Trong khoảng từ mốc vòng 20 (`925f633`) đến HEAD có **46 commit**, chủ yếu là
feature + docs sync. Tốc độ hiện tại: vài feature dọc/ngày, mỗi feature kèm
host test + QEMU + persistence khi chạm FS.

### 2.2 Điểm mạnh

- **ABI đầy đủ**: không còn syscall placeholder; hành vi CNode/SchedContext/
  Irq đã thật, có test host + demo QEMU.
- **Parity 3 kiến trúc** tốt: mọi demo chính chạy trên cả 3; khác biệt chỉ ở
  chi tiết MMU/GIC/PIC/ACPI.
- **Lõi portable sạch**: kernel/runtime/fs không `unsafe`, không `cfg(target)`;
  platform là nơi duy nhất chạm asm/MMIO.
- **Harness mạnh**: 435 case âm bảo vệ hợp đồng; marker cụ thể, đếm chính xác.
- **FS thực dụng**: journal, cache, fsync, hard/sym link, persistence 7 phase;
  đây là phần "giống OS" nhất.
- **User-space service**: VFS server và pager server chạy ngoài kernel, đúng
  tinh thần microkernel.

### 2.3 Điểm yếu / rủi ro

- **Nợ tài liệu**: ít nhất 4 tài liệu lệch code (mục 1.4); agent mới rất dễ
  làm theo docs cũ.
- **Độ phủ test hẹp**: chỉ QEMU, 2 hart, TCG; chưa có stress 4 hart, chưa có
  máy thật, chưa soak dài sau vòng 20.
- **Giới hạn bộ nhớ**: object table 147, TCB 18; RISC-V chỉ 4 MiB cửa sổ kernel
  với headroom 200.016 byte — thêm static data là dễ tràn.
- **Chưa có thu hồi bộ nhớ thực**: destroy object có, nhưng trang không trả về
  allocator; không có free list; chưa có per-object destroy cho user.
- **`static mut`** còn ở platform AArch64 (bảng trang, data pool, trap frame);
  an toàn nhờ BKL/1 writer nhưng là nợ kỹ thuật.
- **Driver QEMU-specific, polling**: VirtIO-MMIO (RISC-V/AArch64), ATA PIO
  (x86); chưa có IRQ/DMA cho block.
- **Không có hardware IOMMU** nên chưa thể chứng minh cách ly DMA.

### 2.4 Kết luận tiến độ

Dự án đã vượt giai đoạn "bring-up kernel": hiện là **microkernel có user-space
services, FS bền vững, SMP 2 hart và 51 syscall**. Khoảng cách còn lại tới mục
tiêu "OS dùng được" nằm ở **driver + userland + bộ nhớ/scale**, không còn ở
lõi ABI. Vì vậy các vòng tiếp theo nên ưu tiên (a) vòng đời object/bộ nhớ,
(b) pager tổng quát, (c) thiết bị mạng/đồ họa — thay vì thêm syscall.

---

## 3. ViFastOs là gì

Microkernel kiểu capability (seL4-inspired), viết Rust, mục tiêu:

- Lõi portable dùng chung cho RISC-V 64, AArch64, x86_64.
- Cách ly mạnh: kernel tối thiểu; FS, driver, pager, compositor về sau đều
  chạy user-space.
- Capability-based: mọi quyền truy cập object đi qua cap; IPC chuyển cap.
- Hướng dài hạn: ViFastOs Space (thiết bị nhúng/real-time) và ViFastOs
  Desktop (compositor, ViFastKey).

**Không phải**: OS production, Linux-compatible, hay hệ thống đã kiểm chứng
formal. Đây là phòng thí nghiệm OS có kỷ luật test.

### 3.1 Kiến trúc lớp

```text
+--------------------------------------------------------------+
| user mode: ring3 / EL0 / U-mode                              |
| root task | VFS server | client | pager server | fault demos |
+--------------------------------------------------------------+
| vifast-user: syscall wrappers + user layout                  |
+--------------------------------------------------------------+
| vifast-abi: BootInfo v3, SyscallId, quyền, lỗi, VFS protocol |
+--------------------------------------------------------------+
| vifast-runtime: boot, Platform trait, syscall dispatch,      |
|                 timer/preemption, context switch             |
+--------------------------------------------------------------+
| vifast-kernel: object, cap, IPC, scheduler, VSpace walk,     |
|                ramfs fallback, invariant checker             |
+--------------------------------------------------------------+
| vifast-arch-*: primitive (UART/clock/IRQ mask/shutdown)      |
| vifast-platform-*: MMU, trap, timer, IRQ, SMP, ELF load      |
+--------------------------------------------------------------+
| QEMU: OpenSBI | virt,GICv2 | q35+OVMF                        |
+--------------------------------------------------------------+
```

### 3.2 Bản đồ crate

| Crate | Vai trò | `no_std` | `unsafe` |
|---|---|---|---|
| `vifast-abi` | Wire ABI, BootInfo, syscall, quyền, lỗi, VFS protocol | có | cấm |
| `vifast-arch` | Trait `Arch`, primitive contract, BKL, RxQueue | có | cấm |
| `vifast-kernel` | Object/cap/IPC/scheduler/VSpace walk/ramfs/invariant | có | cấm |
| `vifast-runtime` | Boot flow, syscall dispatch, timer, context switch | có | cấm |
| `vifast-user` | Syscall wrapper + layout cho 3 arch | có | có (asm syscall) |
| `vifast-pte` | Encode/decode PTE Sv39/x86/AArch64 | có | cấm |
| `vifast-elf` | Parse/validate ELF64 PT_LOAD, W^X, overlap | có | cấm |
| `vifast-fs` | FS v4 (`VF04`), journal, inode, thư mục, link | có | cấm |
| `vifast-virtio` | VirtIO-MMIO block transport | có | có (MMIO) |
| `vifast-acpi` | ACPI RSDP/MADT cho x86 | có | có (đọc bảng) |
| `vifast-root` | Root task Rust ELF + `_start` asm | có | có (start asm) |
| `vifast-server` | VFS server user-space | có | có (syscall asm) |
| `vifast-client` | Client/shell/demo user-space | có | có (syscall asm) |
| `vifast-fsutil` | Công cụ host format/migrate disk | không | có |
| `vifast-arch-*` | UART/PL011/COM1, clock, IRQ, shutdown | có | có (MMIO/asm) |
| `vifast-platform-*` | MMU/trap/timer/IRQ/SMP/user entry | có | có (asm/MMIO) |
| `vifast-kernel-*` | Entry + linker + user image asset | có | có (entry/asset) |

Quy tắc kiến trúc (quan trọng khi sửa):

- `vifast-kernel` và `vifast-runtime` **không** có `cfg(target_arch)`, không
  địa chỉ UART/MMIO, không page size cụ thể.
- `vifast-platform-*` là lớp BSP duy nhất được dùng `asm!`, MMIO, đọc thanh
  ghi hệ thống, định nghĩa trap frame.
- `Kernel` nằm trong static storage của platform, không trên stack.
- Binary `vifast-kernel-*` chỉ còn entry assembly + linker + user image.

### 3.3 Luồng boot

**RISC-V 64** — OpenSBI → S-mode:

```text
QEMU -bios default -machine virt -smp 2 -m 128M
  → OpenSBI (SBI console/timer/IPI/HSM)
  → vifast-kernel-riscv64 (S-mode, Sv39)
  → hart 1 qua SBI HSM
  → PLIC cho UART, VirtIO-MMIO block 0x10001000..0x10009000
```

**AArch64** — kernel ELF trực tiếp, EL1:

```text
QEMU -machine virt,gic-version=2 -cpu max -smp 2
  → vifast-kernel-aarch64 (EL1, 4-level MMU, GICv2)
  → PSCI CPU_ON CPU1
  → VirtIO-MMIO block trong window 0x0a00_0000
  → generic timer PPI 30, 100 Hz
```

**x86_64** — UEFI:

```text
OVMF → FAT ESP → EFI/BOOT/BOOTX64.EFI (PE/COFF)
  → PML4 riêng, GDT/TSS/IDT, COM1
  → ACPI RSDP/MADT (LAPIC base), PIC/PIT, LAPIC timer 100 Hz
  → INIT-SIPI-SIPI, trampoline < 1 MiB, AP vào long mode
  → ATA PIO LBA28 ở 0x1F0/0x3F6, QEMU ide.6
  → isa-debug-exit 0x501 (exit code 3)
```

---

## 4. Hiện trạng tính năng (đối chiếu code)

| Khu vực | Trạng thái | Bằng chứng / giới hạn |
|---|---|---|
| Boot RISC-V/AArch64/x86_64 | ✅ | QEMU PASS 3/3; x86 AP online |
| Console sớm (16550/PL011/COM1) | ✅ | banner + log |
| Self-test portable | ✅ | `SELFTEST PASS` |
| User mode + ELF Rust | ✅ | root/server/client từ ELF |
| VSpace riêng mỗi task | ✅ | `CLIENT VSPACE OK` |
| Object table cố định | ✅ | 147 object, 18 TCB |
| Capability | ✅ | copy/mint/derive/revoke, badge, GRANT |
| IPC | ✅ | Send/Call/Recv/Reply/ReplyRecv |
| Notification | ✅ | signal/wait/poll, chuyển cap |
| Fault endpoint | ✅ | 6 fault record, pager resume |
| Scheduler | ✅ | priority 0..15, round-robin, preempt |
| SMP 2 hart | ✅ | scheduler hart phụ cả 3 arch |
| Per-hart runqueue + steal | ✅ | host test + `SMP SCHED OK` |
| BKL ticket + maintenance IPI | ✅ | `BKL STATS`, `TLB STATS` |
| Per-VA TLB shootdown | ✅ | RISC/AArch: IPI; AArch broadcast (`ipi=0` cho TLB) |
| ASID/PCID | ✅ | ASID theo VSpace; PCID nếu CPUID |
| W^X theo Frame | ✅ | host test + `KERNEL DATA NX OK` (x86) |
| Untyped/retype + quota | ✅ | Frame/PageTable/VSpace/CNode/Endpoint/Notification/Tcb/SchedContext/Iommu |
| TCB lifecycle | ✅ | configure/resume/suspend/restart/bind/destroy/reap |
| CNode phẳng 64 slot | ✅ | `CNodeMint`/`CNodeInstall`, `CNODE OK` |
| SchedContext budget/period | ✅ | `SCHED CTX OK`, `BlockedBudget` |
| IRQ object | ✅ | `IrqGet`/`SetNotification`/`Ack`; timer + UART |
| Console input IRQ + shell | ✅ | shell block trên notification UART |
| Sleep deadline | ✅ | `SLEEP OK`, `BlockedSleep` |
| VFS user-space | ✅ | server qua IPC, `VFS OK` |
| FS v4 journal | ✅ | `VF04`, 64 inode, thư mục, permission |
| Hard link / symlink | ✅ | `HARD LINK OK`, `SYMLINK OK` |
| Single + double indirect | ✅ | `FS LARGE OK`, file lớn |
| O_APPEND/O_EXCL/O_DIRECTORY | ✅ | `FS APPEND OK`, `FS OPEN FLAGS OK` |
| STAT_PATH | ✅ | `FS STAT PATH OK` |
| Block cache write-back | ✅ | 8 slot, data dirty; metadata write-through |
| fsync / BlockFlush | ✅ | VirtIO + ATA flush |
| Persistence | ✅ | 7 phase × 3 arch |
| Software IOMMU | ✅ | domain, 32 mapping, `IOMMU OK` |
| Pager user-space | ✅ | 2 client, `PAGER OK`/`PAGER2 OK` |
| Hardware IOMMU | ❌ | chưa VT-d/SMMU |
| Network/GPU/input device | ❌ | chưa có |
| Compositor/ViFastKey | ❌ | chưa có |
| CNode guard/nhiều cấp | ❌ | flat 64 slot |
| Pager/mount tổng quát | ❌ | demo 2 client, page table pre-map |
| Free object/trang tổng quát | ❌ | chỉ revoke Untyped/TcbDestroy |
| Khóa mịn/x2APIC | ❌ | BKL toàn cục |
| SMP > 2 hart | ❌ | `MAX_HARTS=4` nhưng chưa test |
| Hardware barrier | ❌ | QEMU; chưa có máy thật |
| Formal verification | ❌ | fuzz giới hạn |
| Userland đầy đủ | ❌ | libc/dynamic linking/app thật |

---

## 5. Chi tiết subsystem

### 5.1 Object, capability, Untyped

- `Object` có generation; `ObjRef { index, gen }` chống use-after-free.
- Bảng cố định `MAX_OBJECTS=147`; `MAX_TCBS=18`; cspace root 96 slot, task 64.
- Kind: `Untyped, Tcb, Endpoint, Notification, CNode, Frame, VSpace,
  PageTable, SchedContext, Iommu, Irq, BlockDevice, Free`.
- Retype từ Untyped có accounting (`OBJ_*_BYTES`, `max_objects`), quota con
  được reserve để sibling không ăn hết.
- `Cap` có rights monotonic, badge, `parent` chain; revoke đi qua cây;
  `CapDelete` chỉ xoá slot, từ chối nếu còn object/cap con phụ thuộc.
- Chuyển cap qua IPC cần `GRANT`; badge do kernel giữ, client không tự đổi.
- `destroy_object` tồn tại (dùng cho revoke Untyped, `TcbDestroy`, teardown):
  unmap Frame mọi nơi, xoá DMA mapping, detach page table, reparent cspace,
  trả slot về `Free` và tăng generation. **Chưa có** đường user destroy từng
  object ngoài các đường trên.

### 5.2 IPC và notification

- Endpoint: `Send`, `Call`, `Recv`, `Reply`, `ReplyRecv`; queue 8 entry.
- Message wire: `label` + `data[4]`; IPC buffer 32 word (256 byte) copy qua PA.
- Notification: `Signal`, `Wait`, `Poll`; bind vào TCB (`TcbBindNotification`).
- Fault endpoint: message `LABEL_FAULT`, `data[0]=addr`, `data[1]=cause`,
  `data[2]=fault endpoint slot`, `data[3]=TCB index`; TCB vào
  `BlockedFault`; handler `Recv`, map Frame, `TcbResume`.
- Badge identity: `BADGE_SUPERUSER=0xFFFF`; FS permission theo badge.

### 5.3 Scheduler, SMP, đồng bộ

- Priority 0..15, round-robin trong cùng priority; `Yield` thật.
- Ready queue **theo hart**; enqueue chọn home theo affinity mask; `pick_on`
  lấy local trước, rỗi mới steal từ hart khác (chỉ TCB có affinity chứa hart
  đích); `TcbSetAffinity` requeue.
- Hart 0 có idle TCB; hart phụ dùng vòng idle tại platform (WFI/HLT).
- Timer 100 Hz cả 3: SBI timer, generic timer, LAPIC.
- BKL ticket FIFO + RAII quanh trap/syscall/IRQ; khi spin có hook
  maintenance-poll để hart khác không chết vì chờ TLB ACK.
- Maintenance IPI: mailbox per-hart; `TLB_VA` + `RESCHEDULE`; per-VA flush dùng
  `harts_loaded` mask; AArch64 broadcast phần cứng.
- `SchedContextConfigure/Bind`: budget/period; hết budget →
  `BlockedBudget`, wake cuối period; destroy context tự unbind và đánh thức.

### 5.4 Bộ nhớ, VSpace, MMU

- Frame/PageTable/VSpace; generic page-table walk qua `PageTableOps` (platform
  cung cấp read/write/flush/zero).
- `PageTableMap` chọn level, kiểm tra toàn dải; `FrameMap` kiểm tra quyền
  mapping ⊆ quyền Frame cap; W^X theo Frame kể cả alias.
- PA uniqueness cho Frame-Frame và PageTable-VSpace; reverse map để revoke.
- `validate_user_range` walk leaf thật; kiểm tra user bit/R/W.
- ASID 12-bit theo VSpace; x86 PCID chỉ khi CPUID (QEMU TCG thường fallback
  CR3); `switch_root` flush khi khác.
- First-fit VA allocator; map EXEC gọi `sync_icache` (fence.i / dc+ic / no-op).
- Layout user: `CODE_VA = 0x10_0000_0000` (64 GiB); window 2 MiB cho bootstrap;
  range user tới `CODE_VA + 0x10_0000_0000`; root ELF ở `+0x20_0000`, stack 4
  trang + guard; BootInfo page read-only; module staging 64 KiB.
- RISC-V kernel accessible 4 MiB (`0x8000_0000..0x8040_0000`), headroom
  200.016 byte; x86 identity map 0..1 GiB.

### 5.5 IRQ, console, shell, thời gian

- IRQ object logic: `IRQ_TIMER=0`, `IRQ_UART=1`; `IrqGet` root-only; raise
  coalesce edge; `IrqSetNotification` signal nếu pending; `IrqAck` xoá.
- UART IRQ: RISC-V PLIC source 10; AArch64 GIC SPI 33; x86 PIC IRQ4/vector
  0x24. RxQueue lock-free 64 byte; shell block trên notification UART.
- Shell ở `vifast-client`: command `help time echo ls cat mkdir rm mv ln stat
  sync truncate fsck`; chưa có job control.
- `GetTime` fast path không lấy BKL (đọc atomic tick BSP); `Sleep` deadline
  tuyệt đối.

### 5.6 VFS, FS, block, persistence

- Server user-space nhận request qua IPC (label protocol):
  `OPEN 0x2001, READ 0x2002, WRITE 0x2003, CLOSE 0x2004, STAT 0x2005,
  MKDIR 0x2006, READDIR 0x2007, UNLINK 0x2008, RENAME 0x2009,
  TRUNCATE 0x200A, FSYNC 0x200B, FSCK 0x200C, BADGE_PROBE 0x200D,
  LINK 0x200E, SYMLINK 0x200F, READLINK 0x2010, STAT_PATH 0x2011`.
- `MAX_IO=128` byte/request qua IPC buffer; fd table 8; permission theo badge.
- FS `VF04`: 512 byte block, 8192 block = 4 MiB; inode 64; tên 28 byte; path
  128; 10 direct + single indirect (128) + double indirect; journal 16 data
  block + commit; bitmap 2 block; metadata/journal write-through; data
  write-back 8 slot; `fsync` → `BlockFlush`; mount được `VF03`/`VF02`.
- Giới hạn thực tế: ảnh 4 MiB, 64 inode, tên 28 byte; file logic tối đa
  ~8 MiB (16.522 block) nhưng bị chặn bởi ảnh 4 MiB; journal ghi từng block,
  không atomic cả file.
- Block: RISC-V VirtIO-MMIO slot `0x10008000` (scan `0x10001000..0x10009000`,
  stride 0x1000); AArch64 VirtIO-MMIO trong window `0x0a00_0000`; x86 ATA PIO
  LBA28 `0x1F0/0x3F6`, QEMU `ide.6`. Polling, một request/lần, 512 byte.
- Persistence do `scripts/test-persist.py`: 7 phase × 3 arch.

### 5.7 IOMMU

- Mô hình phần mềm: `IommuDomain`, tối đa 32 mapping/domain, IOVA base
  `0x1_0000_0000`, window 1 GiB, page-aligned, chỉ READ/WRITE (từ chối EXEC).
- `IoMap`/`IoUnmap`; destroy Frame xoá mapping DMA liên quan.
- Chưa có VT-d/SMMU, invalidation queue, interrupt remapping.

### 5.8 Pager, CNode, TCB lifecycle

- Pager demo: TCB pager trong root VSpace đọc VA chưa map → fault endpoint →
  user-space pager server (TCB riêng) `FrameMap` vào root VSpace →
  `TcbResume`; pager chạy lại, gửi label; server in `PAGER OK`/`PAGER2 OK`.
- Server giữ cap cho 2 cặp TCB/frame; chọn theo fault address; page table phải
  được root tạo trước (pre-map/unmap) vì server không có PageTable cap.
- CNode: bảng phẳng 64 slot; `Tcb.cnode` thay cspace nhúng; `CNodeMint` chỉ
  dest 1..63, rights giảm dần, badge một lần; `CNodeInstall` chỉ cho TCB
  Inactive/Suspended/Faulted; cnode đang dùng không destroy được.
- TCB destroy: reparent cspace, detach khỏi queue/endpoint, xoá cap, trả slot;
  root reap 4 fault TCB + CNode demo (`TCB REAP OK`).

---

## 6. Giới hạn cứng (đọc từ code)

| Hằng | Giá trị | Nguồn |
|---|---|---|
| `MAX_OBJECTS` | 147 | `vifast-kernel/src/object.rs` |
| `MAX_TCBS` | 18 | nt |
| `MAX_CAPS_ROOT` | 96 | nt |
| `MAX_CAPS_TASK` | 64 | nt |
| `MAX_QUEUE` | 8 | nt |
| `MAX_MAPPINGS` | 64 | nt |
| `MAX_IOMMU_MAPPINGS` | 32 | nt |
| `MAX_FDS` | 8 | nt |
| `MAX_HARTS` | 4 | `vifast-kernel/src/lib.rs` |
| `MAX_PRIORITY` | 15 | object.rs |
| CNode slot | 64 | `CNode` |
| Boot Untyped | 128 KiB / 32 object | runtime |
| Metadata Untyped | 80 KiB / 34 object | runtime |
| Data Untyped | 304 KiB / 76 object | runtime |
| `Kernel` size | 709.072 byte | generated status |
| `Object` / `Tcb` / `Cap` | 4.744 / 4.696 / 64 byte | nt |
| RISC-V headroom | 200.016 byte (`kernel_end=0x803cf2b0`) | nt |
| RISC-V kernel window | 4 MiB `0x8000_0000..0x8040_0000` | platform |
| x86 identity map | 0..1 GiB | platform |
| User VA | `0x10_0000_0000` .. `+0x10_0000_0000` | ABI |
| FS image | 4 MiB, 8192 block, 64 inode, tên 28 byte | `vifast-fs` |
| VFS I/O | 128 byte/request, fd 8 | ABI/server |
| IPC buffer | 32 word (256 byte) | ABI |
| Timer | 100 Hz | platform |

---

## 7. Thiếu và nợ kỹ thuật (ưu tiên)

### P0 — nên xử lý sớm

1. **4 warning `vifast-root`** (mục 1.4) — sửa nhỏ, giữ "0 warning" đúng nghĩa.
2. **Đồng bộ tài liệu**: `docs/11-hien-tai/09-testing.md`, bảng crate trong
   `11-hien-tai/00-index.md` (thiếu `vifast-fs`, `vifast-fsutil`,
   `vifast-virtio`), SHA trong `overnight-log.md`, làm rõ "5 vs 6 USER FAULT"
   trong text generated (hợp đồng thật là **6**).
3. **Object lifecycle**: destroy từng object + trả trang; hiện chỉ có revoke
   Untyped/TcbDestroy; double-free/generation đã có nền nhưng chưa có API user.
4. **Mô tả sai trong summary lịch sử** (object 128, chưa cache...) — dọn hoặc
   đánh dấu rõ "historical", tránh agent đọc nhầm.

### P1 — mở khóa tính năng

5. **Pager tổng quát + demand-load**: protocol v2 (map/unmap/evict), cấp
   PageTable/VSpace cap cho pager, nạp trang từ file, page cache.
6. **VirtIO net + stack tối thiểu**: transport dùng chung; ARP/IPv4/ICMP/UDP;
   DMA buffer qua software IOMMU; test ping QEMU user-net.
7. **Hardware IOMMU** (VT-d/SMMU) sau khi có DMA thật.
8. **SMP scale**: `-smp 4`, x2APIC, khóa mịn runqueue/endpoint, test race.
9. **x86 memory map** thay identity 1 GiB; hỗ trợ RAM > 1 GiB.

### P2 — hướng sản phẩm

10. **Framebuffer/GPU/input**: ramfb hoặc virtio-gpu; virtio-keyboard/tablet;
    input server.
11. **Compositor + ViFastKey**: surface/damage/z-order/focus; keymap.
12. **CNode guard + nhiều cấp + zombie**.
13. **Userland**: libc tối thiểu, shell hệ thống, dynamic linking, app thật.
14. **Formal/Kani/Verus**, soak dài, máy thật.

---

## 8. Hợp đồng harness — không được phá

Nguồn sự thật: `scripts/harness_spec.py` (được `test-qemu.py` và `gen-docs.py`
import). Sửa code phải giữ nguyên hoặc **thêm** điều kiện; tuyệt đối không nới.

### 8.1 Điều kiện chung (mọi kiến trúc)

Phải có (đúng chuỗi): `SELFTEST PASS`, `booting on`, `BOOT UNTYPED`,
`PREEMPT OK`, `Isolation`, `Server`, `Client`, `SLEEP OK`, `Mapped`, `Cap OK`,
`ViFastOs ramfs`, `ViFastOs write`, `FS V2 OK`, `FS FSCK OK`, `FS PERM OK`,
`BADGE SPOOF DENIED OK`, `IPC BUF OK`, `BLOCK DENIED OK`, `Reply OK`,
`ROOT RUST`, `BOOTINFO OK`, `SCHED CTX OK`, `RUST MODULES OK`,
`CLIENT VSPACE OK`, `FAULT EP OK`, `ROOT READY`, `Zero OK`, `BLOCK OK`,
`FS DISK OK`.

Đúng **một** lần: `VFS SERVER OK`, `VFS OK`, `VFS STAT OK`, `IOMMU OK`,
`PERCPU CLOBBER OK`, `TIME MONOTONIC OK`, `TLB REMAP OK`, `CAP DELETE OK`,
`TCB SUSPEND OK`, `FS LARGE OK`, `FS APPEND OK`, `FS OPEN FLAGS OK`,
`FS STAT PATH OK`, `HARD LINK OK`, `SYMLINK OK`, `IRQ OK`, `CNODE OK`,
`TCB REAP OK`, `PAGER OK`, `PAGER2 OK`.

**Fault**: đúng **6** dòng `USER FAULT`; phải chứa các địa chỉ
`0x80000000`, `0x1000002000`, `0x1000301000`, `0x1001000000`,
`0x1001001000` (record còn lại hiện là `0x0`). Ghi chú: text mô tả trong
`NUMERIC` còn ghi "5 USER FAULT records" — **hợp đồng thật là 6**, đừng sửa
`accept()` xuống 5.

Cấm xuất hiện: `PERCPU CLOBBER BAD`, `VFS BAD`, `ROOT PANIC`, `STALL`,
`BOOT HANDOFF FAIL`, `KERNEL IRQ OUTSIDE IDLE`, `AP PIC IRQ`,
`BLOCK DENIED BAD`, `FS CORRUPT`, `FS V2 BAD`, `FS PERM BAD`,
`BADGE SPOOF BAD`, `TIME MONOTONIC BAD`, `TLB REMAP BAD`, `CAP DELETE BAD`,
`TCB SUSPEND BAD`, `FS LARGE BAD`, `FS APPEND BAD`, `FS OPEN FLAGS BAD`,
`FS STAT PATH BAD`, `TCB REAP BAD`, `PAGER2 BAD`, `HARD LINK BAD`,
`SYMLINK BAD`, `IRQ BAD`, `PAGER BAD`.

### 8.2 Theo kiến trúc

- **RISC-V/AArch64**: `SMP OK (... ipi=1)`, `SMP TIMER RATE`,
  `SMP TIMER OK`, `SMP IDLE OK`, `SMP PREEMPT OK`, `SMP SCHED OK`,
  `SMP WORKERS OK (a=50, b=50, c=1)`, `SMP POST IDLE OK`,
  `TLB STATS (va_flush>=1, ...)`.
  AArch64 yêu cầu TLB `ipi == 0` (broadcast phần cứng); RISC-V yêu cầu
  `ipi >= 1`, `skipped >= 1`.
- **x86_64**: thêm `KERNEL DATA NX OK`, `ACPI MADT OK (cpus=2)`, `LAPIC OK`,
  `AP TRAMPOLINE OK (page < 0xA0000, aligned)`, `AP REAL MODE OK` →
  `AP LONG MODE OK` → `SMP OK` đúng thứ tự, `SMP IPI COUNT (1<=seen<=sent)`.
- **Exit code**: RISC-V 0, AArch64 0, x86_64 **3** (`isa-debug-exit` ghi 1).

### 8.3 Điều cấm

- Không xoá/giảm fault demo; không đổi địa chỉ fault cho dễ pass.
- Không đổi `ROOT SWITCH` thành `>= 1`; nó phải `> 1`.
- Không bỏ `SMP OK`/`ipi=1`/các counter SMP.
- Không giả lập disk bằng RAM để "cho xanh".
- Không thêm `unsafe` vào kernel/runtime/fs.
- Không commit khi QEMU đỏ hoặc vượt giới hạn RISC-V 4 MiB.
- Không sửa `test-qemu.py`/`harness_spec.py` để dễ pass; chỉ được thêm
  assertion mới khi thêm tính năng.

---

## 9. Đề xuất roadmap cho các vòng tiếp theo

Mỗi mục là một "vertical slice" 1–2 vòng, có test host + demo QEMU + harness
marker mới.

### Vòng 23 (khuyến nghị): Object lifecycle + reclaim

**Mục tiêu**: destroy được từng object và trả bộ nhớ; nền cho pager eviction,
driver buffer recycling, soak dài.

**Phạm vi**:

- Syscall/thao tác destroy cho Frame, Endpoint, Notification, PageTable,
  VSpace, CNode, SchedContext (có thể dùng `CapDelete` mở rộng hoặc syscall
  `ObjectDestroy` mới — cân nhắc ABI).
- Free list/trang trong Untyped: `untyped_free` trả trang về bump/free pool;
  cập nhật accounting `objects/pages`.
- Chống double-free bằng generation; reject destroy nếu còn cap con/đang map
  (hoặc unmap có kiểm soát).
- Demo: tạo/huỷ 100 Frame + 10 VSpace, in `DESTROY OK (frames=100, reused=...)`;
  kiểm tra DATA UNTYPED trở về mức ban đầu.
- Host test: double-free, generation mismatch, destroy khi đang map, destroy
  object của TCB khác, revoke sau destroy.

**Acceptance**: kernel test tăng; QEMU 3/3 × 3; persistence không đổi;
harness thêm `DESTROY OK` + cấm `DESTROY BAD`; không tăng warning.

**Rủi ro**: ABI mới; invariant cap/object; 200 KiB headroom RISC-V.

### Vòng 24: Pager tổng quát + demand-load

- Pager protocol v2: `MAP/UNMAP/EVICT/FAULT_REPLY`; pager giữ cap VSpace +
  PageTable (hoặc kernel cấp bảng qua pager).
- Nạp trang từ file qua VFS (`FrameWrite` hoặc đọc trực tiếp); page cache.
- Demo: 4 client, fault nhiều trang, evict rồi fault lại; `PAGER3 OK`.
- Acceptance: host test protocol; QEMU 3/3; persistence; pager không leak.

### Vòng 25: VirtIO net + stack tối thiểu

- Tách transport VirtIO (MMIO trước), thêm net device; RX/TX queue; IRQ.
- ARP + IPv4 + ICMP echo + UDP echo; dùng DMA buffer qua software IOMMU.
- QEMU user-net, test ping/echo từ host hoặc loopback nội bộ; `NET OK`.
- Sau mốc này làm hardware IOMMU (VT-d/SMMU) mới có ý nghĩa.

### Vòng 26+: chọn 1 trong

- **Framebuffer/input/compositor MVP**: ramfb/virtio-gpu + virtio-keyboard;
  `FB OK`, vẽ + phím; nền ViFastKey.
- **Scale/hardening**: `-smp 4`, x2APIC, khóa mịn, x86 memory map, soak 1h.
- **CNode guard/nhiều cấp + zombie reaping**.

### Việc nhỏ có thể làm ngay (không cần cả vòng)

| Việc | Ước lượng | Ghi chú |
|---|---|---|
| Sửa 4 warning `vifast-root` | 30 phút | cast qua fn pointer |
| Cập nhật `docs/11-hien-tai/09-testing.md` | 1 giờ | 112 test, 6 fault, x86 SMP, persistence |
| Cập nhật SHA trong `overnight-log.md` + bundle 22 | 1 giờ | make-bundle yêu cầu |
| Thêm `vifast-fs`/`vifast-fsutil`/`vifast-virtio` vào bảng crate docs/11 | 15 phút | tài liệu |
| Làm rõ "5 vs 6 USER FAULT" trong generated text | 15 phút | sửa NUMERIC mô tả, giữ accept=6 |

---

## 10. Quy trình làm việc cho agent

### 10.1 Mỗi vòng

1. `git status` — không ghi đè việc đang làm dở.
2. Đọc: `docs/00-hien-trang.md`, `docs/agent2-report-22.md`, tài liệu này,
   `scripts/harness_spec.py`; tra code khi nghi ngờ.
3. Chạy baseline:
   ```bash
   export PATH="$HOME/.cargo/bin:$PATH"
   ./scripts/build.sh
   python3 scripts/test-qemu.py --repeat 3
   python3 scripts/test-persist.py   # nếu chạm FS/block
   python3 scripts/check-docs.py
   ```
4. Chọn **một** hạng mục nhỏ, viết test trước nếu được.
5. Sửa code; giữ `no_std`/`forbid(unsafe_code)` cho kernel/runtime/fs.
6. Chạy host test + QEMU 3 lần; persistence nếu chạm FS.
7. Cập nhật:
   - `docs/overnight-log.md` (mục mới, số liệu, SHA summary);
   - `ViFastOs-architecture-summary.txt` bằng `python3 scripts/gen-docs.py`;
   - `docs/agent2-report-NN.md`;
   - `docs/00-hien-trang.md` nếu trạng thái đổi;
   - bundle nếu cần gửi đi (`scripts/make-bundle.py --round NN ...`).
8. Commit một commit sạch, message theo quy ước.
9. Báo lại: đã làm gì, test nào chạy, còn kẹt gì, giới hạn mới.

### 10.2 Definition of Done

- Build không warning mới.
- Host test liên quan PASS; `cargo fmt --check` PASS.
- QEMU `--repeat 3` 3/3 cả 3 kiến trúc.
- Persistence 7 phase × 3 nếu chạm FS/block/ABI liên quan.
- Harness âm PASS; nếu thêm tính năng, thêm marker + case âm tương ứng.
- `python3 scripts/check-docs.py` PASS (sau `gen-docs.py`).
- Docs + log + report cập nhật; giới hạn mới được nêu rõ.
- Không làm yếu test, không giảm fault demo, không vượt 4 MiB RISC-V.

### 10.3 Quy ước commit

```text
feat: ...
fix: ...
test: ...
docs: ...
refactor: ...
chore: ...
```

Một commit = một thay đổi logic + docs liên quan. Không commit binary/`target/`.

### 10.4 Scripts

| Script | Việc |
|---|---|
| `scripts/build.sh` | host test + root/server/client ELF + 3 kernel |
| `scripts/check-all.sh` | cổng cục bộ: fmt + build + QEMU repeat 3 + persist + harness + docs |
| `scripts/test-qemu.py` | boot QEMU, đối chiếu `harness_spec.py`, dump artifact khi timeout |
| `scripts/test-persist.py` | 7 phase × 3 arch, dùng `vifast-fsutil` |
| `scripts/test-harness-spec.py` | 435 case âm cho harness |
| `scripts/gen-docs.py` | sinh summary từ ABI/git/test/target json |
| `scripts/check-docs.py` | kiểm summary/handoff khớp code + harness |
| `scripts/make-bundle.py` | bundle 1 file: report + status + summary |

---

## 11. Bẫy và điểm dễ sai

1. **PATH**: shell không login không thấy `cargo`; export `~/.cargo/bin`.
2. **`cargo check -p vifast-root` trên host sẽ fail** (unwind/no_std). Chỉ
   build theo target + profile `root-task` như `build.sh`.
3. **`test-persist.py` không có `--help`**; gọi là chạy luôn.
4. **`check-docs.py` phụ thuộc lần chạy QEMU gần nhất**; sau khi đổi
   `--repeat`, chạy `gen-docs.py` rồi `check-docs.py`.
5. **`make-bundle.py` kiểm SHA trong `overnight-log.md`**; SHA đang lệch =>
   phải cập nhật trước khi bundle. Bundle hiện chỉ có tới vòng 20.
6. **RISC-V 4 MiB**: thêm static/array lớn có thể vượt; headroom 200.016 byte;
   runtime assert lúc boot, QEMU sẽ panic nếu tràn.
7. **x86 identity 1 GiB**: OVMF/RAM vượt mốc sẽ `ACPI SKIP`/hỏng; đừng tăng
   `-m` quá cao khi chưa có memory map.
8. **Harness đếm chính xác**: marker demo phải xuất hiện đúng một lần; thêm
   retry/log lặp có thể làm đỏ.
9. **6 fault record**: thêm/bớt fault demo phải sửa harness có chủ đích, không
   nới điều kiện.
10. **Pager cần page table có sẵn**: server `FrameMap` không tự tạo PageTable;
    root phải pre-map/unmap hoặc cấp PageTable cap.
11. **FS geometry cố định**: 4 MiB/64 inode/28-byte name; đổi format phải
    `VF05` + migration + cập nhật persistence phase.
12. **UART IRQ chỉ route BSP/hart 0**: shell/console giả định hart 0.
13. **Block polling, một request/lần**: không gọi song song từ nhiều hart nếu
    chưa có lock; `fsync` dựa vào `BlockFlush`.
14. **`static mut` ở platform AArch64** (bảng trang, data pool, trap frame):
    an toàn nhờ BKL + 1 writer; thêm hart/đường ghi phải audit.
15. **`docs/11-hien-tai/09-testing.md` lỗi thời** và `docs/01`–`07` là lịch sử;
    đừng làm theo.
16. **Hai untracked** (`content/`, `docs/08-...`) không thuộc build; đừng xoá
    khi chưa hỏi.
17. **QEMU x86 exit code 3** là đúng (`isa-debug-exit`), không phải lỗi.
18. **TCG không có PCID**: đừng giả định PCID bật; code phải fallback CR3.

---

## 12. Bản đồ repo (nơi sửa cho việc gì)

| Đường dẫn | Vai trò |
|---|---|
| `crates/vifast-kernel/src/object.rs` | object, cap, hằng số capacity |
| `crates/vifast-kernel/src/lib.rs` | syscall dispatch, scheduler, VSpace walk, revoke/destroy, invariant |
| `crates/vifast-kernel/src/ipc.rs` | endpoint/notification |
| `crates/vifast-kernel/src/fs.rs` | ramfs fallback trong kernel |
| `crates/vifast-kernel/src/selftest.rs` | self-test portable |
| `crates/vifast-runtime/src/lib.rs` | boot, syscall action, timer, context switch, switch_root |
| `crates/vifast-abi/src/lib.rs` | SyscallId, `SYSCALL_STATUS`, BootInfo, layout, VFS protocol |
| `crates/vifast-fs/src/lib.rs` | FS v4/VF03/VF02, journal, inode, link, fsck |
| `crates/vifast-elf`, `vifast-pte` | ELF/PTE thuần, host test |
| `crates/vifast-platform-riscv64/src/platform.rs` | Sv39, trap, SBI timer, PLIC, HSM, VirtIO |
| `crates/vifast-platform-aarch64/src/platform.rs` | 4-level, GICv2, PSCI, VirtIO |
| `crates/vifast-platform-x86_64/src/platform.rs` | PML4, GDT/IDT/TSS, PIC/PIT, LAPIC, ACPI, ATA, AP |
| `crates/vifast-root/src/main.rs` | root task: cấp cap, dựng VSpace, demo, pager, CNode, reap |
| `crates/vifast-server/src/main.rs` | VFS server user-space |
| `crates/vifast-client/src/main.rs` | client, shell, demo FS/IRQ |
| `crates/vifast-fsutil/src/main.rs` | format/migrate/inspect disk trên host |
| `scripts/harness_spec.py` | hợp đồng QEMU (nguồn sự thật) |
| `scripts/test-qemu.py` | chạy QEMU + đối chiếu |
| `scripts/build.sh`, `check-all.sh` | build/cổng cục bộ |
| `docs/00-hien-trang.md` | trạng thái tổng hợp (đọc đầu tiên) |
| `docs/agent2-report-22.md` | báo cáo vòng gần nhất |
| `docs/overnight-log.md` | log từng mốc + số liệu |
| `ViFastOs-architecture-summary.txt` | tổng hợp kiến trúc (generated blocks là số liệu sống) |
| `docs/bundles/` | bundle 1 file các vòng (mới tới 20) |

---

## 13. Câu hỏi mở / cần quyết định

1. Ưu tiên vòng 23: object lifecycle hay pager tổng quát? (Đề xuất: object
   lifecycle trước vì pager eviction cần reclaim.)
2. Có cần giữ chuẩn "1 file bundle/vòng" cho agent0 không? Nếu có, sửa SHA
   log + sinh bundle 21/22.
3. Có mục tiêu thời gian cho mốc "chạy được ứng dụng đồ họa đầu tiên" không?
   Điều này quyết định làm virtio-gpu/ramfb trước hay net trước.
4. Có cần hỗ trợ 4 hart ngay (thay vì 2) để chuẩn bị cho server đa nhân?
5. Chấp nhận ABI mới cho destroy/pager đến mức nào (thêm syscall vs mở rộng
   syscall cũ)?

---

## Phụ lục A — 51 syscall (0..50)

```text
 0 Send              1 Call               2 Recv              3 Reply
 4 ReplyRecv         5 Signal             6 Wait              7 Poll
 8 Yield             9 Retype            10 CapCopy          11 CapMint
12 CNodeMint        13 CapDelete         14 CapRevoke        15 TcbConfigure
16 TcbResume        17 TcbSuspend        18 TcbRestart       19 TcbSetPriority
20 TcbSetAffinity   21 TcbBindNotification 22 SchedContextConfigure
23 SchedContextBind 24 PageTableMap      25 FrameMap          26 FrameUnmap
27 IoMap            28 IoUnmap           29 IrqGet            30 IrqSetNotification
31 IrqAck           32 DebugWrite        33 GetTime           34 Exit
35 Open             36 Read              37 Write             38 Close
39 Stat             40 Seek              41 TcbSetIpcBuffer   42 FrameWrite
43 BlockRead        44 BlockWrite        45 BlockFlush        46 Sleep
47 ConsoleRead      48 CNodeInstall      49 TcbDestroy       50 ObjectDestroy
```

Tất cả `SYSCALL_STATUS = true` (implemented, 0..50); không còn placeholder.

## Phụ lục B — Commit tiêu biểu (một phần, từ mốc vòng 20)

```text
c86de49 docs: document VFS stat-by-path
13c26f8 feat: add VFS stat-by-path request
43fb019 docs: document tcb reap, open flags, two pager clients
46e8ad6 feat: reap TCBs and serve two pager clients
79005ce feat: move pager handling into user-space server
ca4d2f4 feat: add demand paging fault-resume demo
801fb13 test: cover cnode delete and revoke integration
363574a feat: add write-back block cache for file data
e9b8286 feat: implement o_append for vfs
b014c8b feat: add end-to-end cnode demo task
6d3129f feat: add flat cnode cspace and cnode install
c521528 feat: route uart irq and make shell interrupt-driven
814e28f feat: add irq objects and timer irq delivery
aaafbe0 feat: implement sched context budget scheduler
af197ea feat: expand shell and raise object capacity
5bd47c8 feat: add console input and shell commands
bdc85e5 feat: add o_trunc and fsutil utilities
3e33cfa feat: add bsp idle tcb and blocking sleep
26b9c2c feat: implement tcb restart and bind notification
8f7ece9 feat: add sleep deadline primitive
b9ed3bc feat: add vf04 double indirect files
6e415a7 feat: add real fsync and block flush
debcf57 feat: add vfs write-through block cache
49de105 chore: normalize ViFastOs spelling
36147fb feat: fs symlinks and object capacity headroom
```

(Đầy đủ: `git log --oneline 925f633..HEAD` — 46 commit.)

## Phụ lục C — Số liệu nhanh

- Tổng commit: 256.
- Từ vòng 20: 46 commit.
- QEMU marker chính: xem mục 8 và `scripts/harness_spec.py`.
- Tài liệu nguồn: `docs/00-hien-trang.md`, `docs/agent2-report-22.md`,
  `ViFastOs-architecture-summary.txt` (generated blocks).
- SHA-256 summary hiện tại: `d1dc046df6f1ad63096c9a9d620c942e5abeea363e91f19312b90703104b9f97`
  (log đang ghi SHA cũ — xem mục 1.4).
