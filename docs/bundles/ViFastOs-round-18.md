# ViFastOs round 18 bundle

## Part 1 - Agent 2 report

# ViFastOs — Báo cáo Agent 2, vòng 18

**Ngày:** 2026-11-05
**Baseline:** `85cbdb9` / bundle vòng 17
**Code commit cuối:** `111e072`
**Docs commit:** `docs: round 18`

## 1. Bảng task → commit → test riêng

| Task | Commit | Test đã chạy cho commit đó | Kết quả |
|---|---|---|---|
| 1. RISC-V panic exit code | `2851cdf` | `test-panic-hart1 --repeat 5` cả 3 | ✅ RISC rc=1 |
| 2. Chặn leo thang badge (D6) | `763f6ff` | kernel host 92/92; QEMU 1/1 + 50/50; badge lines | ✅ |
| 3. GetTime đơn điệu | `5e9c11c` | kernel host 93/93; QEMU 50/50 cả 3 | ✅ |
| 4. FS ảnh nhỏ / ENOSPC / dir full | `2ea3944` | `vifast-fs` 10/10 | ✅ |
| 5. Per-VA TLB shootdown | `111e072` | kernel host 94/94; QEMU 50/50 cả 3 | ✅ |
| 6. Docs R6 | `docs: round 18` | `check-docs.py`, `test-harness-spec.py` 300 case | ✅ |

Task 1–5 tách commit; Task 6 chỉ docs (không gộp code).

## 2. Kết quả test cuối

```text
./scripts/build.sh                  PASS, 0 warning
cargo test -p vifast-kernel         94/94
cargo test -p vifast-arch            6/6
cargo test -p vifast-pte            11/11
cargo test -p vifast-elf            17/17
cargo test -p vifast-acpi            8/8
cargo test -p vifast-fs             10/10
cargo test -p vifast-fsutil          0/0 PASS
test-qemu.py --repeat 10            10/10 cả 3 kiến trúc
riscv64 --repeat 50                 50/50
aarch64 --repeat 50                 50/50
x86_64 --repeat 50                  50/50
test-panic-hart1 --repeat 5         5/5 mỗi kiến trúc
test-boot-handoff --repeat 10       10/10 (RISC-V)
test-persist.py                     6/6 phase × 3 kiến trúc
test-harness-spec.py                300 case âm PASS
cargo fmt --all -- --check           PASS
check-docs.py                        PASS
```

RISC-V headroom: `138672` byte, `kernel_end=0x803de250`.

## 3. Task 1 — RISC-V panic

Đã sửa `panic_exit`:

```text
value = (1 << 16) | 0x3333 = 0x00013333
```

QEMU 11.1.3, lệnh `-machine virt -smp 2 -bios default -kernel ...`:

```text
riscv64: rc=1 (5/5)
aarch64: rc=1 (5/5, semihosting SYS_EXIT_EXTENDED)
x86_64:  rc=7 (5/5, isa-debug-exit)
```

Đường thành công của RISC-V ghi `0x5555`; QEMU boot bình thường vẫn rc=0.
`test-panic-hart1` harness nay yêu cầu rc != 0 trên cả ba kiến trúc.

## 4. Task 2 — D6 badge

**Trước sửa:** `CapMint` trong `85cbdb9:crates/vifast-kernel/src/lib.rs:3522-3555`
không có kiểm tra `source.badge`; nó gọi `copy_cap(..., a4)`. `Cap::derive`
(`object.rs:80-96`) giữ badge nguồn khi `a4=0`, và từ chối badge mới nếu nguồn
đã có badge, nhưng không có quy tắc D6 rõ ràng/`IllegalOperation`. Badge được
trả cho user qua **`value`** (`runtime/src/lib.rs:117-126`), không có trường
riêng.

**Sau sửa:**

- Badge register: RISC-V `a7`, AArch64 `x7`, x86_64 `r10`; `MessageResult.badge`.
- `Message.badge` do kernel đặt từ endpoint cap trước khi direct delivery và
  trước khi đưa sender vào hàng đợi; sender không có trường nhập badge.
- `CapMint` source badge != 0 → `IllegalOperation`, cap đích không tạo.
- `CapCopy` giữ badge nguồn.
- Server: badge 0 → `EACCES`; superuser duy nhất `0xFFFF`; uid thường lấy badge.
- Client chỉ giữ endpoint badge 1/2; thử mint lại badge 2 → 0 thất bại; gửi
  probe uid giả `0`/`0xFFFF` qua badge 2, server trả badge=2 và từ chối ghi
  `/priv`; `BADGE SPOOF DENIED OK` đúng một lần.

Host test `kernel_overwrites_sender_badge_on_direct_and_queued_delivery` và
`capmint_rejects_rebadging_an_already_badged_cap`.

## 5. Task 3 — GetTime

Chọn **`ticks[0]` BSP** làm thời gian hệ thống 10 ms. Lý do: mọi kiến trúc đã
có nguồn tick BSP 100 Hz; không cần quy đổi `time` CSR / `cntpct_el0` / TSC
riêng, và một task chuyển hart không thấy thời gian lùi/nhảy. `ticks[hart]`
vẫn giữ cho timer/preempt/SMP harness.

- `GetTime` trả `ticks[0]` cho cả hart 0 và hart 1.
- Worker c giữ bận `start + 5` tick BSP, tương đương ≥5 timer tick hart 1;
  `U >= 3` vẫn đạt.
- Root kiểm tra monotonic trên BSP, worker c kiểm tra trên AP; in
  `TIME MONOTONIC OK` đúng một lần. Host test `gettime_is_system_wide_bsp_time_not_per_hart`.
- 50/50 mỗi kiến trúc.

## 6. Task 4 — FS ảnh nhỏ

Thêm `FormatOptions { total_blocks, inode_limit, dir_block_limit }`;
`format_with_options` lưu các giới hạn vào superblock VF02 (inode count +
dir block limit; CRC32). Mặc định 64 inode/12 block.

Test mới:

1. Ảnh 48 block (data LBA30..47 = 18 block). Root dir 1 + `/full` 12 +
   `/rest` 5 = 18; write `/third` trả `NoSpace`; remount/fsck sạch; xóa
   `/full` rồi ghi `/again` 8 block thành công.
2. `dir_block_limit=1`: 16 entry đầy block, entry 17 trả `NoSpace`; xóa một
   entry rồi tạo lại thành công; fsck sạch.
3. Ảnh 49 block (19 data), gần đầy (root 1 + full 12 + rest 5 = 18, còn 1
   block); crash tại mọi write của `/spare`, mỗi lần mount/fsck sạch và file
   cũ hoặc mới đầy đủ, không partial.

## 7. Task 5 — Per-VA TLB

- `VSpace.harts_loaded: u32`; `note_vspace_hart_loaded` khi hart nạp root,
  không xóa khi switch sang VSpace khác; full-ASID flush mới xóa bit.
- `PageTableOps::flush_va_asid(asid,va,pages,targets)` và fallback
  `flush_all_asid`. Unmap lấy `harts_loaded`; nếu >16 trang dùng full ASID.
- RISC-V mailbox thêm `TLB_VA` + payload ASID/VA/pages; x86 thêm
  `AP_MAINT_TLB_VA`, flush `invlpg`/CR3 fallback; AArch64 dùng `tlbi vae1is`
  broadcast, không gửi IPI.
- `TLB STATS (va_flush=V, ipi=I, skipped=K)` in sau demo.
- Worker d (hart 1, priority cao) đọc A, báo root, chờ notification; root
  unmap A/map B, gửi IPI target, signal d; d đọc B và báo.
- 10 lần/lần chạy đầu:

```text
RISC-V  V=4 I=1 K=3, latency=0..1 tick
AArch64 V=4 I=0 K=0, latency=0..0 tick
x86_64  V=4 I=1 K=3, latency=0..0 tick
```

50/50 mỗi kiến trúc; `TLB REMAP OK (seen_new=1)` đúng 1 lần; vẫn đúng 4
USER FAULT. Host test `vspace_hart_mask_tracks_loaded_harts_and_full_flush_clear`.

## 8. Task 6 — Docs

- `check-docs.py` so chuỗi cấm không phân biệt hoa/thường.
- Thêm cấm `BootInfo v2`, `stack 1 trang`, `phase âm magic sai`; pass spec
  đổi thành BootInfo v3.
- Mục 9/12 thống nhất server stack `+0x2000..+0x5000` (3 trang); mục 10 nói
  journal trong `.bss`, không còn “stack 1 trang”.
- Mục 14 mô tả 6 phase test-persist; mục 11 badge register + GetTime; mục 13
  per-VA TLB; mục 16 R2 đã đóng; mục 18 D6/vòng 18.
- Xóa câu x86 lặp điều kiện B; sửa “hai worker” thành bốn worker A/B/C/D.

## 9. Nợ / giới hạn

- FS v2 vẫn 64 inode, file 6 KiB; chưa có block cache/indirect/hard link/
  symlink. Crash test giả định sector ghi nguyên tử; chưa có barrier đĩa thật.
- Thư mục đầy vật lý 12 block không đạt với 64 inode; đã test nhánh giới hạn
  `dir_block_limit=1` đúng như decision cho phép.
- IOMMU phần cứng, per-hart runqueue/khóa mịn/x2APIC, `CapDelete` syscall,
  net/input/compositor chưa làm.
- RISC-V headroom còn `138672` byte; theo dõi.

## Part 2 - Quick status

```text
code commit: 111e072
docs commit: round 18 docs
tests: kernel 94/94, arch 6/6, pte 11/11, elf 17/17, acpi 8/8, fs 10/10; QEMU 10/10 all, 50/50 each; panic 5/5 each; handoff 10/10; persist 6 phases each; harness 300
RISC-V headroom: 138672 byte (kernel_end=0x803de250)
summary SHA-256: 282f2b7fe48c847a9decb59b3864292ea7ccf0c95cfe49a5a829ae0f628b469e
```

## Part 3 - Full ViFastOs-architecture-summary.txt

```text
ViFastOs - TỔNG HỢP KIẾN TRÚC HIỆN TẠI VÀ ĐỊNH HƯỚNG PHÁT TRIỂN
================================================================

Mục đích file này
-----------------
Tài liệu tự chứa (self-contained) để copy/paste cho AI khác hoặc người khác
đọc và thảo luận thiết kế. Nội dung phản ánh code hiện tại trong:

    /Users/mps/dvchd/ViFastOs

Đây là một microkernel viết bằng Rust, hướng tới nhiều kiến trúc:
RISC-V64, AArch64 và x86_64. Project đang ở mức "bring-up kernel + user demo",
không phải một hệ điều hành production hoàn chỉnh.

Thư mục này LÀ git repository. Các mốc hiện tại:
### BEGIN GENERATED commits
- 456d5ef checkpoint: multi-arch microkernel before hardening
- eece7c3 harden: kernel trap stack, private x86 page tables, explicit cap slots
- 3f16fb1 feat: root task, user pointer validation, untyped accounting
- 604576e test: exercise PAN/SMEP/SMAP and document root task
- c79d8f9 fix: destroy untyped children and isolate user faults
- c9e3028 fix: distinguish kernel faults, zero retyped frames, record 3b design
- 7cf9adc refactor: efiapi exceptions, PageTableOps, static kernel storage
- 5b1a992 feat: generic page-table ops, IST, larger data untyped
- e785646 feat: vifast-pte and generic mapping walk
- cc94730 test: cover generic page-table safety rules
- 611ab2c feat: centralize user VA ranges and table spans
- 68dcd50 feat: add PageTableMap and dependency-ordered revoke
- e748d1b test: pin user VA boundary indices
- 4ab18c6 feat: retype VSpace and harden cross-untyped revoke
- 99f7179 test: keep kernel slots untouched by PageTableMap
- 27a40f1 fix: scope Untyped revoke and assert kernel mappings
- 9a325c5 fix: size the data Untyped object quota
- 0eb3d67 fix: use exact cap ids and walk kernel mappings
- d1c42b0 fix: reclaim orphaned objects and assert kernel image range
- c9c4591 fix: reparent dying capability subtrees
- 43d5200 test: rename reparented object case
- df9e1c1 feat: add boot untyped and task-space builder
- eae0741 test: cover builder and shared frame unmapping
- 24c668d feat: validate user pointers through page tables
- a68b488 test: exercise real PTE formats through fake ops
- 1c488a2 feat: build root task space from boot untyped
- 3758e81 feat: isolate server VSpace and switch roots from TCB
- 1fb01b2 feat: centralize user layout and add ELF parser
- 7fd7a3d feat: harden ELF parser constraints
- 4b667b9 feat: load ELF segments into a task VSpace
- 91dfbcd feat: add Rust root ELF link scaffold
- 71b83c9 feat: boot Rust root task from ELF
- 3328169 test: bound FrameMap rights by frame cap
- f4a9da6 fix: enforce wx per frame across aliases
- c363e7d fix: enforce unique physical ranges and sync icache
- b0a82cb feat: add child untyped allocation and revoke
- ba17546 feat: pass bootinfo and user code caps to root
- 39fc83c feat: build private client vspace
- aeea6bc feat: isolate client vspace and add fault4
- c5119e7 feat: add reply-recvsyscall and server loop
- a18663f feat: deliver user faults to endpoint
- 854744a feat: add ipc buffer binding and copy
- 49e7b1f feat: demo ipc buffer transfer across tasks
- 82c9c11 feat: mark x86 kernel data non-executable
- ed40921 test: fuzz syscalls across pte formats
- ea3793b feat: load rust server and client elf modules
- 6963f58 refactor: supply rust modules through bootinfo
- 562727d feat: add first-fit vspace range allocator
- 395a589 feat: demo notification capability transfer over call
- 85fe2cf feat: add user-space vfs server over ipc
- 826efed feat: tag vspaces with asid and pcid
- fffa7d6 feat: add vfs stat request
- 687962c feat: add software iommu domains and dma maps
- ed06f82 feat: add smp maintenance ipi and tlb shootdown
- f216966 feat: add aarch64 virtio block backend
- 1d4f0ea feat: persist vfs scratch on block storage
- fe569c1 feat: add x86 ata block backend for persistent vfs
- 096faf8 test: hermetic two-boot persistence check
- 4293406 refactor: add big kernel lock and per-cpu state groundwork
- 23d8dfb feat: riscv virtio-mmio block backend
- b51782b fix: make bkl cover all kernel entries and assert release on user return
- 4d44b60 refactor: per-cpu trap stacks and current tcb
- 74b7d75 feat: per-cpu security and per-hart current groundwork
- 31f023b test: reject revoke of tcb current on another hart
- c4854d1 fix: symmetric per-hart maintenance mailbox
- 76f1c41 feat: riscv secondary hart idle loop and reschedule ipi
- 067b0d9 chore: single-file round bundle
- b76e930 feat: run tasks on riscv secondary hart
- e05775a feat: two workers on riscv hart 1 with real per-hart counters
- b7f26ef fix: riscv idle uses wfi with interrupts masked
- ce7dbc9 fix: restrict smp workers to riscv
- e895bda fix: send reschedule ipi only to idle harts
- dbe7675 refactor: bootinfo advertises scheduling harts
- e3c4280 chore: aarch64 kernel exception reports vector and cpu
- b9bfad9 chore: automated docs consistency check
- c7c45ed feat: aarch64 secondary cpu idle with masked wfi
- 3631ed7 chore: derive docs checks from abi, harness and git
- 060fb67 test: repeat mode and register dump on qemu timeout
- 010cccd test: drain qemu monitor banner before register dump
- 386a2ad fix: handle arbitrary boot hart and empty secondary runqueue
- dbe3a39 fix: accurate idle wake accounting on secondary harts
- cce094a feat: run tasks on aarch64 secondary cpu
- ec090c9 chore: generate summary sections from abi, harness and git
- 17aee2a test: restore and negative-test every harness condition
- d8424e5 test: prove timer wakes idle secondary harts after workers block
- b122c05 test: inject boot hart handoff on riscv
- 32c2e03 chore: syscall status and generated status block
- 1ff8644 fix: bounded boot handoff fails loudly
- 9b8f031 feat: parse acpi madt on x86
- 627f054 feat: map and enable x86 local apic
- 83875a7 fix: use madt lapic base before mapping
- 78af406 feat: allocate x86 ap trampoline page below 1 mib
- dd07e6b feat: x86 ap reaches real mode via init-sipi-sipi
- 7dc8a94 feat: x86 ap enters long mode
- 49c4522 feat: x86 ap online with maintenance ipi
- 03e8c1d fix: treat x86 lapic pdpt slot as kernel shared
- 6ed6cde fix: single sipi sequence, per-cpu tss/gs on x86 ap, real lapic ipi
- 03cb048 feat: x86 ap lapic timer and idle via irr polling
- 58016c8 feat: x86 ap scheduler workers and trampoline stages
- 7196a4e feat: smp hardening bkl owner ap preempt panic halt stats
- 225ccdb fix: mask interrupts on panic halt
- 34db2e0 fix: serialize panic console output
- 85cbdb9 feat: fs v2 journaled vfs per-hart ticks block cap capmint
- 2851cdf fix: riscv panic exit code via sifive_test fail code
- 763f6ff fix: forbid rebadging and spoofed badge delivery
- 5e9c11c fix: system-wide monotonic GetTime
- 2ea3944 test: vifast-fs enospc and full directory on small images
- 111e072 feat: per-va tlb shootdown targeted by vspace hart mask
### END GENERATED commits

Cập nhật code gần nhất (commit 111e072): RISC-V panic rc=1, D6 badge chống
giả mạo, GetTime toàn hệ thống, FS ảnh nhỏ ENOSPC/thư mục 1 block, per-VA TLB
shootdown + mask harts_loaded; QEMU 10/10 và 50/50 cả 3 kiến trúc; persistence
6 phase.
VFS scratch trên cả 3 kiến trúc,
VirtIO-MMIO/ATA block, BKL ticket + per-CPU security + per-hart current, hermetic persistence test, SMP maintenance IPI/TLB shootdown, software IOMMU, VFS STAT, ASID/PCID, user-space VFS server, notification cap transfer, VSpace range allocator, runtime cấp Rust server/client ELF qua BootInfo, FrameWrite, current_root từ TCB,
FrameUnmap cần Frame cap, ROOT SWITCH n>1, stack/slot constants trong ABI,
vifast-elf parser machine/window và build_task_space_elf đã có test host,
root Rust ELF + server/client Rust ELF nạp vào boot, RUST MODULES OK/ROOT READY, jump table, FrameMap cap bound,
W^X theo Frame, PA uniqueness, sync I-cache, Untyped con/reserved, BootInfo, client VSpace, fault4, ReplyRecv, fault endpoint, IPC buffer + FrameWrite loader, notification cap transfer, user-space VFS server, 3 target link/FP
validator, ROOT_ELF_BASE/soft-float chốt, boot quota 117, revoke Untyped thật, fault
suspend, max_objects, Yield/Wait, kiểm tra con trỏ user, zero frame/PageTable,
PageTableOps generic + generic page-table walk, exception vector x86, IST,
Kernel static storage, Untyped accounting, vifast-pte (format PTE + user VA
range + table_span), PageTableMap level động, retype VSpace/install_kernel,
creator CapRef id + reparent, delete cap KResult, boot Untyped,
build_task_space, validate_user_range walk leaf, AArch64 leaf rights decode,
PteFakeOps 3 format tách test, boot quota 117 object, KERNEL_SHARED_SLOTS,
kernel accessible range, x86 LoadedImage assert, thứ tự revoke
Frame → PageTable sâu → VSpace, vifast-pte host test, SMEP/SMAP/PAN/sscratch
hardening và vifast-user.

1. MỤC TIÊU
-----------
- Một lõi kernel portable, không phụ thuộc phần cứng.
- Nhiều port kiến trúc, dùng chung logic object/capability/IPC/scheduler/MMU/VFS.
- Chạy được trên QEMU cho cả 3 kiến trúc.
- Hướng tới microkernel kiểu seL4, capability-based.
- Có user mode, syscall, timer preemption, IPC, capability transfer, memory
  mapping, VFS tối giản.
- Về lâu dài hướng tới desktop/space profile, driver user-space, filesystem,
  network, compositor, ViFastKey và formal verification.

2. TRẠNG THÁI HIỆN TẠI (TÓM TẮT)
-------------------------------
Đã chạy được trên QEMU:
- RISC-V64 (OpenSBI, -smp 2)
- AArch64 (QEMU virt, GICv2, -smp 2)
- x86_64 (OVMF/UEFI, -smp 2; AP scheduler + bốn worker)

Test hiện tại:
### BEGIN GENERATED tests
- `cargo test -p vifast-kernel`: 94/94 PASS
- `cargo test -p vifast-arch`: 6/6 PASS
- `cargo test -p vifast-pte`: 11/11 PASS
- `cargo test -p vifast-elf`: 17/17 PASS
- `cargo test -p vifast-runtime`: 0/0 PASS
- `cargo test -p vifast-user`: 0/0 PASS
- `cargo test -p vifast-acpi`: 8/8 PASS
- `cargo test -p vifast-fs`: 10/10 PASS
- `cargo test -p vifast-fsutil`: 0/0 PASS
### END GENERATED tests
### BEGIN GENERATED status
- RISC-V headroom: 138672 byte (`kernel_end=0x803de250`)
- QEMU lần chạy gần nhất (`--repeat 10`): riscv64 10/10, aarch64 10/10, x86_64 10/10
### END GENERATED status
- root ELF link: PASS (ET_EXEC, segment/window, soft-float check)
- root ELF stripped: RISC-V 12.880 B, AArch64 12.736 B, x86 16.744 B
- vifast-runtime chỉ tạo root TCB; root task tự retype/cap-copy/configure/resume
  hai TCB server/client, bốn fault TCB và notification riêng cho root.
- ./scripts/build.sh: PASS, không warning

Output tiêu biểu:
- SELFTEST PASS
- SMP OK (harts=2, ipi=1) trên cả RISC-V/AArch64/x86_64
- ACPI MADT OK (cpus=2, bsp=0, lapic=0xfee00000), LAPIC OK (id=0) trên x86_64
- "ROOT RUST", "BOOTINFO OK", "CLIENT VSPACE OK", "ROOT READY", "Fault",
  "Fault4", "USER FAULT" ×4, "Isolation", "VFS SERVER OK",
  "Zero OK", "Server", "Mapped", "ViFastOS ramfs", "ViFastOS write",
  "Client", "Cap OK", "Reply OK", "VFS OK", "VFS STAT OK", "IOMMU OK",
  "SMP OK (... ipi=1)", "SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)",
  "SMP TIMER OK (hart1_ticks=T)" trên cả 3 kiến trúc,
  "SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)",
  "SMP PREEMPT OK (user_ticks=U, preempts=P)",
  "SMP WORKERS OK (a=50, b=50, c=1)", "BKL STATS (h0_acq=..., h1_acq=...)",
  "BLOCK DENIED OK", "FS V2 OK", "FS FSCK OK", "FS PERM OK",
  "BADGE SPOOF DENIED OK", "TIME MONOTONIC OK", "TLB REMAP OK (seen_new=1)",
  "TLB STATS (va_flush=V, ipi=I, skipped=K)", "TLB LATENCY (ticks=N)",
  "PERCPU CLOBBER OK",
  "BLOCK OK"/"FS DISK OK", "PREEMPT OK", "ROOT SWITCH n>1";
  persistence do `scripts/test-persist.py` kiểm tra.
- user layout chung: boot mappings tại 0x0000_0010_0000_0000 (64 GiB);
  full user range hiện tại 64..128 GiB

3. KIẾN TRÚC TỔNG THỂ SAU REFACTOR
----------------------------------
Mục tiêu của refactor: lõi portable không biết phần cứng; phần cứng nằm trong
platform/arch crate; binary kernel chỉ còn entry/linker/user asset.

Sơ đồ lớp:

    +--------------------------------------------------+
    | user mode: ring3 / EL0 / U-mode                   |
    +--------------------------------------------------+
    | vifast-kernel (portable, no_std, forbid unsafe)  |
    | object - capability - IPC - scheduler - VSpace   |
    | generic walk + reverse map - ramfs - ABI checks  |
    +--------------------------------------------------+
    | vifast-runtime (generic boot/syscall/timer)      |
    | no_std, forbid unsafe                            |
    +--------------------------------------------------+
    | vifast-user (syscall wrappers, no_std)           |
    +--------------------------------------------------+
    | vifast-pte (safe PTE encode/decode, no_std)      |
    +--------------------------------------------------+
    | vifast-platform-* (BSP integration)              |
    | trap - MMU - timer - IRQ - SMP - user entry      |
    +--------------------------------------------------+
    | vifast-arch-* (hardware primitives)              |
    | UART/PL011/COM1 - clock - IRQ mask - shutdown    |
    +--------------------------------------------------+
    | vifast-kernel-* (entry + linker + user image)    |
    +--------------------------------------------------+
             |                |                |
          RISC-V64          AArch64          x86_64
          OpenSBI           QEMU virt        OVMF/UEFI

Quan hệ phụ thuộc chính:
- vifast-kernel -> vifast-abi + vifast-pte (không còn phụ thuộc vifast-arch)
- vifast-runtime -> vifast-kernel + vifast-arch
- vifast-user -> vifast-abi (syscall wrappers + layout)
- vifast-elf -> không dependency (ELF64 PT_LOAD parser, no_std, host-tested)
- vifast-root -> vifast-abi + vifast-user (user ELF, không default-members)
- vifast-platform-* -> vifast-runtime + vifast-kernel + vifast-arch-*
- vifast-platform-* implement PageTableOps (read/write/flush/zero)
- vifast-kernel-* -> vifast-platform-* + vifast-runtime + vifast-arch
- Không có cfg(target_arch) trong vifast-kernel hoặc vifast-runtime.

4. CÁC CRATE VÀ TRÁCH NHIỆM
---------------------------
vifast-abi
- Wire ABI, no_std, forbid unsafe.
- KError, Rights, ObjType, ThreadState, Message, SyscallId.
- Hằng số page/message/cờ file.
- File: crates/vifast-abi/src/lib.rs.

vifast-arch
- Trait Arch + BootInfo + primitive contract.
- no_std, forbid unsafe.
- Không biết kernel object, MMU, trap hoặc page table.
- File: crates/vifast-arch/src/lib.rs.

vifast-kernel
- Toàn bộ logic portable:
  - object table cố định có generation;
  - capability install/copy/derive/revoke;
  - endpoint send/recv/call/reply;
  - notification signal/wait/poll;
  - scheduler priority + round-robin;
  - Frame/VSpace/PageTable generic page-table walk + reverse map;
  - PageTableMap: kernel chọn level, kiểm tra toàn dải bảng;
  - validate_user_range() walk page table leaf thật; kiểm tra user bit, R, W;
  - Untyped/retype có accounting metadata;
  - root-task syscalls: Retype, CapCopy, TcbConfigure, TcbResume, PageTableMap;
  - build_task_space, build_task_space_shared_code, build_task_space_elf;
  - PageTableOps copy_to_frame_at cho segment bắt đầu giữa trang;
  - ramfs + fd table;
  - invariant checker.
- no_std, forbid unsafe.
- Files:
  - lib.rs
  - object.rs
  - ipc.rs
  - fs.rs
  - selftest.rs
  - console.rs.

vifast-runtime
- Logic dùng chung cho mọi port:
  - Platform trait;
  - boot sequence chỉ tạo một root TCB;
  - giao VSpace/Untyped cho root task;
  - syscall action dispatch (gồm ConfigureTcb);
  - dựng root VSpace từ boot Untyped bằng TaskLayout (code RX, 4 stack page
    fault/client, không map data/server stack); parse/nạp root ELF + stack Rust;
  - dựng VSpace server từ boot Untyped, code dùng chung Frame root, stack riêng;
  - cài Frame cap code READ|EXEC và map BootInfo READ-only tại ROOT_BOOTINFO_VA;
  - truyền BootInfo trong arg0 của context root;
  - root VSpace chỉ còn stack fault1/2/3; client stack/fault4 nằm trong VSpace client;
  - log lại DATA UNTYPED ở test-exit;
  - đọc root từ TCB mỗi lần restore; switch_root + full flush khi khác;
  - kiểm tra loaded root trong read_user/write_user;
  - context switch khi block;
  - timer tick/preemption.
- no_std, forbid unsafe.
- File: crates/vifast-runtime/src/lib.rs.

vifast-pte
- no_std, forbid unsafe; format Sv39, x86_64 4-level, AArch64 4 KiB.
- encode/decode/index thuần bit; table_span() cho từng level.
- USER_VA_START/USER_VA_END thật của 3 kiến trúc; platform chỉ trả hằng số.
- ELF machine constants: EM_RISCV=243, EM_AARCH64=183, EM_X86_64=62.
- Host test kiểm tra dải user không giao kernel identity, nằm trong Sv39,
  entry PML4/L0, KERNEL_SHARED_SLOTS, quyền leaf L3 AArch64 và index biên.
- File: crates/vifast-pte/src/lib.rs.

vifast-elf
- no_std, forbid unsafe; parser ELF64 little-endian executable.
- Chỉ nhận PT_LOAD; từ chối PT_INTERP/PT_DYNAMIC/PT_TLS; bỏ qua các metadata
  PT_NOTE/PT_GNU_*; kiểm tra header/phdr bounds, file range, p_offset cùng dư
  trang với p_vaddr, user VA range truyền vào, overlap theo trang, R bắt buộc,
  W^X và entry executable.
- Host test ELF hỏng: magic/class, phdr table/offset overflow, file range,
  memory range overflow, misaligned/congruence, ET_DYN, wrong machine,
  unsupported segment, segment thiếu R, hai segment chung một trang, W+X,
  overlap, bad entry, no segments.
- File: crates/vifast-elf/src/lib.rs.
- 17 host test.

vifast-user
- no_std syscall wrappers cho 3 kiến trúc.
- Dùng SyscallId + user_layout từ vifast-abi.
- File: crates/vifast-user/src/lib.rs.

vifast-root
- no_std, no_main, root task Rust ELF; `_start` asm cho 3 arch.
- Logic root: đọc BootInfo từ arg0, kiểm tra magic/version/size, in
  `BOOTINFO OK`; cấp slot tuần tự từ free_slot_start; retype TCB/endpoint/
  notification, dựng VSpace client từ data Untyped, map code RX + client/fault4
  stack RW, in `CLIENT VSPACE OK`; tạo Untyped con 16 KiB/4 object cho client;
  cấu hình client/fault4 trên VSpace riêng; in `ROOT RUST`/`ROOT READY`, Wait
  notification động; panic in `ROOT PANIC` rồi Exit.
- Linker script tại `ROOT_ELF_BASE`; x86 code-model pic + -no-pie, RISC-V
  code-model medium, AArch static; stack 4 trang + guard.
- 6.4 đã nạp root ELF vào runtime; root assembly đã bị xoá; 7.3 root dùng BootInfo; 7.5 client tách hoàn toàn.
- File: crates/vifast-root/src/main.rs.

vifast-arch-riscv64 / aarch64 / x86_64
- Chỉ primitive phần cứng:
  - UART 16550 / PL011 / COM1;
  - clock (time CSR / cntpct_el0 / rdtsc);
  - IRQ mask/unmask;
  - shutdown (SBI SRST / PSCI / isa-debug-exit).
- Có unsafe/asm/port I/O.
- Files: crates/vifast-arch-*/src/lib.rs.

vifast-platform-riscv64
- RiscvPlatform newtype.
- Sv39 bảng kernel-only tĩnh; VSpace retype copy các kernel slot.
- Trap entry/restore + trap handler.
- SBI timer.
- Page/stack user do boot Untyped + builder/FrameMap tạo.
- HSM secondary hart + SMP bootstrap.
- TLB: sfence.vma. Map EXEC gọi fence.i local + maintenance IPI remote fence.
- Files: platform.rs, trap.S, smp.S.

vifast-platform-aarch64
- Aarch64Platform newtype.
- 4-level bảng kernel-only tĩnh; VSpace retype tạo L0/L1 riêng.
- trap vector + IRQ handler.
- GICv2 + generic timer PPI 30.
- user context EL0, SP_EL0/SP_EL1.
- PSCI CPU_ON cho SMP.
- TLB: map/unmap `tlbi vae1is`; switch_root `tlbi vmalle1is` + `dsb ish` + `isb`.
- Map EXEC: `dc cvau` + `dsb ish` + `ic ivau` + `dsb ish` + `isb`; line size từ CTR_EL0.
- Files: platform.rs, trap.S, smp.S.

vifast-platform-x86_64
- X86Platform newtype.
- UEFI entry phối hợp qua PlatformInfo.
- GDT/TSS/IDT.
- PIC + PIT 100 Hz.
- Tự dựng PML4/PDPT/PD/PT boot 4 KiB; identity map 0..1 GiB.
- VSpace retype tạo PML4 + PDPT riêng; PDPT[0] dùng chung PD kernel, PDPT[3]
  dùng chung cửa sổ LAPIC supervisor-only.
- User range 64..128 GiB: boot code/stack mappings ở 64 GiB;
  code RX; stack/data RW+NX; unmap thật.
- SMEP/SMAP nếu CPU hỗ trợ; user copy dùng STAC/CLAC.
- ring3 qua iretq; CR3 chuyển sang PML4 riêng.
- ACPI MADT parse, LAPIC BSP map/enable; AP trampoline 16→32→64-bit qua
  INIT-SIPI-SIPI, vào long mode, online scheduler trên AP. Không dùng MP Services.
- Files: platform.rs, trap.S; `vifast-acpi` parser dùng chung.

vifast-kernel-riscv64 / aarch64 / x86_64-uefi
- Binary mỏng (RISC-V 81, AArch64 80, x86 UEFI 144 dòng main.rs).
- Chỉ còn:
  - global_asm boot.S (RISC-V/AArch64);
  - global_asm user.S (jump table + fault threads);
  - runtime cấp server/client ELF qua BootInfo staging; root nạp vào VSpace riêng;
  - include_bytes!(VIFAST_ROOT_ELF) + UserImage(root_elf);
  - user_image() assert offset USER_ENTRY_*;
  - gọi runtime::boot::<Platform>(...);
  - panic handler.
- Không còn page table, GIC, trap frame, context switch hay syscall dispatch.

5. KÍCH THƯỚC / ĐỘ TÁCH LỚP HIỆN TẠI
-------------------------------------
Số dòng nguồn thay đổi liên tục; không ghi số viết tay trong tài liệu.

Kích thước Kernel (host test):
- Kernel = 597624 bytes
- Object = 4632 bytes
- Tcb = 4584 bytes
- Kernel nằm trong static storage per platform, không trên boot stack.

arch primitive:
- RISC-V: khoảng 86 dòng
- AArch64: khoảng 89 dòng
- x86_64: khoảng 95 dòng

vifast-kernel + runtime + arch là các crate không unsafe (trừ primitive arch
có unsafe). Platform là nơi duy nhất được phép dùng asm/MMIO/trap frame.

6. BOOT FLOW
------------
RISC-V:
  OpenSBI -> boot.S:_start -> rust_main(hartid, dtb)
  -> vifast_runtime::boot::<RiscvPlatform>
  -> runtime tạo root task + selftest/SMP
  -> Platform::enter_user: satp, stvec, sret.

AArch64:
  QEMU virt -> boot.S:_start
  -> nếu EL2 thì drop xuống EL1
  -> rust_main(dtb)
  -> runtime boot
  -> Platform::enter_user: TTBR0/TCR/MAIR/SCTLR, eret.

x86_64:
  OVMF -> efi_main(image, system_table)
  -> runtime boot
  -> Platform::prepare_user: GDT/TSS/IDT
  -> Platform::start_timer: PIC/PIT
  -> Platform::enter_user: chuyển CR3 sang PML4 riêng, iretq ring3.

Runtime boot sequence chi tiết:
1. A::init() (console/clock)
2. A::irq_disable()
3. A::init_platform(info)
4. in banner
5. A::init_kernel() static; chạy selftest
6. in SELFTEST PASS
7. A::start_smp()
8. A::init_kernel() reset static; tạo boot Untyped 128 KiB/32 object, metadata
   Untyped, data Untyped; bảng kernel-only đã dựng trong init_platform
9. cài metadata/data Untyped vào slot 2/3
10. build_task_space(boot Untyped, user image, layout) dựng root VSpace mới:
    code RX, không stack/data; map rời 4 stack page fault/client.
    Parse root ELF cửa sổ 1 MiB và build_task_space_elf(Some(root)) thêm ELF +
    stack Rust 4 trang + guard. Root assembly + stack +0x4000 đã bị bỏ.
11. set TCB root -> VSpace mới; next_va = DATA_VA để client FrameMap hint 0
12. dựng VSpace server từ boot Untyped, code dùng chung Frame root, stack
    +0x2000; cài VSpace server vào root slot 12
13. check_kernel_mappings(root_pa) và server_pa; log BOOT UNTYPED sau server
14. A::prepare_user()
15. tạo context cho duy nhất root TCB
16. A::set_kernel(&mut kernel)
17. enqueue root, pick root, ensure_current_root() -> ROOT SWITCH 1
18. A::start_timer()
19. A::enter_user(root)

Root task sau đó tự:
- Retype endpoint/2 TCB/notification, 4 fault TCB, notification riêng;
- CapCopy endpoint, client VSpace, Untyped con, notification cho server/client;
- TcbConfigure entry/stack/VSpace/priority;
- TcbResume server/client và 4 fault thread;
- Wait trên notification riêng.

7. OBJECT MODEL VÀ CAPABILITY
-----------------------------
Giới hạn cố định:
- MAX_OBJECTS = 128
- MAX_TCBS = 16
- MAX_CAPS = 96
- MAX_QUEUE = 8
- MAX_FDS = 8
- MAX_MAPPINGS = 32
- MAX_FILES = 8

Object:
- Object { gen: u32, kind: Kind, owner: Option<ObjRef>, creator: Option<CapRef> }
- ObjRef { index: u8, gen: u32 }
- Generation tăng khi slot tái sử dụng; ObjRef cũ bị từ chối.

Kind:
- Free
- Endpoint { send: Queue, recv: Queue }
- Notification { word, waiter }
- Tcb
- Untyped { base, size, bump, page_size, max_objects, objects, reserved }
- Frame { pa, size, mapped_w: u16, mapped_x: u16 } số leaf W/X trên mọi VSpace
- VSpace { root, aux_pa, page_size, base, next_va, end, mappings }
- PageTable { root, mapped: Option<{vspace, va_base, level}> }
  // level chỉ nằm trong mapping do PageTableMap gán; mỗi bảng gắn một lần

Tcb:
- name, state, priority
- entry, stack, vspace: Option<ObjRef>
- cspace [Option<Cap>; 64]
- caller: Option<(ObjRef, epoch)>
- reply_epoch
- recv_slot
- bound_ntfn
- pending: Option<PendingSend>
- resume: Option<Message>
- fds [Option<Fd>; 8]
- queued, queue_seq

Capability:
- Cap { obj, ty, rights, badge, id: u64, root: bool, parent: Option<CapRef> }
  // root chỉ được set bởi install_cap cho Untyped khi parent=None (runtime boot);
  // CapCopy/IPC/reparent không bao giờ set root=true.
- install_cap/lookup_cap/copy_cap/delete_cap/revoke_cap/set_cap_badge
- Rights: READ=1, WRITE=2, EXEC=4, GRANT=8
- copy/derive chỉ giảm quyền
- parent tạo cây để revoke descendant
- Chỉ Endpoint/Notification mang badge
- Chuyển capability qua IPC: sender cần GRANT; receiver nhận bản copy
  không có GRANT.

Untyped/retype:
- Frame: cấp 1 page căn theo page_size; có thể map vào nhiều VSpace.
- PageTable: cấp 1 page; khi retype mapped=None; PageTableMap gán
  level và vị trí thật trong mapping.
- VSpace: retype cấp root page + aux_pa (PDPT/L1) cho x86/AArch64; zero
  các trang và gọi install_kernel. Base mặc định là user range 64..128 GiB.
- Endpoint: tiêu OBJ_ENDPOINT_BYTES = 256.
- Notification: tiêu OBJ_NOTIFICATION_BYTES = 128.
- Tcb: tiêu OBJ_TCB_BYTES = 4096 (quota accounting).
- Hết Untyped trả OutOfMemory.
- Mỗi object ghi `creator` = CapRef { addr, id } của cap đã dùng để retype;
  revoke chỉ khớp cả slot và id, nên slot tái sử dụng không kế thừa creator.
  Revoke Untyped copy chỉ hủy object tạo qua cap đó hoặc cap con của nó.
- `delete_cap` trả KResult: InvalidCap nếu không có cap, IllegalOperation nếu còn
  object/cap con, Ok nếu xoá thành công.
- Root TCB không được destroy; revoke nhóm chứa root TCB trả IllegalOperation.
- Khi TCB bị hủy, object/cap con được reparent lên cap cha còn sống gần nhất;
  nếu không còn cha, creator=parent=None và chỉ revoke root cap mới thu hồi.
- Chỉ reset bump khi không còn object nào thuộc Untyped từ bất kỳ cap nào.
- Revoke cap Untyped xoá descendant, destroy object thật (Frame unmap, TCB detach,
  Endpoint/Notification wake lỗi).
- Revoke cap không phải Untyped chỉ xoá descendant; object gốc vẫn sống.
- Chưa có free/destroy object/frame ngoài đường revoke Untyped.

8. IPC VÀ SCHEDULER
-------------------
Endpoint:
- send queue chứa TCB BlockedSend.
- recv queue chứa TCB BlockedRecv.
- Queue FIFO 8 phần tử.

Send:
- Có receiver: gán resume, đưa receiver Ready.
- Call: sender BlockedReply, lưu caller.
- Không receiver: sender BlockedSend + PendingSend + vào send queue.

Recv:
- Nếu đã có caller: trả WouldBlock (server phải Reply trước).
- Có sender: nhận PendingSend, chuyển cap nếu cần, trả message.
- Không sender: block, lưu recv_slot.

Reply:
- Chỉ một caller.
- Kiểm tra BlockedReply + reply_epoch.
- Gán resume message, đưa client Ready.

Notification:
- word OR bit.
- signal có waiter: đánh thức + giao word.
- wait: trả word hoặc block.
- poll: trả + clear.
- Không có waiter: word tích luỹ.

Chuyển cap:
- Message.cap = slot nguồn.
- Kiểm tra GRANT.
- Slot nhận phải do receiver chỉ định rõ; không fallback; slot 0/đã dùng là lỗi.
- Copy cap, bỏ GRANT.
- Nếu lỗi, đưa sender lại queue để không mất trạng thái.

Scheduler:
- Một runqueue cố định.
- Priority lớn hơn chạy trước.
- Cùng priority: FIFO queue_seq.
- pick() lấy TCB, state Running, tăng context_switches.
- yield_current() đưa Running về Ready + enqueue.
- Block state không nằm trong runqueue.
- Runtime lưu/nạp Platform::Context.
- Timer 100 Hz; binary test bật feature test-exit để sau 20 tick in PREEMPT OK và exit QEMU.
- Cả RISC-V, AArch64 và x86_64 cho hart phụ chạy ba worker.
- Worker stack A/B/C/D: `CODE_VA+0xd000` / `+0xe000` / `+0x10000` / `+0x11000`, mỗi stack 1 page; ABI
  const assert không chồng nhau và trên `DATA_VA`. Root tạo worker, cấp TCB,
  frame, endpoint cap slot 1, affinity `0b10`.

9. MEMORY / MMU
---------------
Portable generic page-table walk + reverse map:
- Page table thật là nguồn sự thật duy nhất.
- Kernel đi walk generic qua PageTableOps cho map/unmap.
- VSpace.mappings là reverse map cho revoke, không phải nguồn
  sự thật của phần cứng.
- W^X: từ chối mapping vừa WRITE vừa EXEC; mọi mapping phải có READ.
- W^X theo Frame: `mapped_w`/`mapped_x` đếm leaf W/X trên toàn bộ VSpace;
  Frame đã có W không nhận X và ngược lại, nên hai alias không lách được W^X.
  FrameMap/FrameUnmap, revoke VSpace/PageTable và destroy Frame cập nhật bộ đếm.
  Bộ đếm dùng checked_add; tràn u16 trả OutOfMemory.
- PA uniqueness: check_invariants từ chối hai Frame/PageTable/VSpace root hoặc
  aux_pa chồng dải; Untyped chỉ được lồng nhau theo owner chain và con phải
  nằm trọn trong cha. Đây là tiền đề để W^X theo Frame còn đúng khi có Untyped con.
- Map EXEC gọi PageTableOps::sync_icache(pa, size) sau khi mapping thành công:
  RISC-V fence.i; AArch64 dc cvau + dsb + ic ivau + dsb + isb; x86 no-op.
  Map RW không gọi. Host test đếm số lần gọi.
- Kiểm tra alignment, overlap, range.
- VSpace có page_size, range, next_va và reverse mappings; không còn window.
- Map thiếu bảng trung gian trả MissingPageTable; kernel không tự cấp phát.
- PageTableMap(page_table_cap, vspace_cap, va):
  kernel walk tới entry invalid đầu tiên, tự chọn level, kiểm tra toàn bộ
  table_span(level) nằm trong user_va_range rồi mới ghi PTE.
- Mỗi PageTable lưu {vspace, va_base, level}; gắn lần hai trả AlreadyMapped.
- Walk dừng ở leaf cấp cao; không ghi vào bảng kernel dùng chung.
- x86/AArch64: PML4/L0 là **riêng** cho từng VSpace; entry 0 trỏ tới
  PDPT/L1 **riêng**. PDPT[0] và PDPT[3] trỏ bảng kernel dùng chung;
  các entry user 64..127 trỏ bảng user riêng. PageTableMap đi xuyên qua entry 0
  nhưng không thay nó.
- AArch64 hiện dùng L1[0] cho MMIO thấp (GIC/PL011) và L1[1] cho RAM kernel
  0x4000_0000..; cả hai là KERNEL_SHARED_SLOTS. RISC-V copy root[0] (UART) và
  root[2] (kernel RAM); x86 copy PDPT[0] (identity 0..1 GiB) và PDPT[3]
  (LAPIC MMIO).
- Revoke Untyped: Frame trước (unmap leaf), PageTable sâu nhất trước
  (xoá entry cha + flush), cuối cùng VSpace.
- validate_user_range() walk page table thật cho từng trang: leaf phải non-kernel,
  có R, và W nếu cần ghi. Reverse map không còn dùng để validation.
- User layout hằng trong vifast-abi: code 64 GiB, data +0xa000.
  USER_ENTRY_* tại +0x00/+0x10/+0x20/+0x30/+0x40/+0x50 cho server/client/
  fault1/2/3/4. Stack tops: server +0x5000, client +0x9000, fault1/2/3
  +0x6000/+0x7000/+0x8000, fault4 +0xa000.
  Root ELF ở +0x20_0000, stack Rust 4 trang tại ROOT_ELF_STACK_BASE + guard;
  BootInfo READ-only ở ROOT_BOOTINFO_VA ngay trên stack; root builder map code + 4 stack fault/client;
  server builder map code chung + stack 3 trang từ +0x2000 tới +0x5000; data để trống cho client
  FrameMap. TcbConfigure không kiểm tra entry/stack.
- x86 dùng VA cố định; VSpace.direct đã bị xoá.
- Frame object metadata PA/size + mapped_w/mapped_x; retype Frame/PageTable zero page.
- Chưa có page-table page allocator; đã có range VA allocator; ASID 12-bit gán
  theo VSpace, x86 PCID chỉ bật khi CPUID hỗ trợ (QEMU TCG fallback CR3).

RISC-V64:
- Sv39 root boot tĩnh; kernel identity 0x8000_0000..0x803f_ffff; UART 0x1000_0000.
- KERNEL_SHARED_SLOTS = root[0] (UART/device) và root[2] (kernel RAM).
- User code 64 GiB: RX U (Frame từ boot Untyped, dùng chung server).
- Root VSpace: fault1/2/3 stacks +0x5000/+0x6000/+0x7000; server/client stacks
  để trống; root Rust stack 4 trang + guard ở ROOT_ELF_STACK_BASE.
- VSpace client: code chung RX, client stack +0x8000, fault4 stack +0x9000.
- VSpace server: +0x2000 RW U, code dùng chung Frame.
- Data 64 GiB+0xa000: để trống trong root VSpace; client FrameMap RW U.
- TLB: sfence.vma. Map EXEC gọi fence.i local + maintenance IPI remote fence.
- Trap dùng kernel trap stack qua sscratch; sscratch=0 khi ở kernel.
- SUM=0; read_user/write_user bật SUM tạm thời quanh copy.

AArch64:
- 4-level page table boot tĩnh; VSpace retype tạo L0/L1 riêng.
- Identity/MMIO dùng L1[0] (GIC 0x0800_0000, PL011 0x0900_0000) và L1[1]
  (RAM kernel 0x4000_0000..); KERNEL_SHARED_SLOTS = [0, 1].
- User code 64 GiB: EL0 RO + X (Frame từ boot Untyped, dùng chung server).
- Root VSpace: fault1/2/3 stacks +0x5000/+0x6000/+0x7000; server/client stacks
  để trống; root Rust stack 4 trang + guard.
- VSpace client: code chung RO+X, client stack +0x8000, fault4 stack +0x9000.
- VSpace server: +0x2000 RW + NX, code dùng chung Frame.
- Data 64 GiB+0xa000: để trống trong root VSpace; client FrameMap RW + NX.
- TLB: map/unmap `tlbi vae1is`; switch_root `tlbi vmalle1is` + `dsb ish` + `isb`.
- Map EXEC: `dc cvau` + `dsb ish` + `ic ivau` + `dsb ish` + `isb`; line size từ CTR_EL0.
- Trap stack SP_EL1; user stack SP_EL0.
- PAN nếu CPU hỗ trợ; read/write user tạm clear PAN.

x86_64:
- Tự dựng PML4/PDPT/PD/PT boot 4 KiB; identity map 0..1 GiB.
- User code 64 GiB: RX + U (Frame từ boot Untyped, dùng chung server).
- Root VSpace: fault1/2/3 stacks +0x5000/+0x6000/+0x7000; server/client stacks
  để trống; root Rust stack 4 trang + guard.
- VSpace client: code chung RO+X, client stack +0x8000, fault4 stack +0x9000.
- VSpace server: +0x2000 RW + NX + U, code dùng chung Frame.
- Data 64 GiB+0xa000: để trống trong root VSpace; client FrameMap RW + NX + U.
- FrameMap/FrameUnmap đi generic walk, không còn platform map riêng.
- CR3 chuyển sang PML4 riêng trước iretq.
- PML4[0] và PDPT là riêng mỗi VSpace; PDPT[0] trỏ PD kernel dùng chung,
  PDPT[3] trỏ LAPIC supervisor-only
  cho identity 0..1 GiB; user 64..128 GiB dùng PDPT[64..127] trỏ bảng riêng.
- SMEP/SMAP bật nếu CPUID hỗ trợ; EFER.NXE + tách 2 MiB để đánh dấu NX cho static data kernel (S2).
- I-cache và D-cache coherent; sync_icache là no-op.
- Cả ba kiến trúc cho CPU phụ chạy scheduler + ba worker; x86 dùng D4 idle
  `sti; hlt; cli` và LAPIC timer/IPI vector.

10. VFS V2 / VIFAST-FS
------------------------
User-space VFS server: `crates/vifast-server/src/main.rs`; filesystem core:
`crates/vifast-fs/src/lib.rs`; host image tool: `crates/vifast-fsutil`.

- `vifast-fs` là `no_std`, `forbid(unsafe_code)`, không dùng `alloc`; toàn bộ
  buffer cố định. Server giữ `Fs` trong `.bss`, không đặt journal 8 KiB trên
  stack; stack server là 3 trang `CODE_VA+0x2000..+0x5000`.
- Layout 4 MiB: LBA0 legacy `VF01` (chỉ đọc để migrate), LBA1 superblock
  `VF02`, LBA2..3 bitmap, LBA4..11 64 inode x 64 byte, LBA12..29 journal
  1 header + 16 data + 1 commit, LBA30.. data. Inode có 12 block trực tiếp,
  file tối đa 6 KiB; thư mục tối đa 12 block, entry 32 byte, tên tối đa 28.
- Journal write-ahead: mỗi thao tác là một transaction tối đa 16 block; ghi
  data journal, header, commit một sector có CRC32, ghi target, rồi xóa commit.
  Mount replay commit hợp lệ; commit hỏng bị bỏ, dữ liệu cũ giữ nguyên.
  Giả định ghi một sector 512 byte là nguyên tử; chưa có flush/barrier phần cứng.
- Protocol giữ OPEN/READ/WRITE/CLOSE/STAT và thêm MKDIR/READDIR/UNLINK/RENAME/
  TRUNCATE/FSYNC/FSCK. Mã lỗi FS nằm trong `vifast_abi::vfs`; path tối đa 128
  byte, dữ liệu path/write đi qua IPC buffer. Server trả fd/size cho OPEN,
  read/write theo byte, READDIR trả tên qua IPC buffer.
- Quyền: uid lấy từ **badge register** của message, không lấy `value`; badge 0
  không có danh tính và bị từ chối, badge `0xFFFF` là superuser duy nhất.
  File/thư mục mới có uid người tạo, mode owner rw + other r. Đọc cần r,
  ghi/truncate cần w; tạo/xóa/rename trong thư mục cần w trên thư mục cha.
  `UNLINK` thư mục chỉ khi rỗng; `RENAME` không đưa thư mục vào cây con của nó.
- Block syscall 43/44 bắt buộc cap `ObjType::BlockDevice`; runtime cài cap
  vào root slot từ BootInfo v3, root chỉ copy cho server. Client thử
  BlockWrite không cap và in `BLOCK DENIED OK`.
- Server tự format đĩa trắng (`FS FORMAT OK`), mount đĩa hợp lệ
  (`FS PERSIST OK`), migrate `VF01` sang `/scratch` (`FS MIGRATE OK`), và
  replay/torn journal (`FS RECOVER OK (replayed=1|0)`). FSCK sau demo in
  `FS FSCK OK`; `FS V2 OK`, `FS PERM OK` là điều kiện harness.
- `vifast-fsutil` (host) có mkfs/put/get/ls/fsck/inject-journal --valid|--torn;
  `scripts/test-persist.py` chạy 6 phase cho cả 3 kiến trúc.
- Kernel ramfs `crates/vifast-kernel/src/fs.rs` vẫn chỉ là fallback/selftest,
  không chứa logic FS v2.

11. ABI / SYSCALL HIỆN CÓ
-------------------------
Số syscall chính:
### BEGIN GENERATED syscalls
0 Send — implemented
1 Call — implemented
2 Recv — implemented
3 Reply — implemented
4 ReplyRecv — implemented
5 Signal — implemented
6 Wait — implemented
7 Poll — implemented
8 Yield — implemented
9 Retype — implemented
10 CapCopy — implemented
11 CapMint — implemented
12 CNodeMint — placeholder (NotImplemented)
13 CapDelete — placeholder (NotImplemented)
14 CapRevoke — implemented
15 TcbConfigure — implemented
16 TcbResume — implemented
17 TcbSuspend — placeholder (NotImplemented)
18 TcbRestart — placeholder (NotImplemented)
19 TcbSetPriority — placeholder (NotImplemented)
20 TcbSetAffinity — implemented
21 TcbBindNotification — placeholder (NotImplemented)
22 SchedContextConfigure — placeholder (NotImplemented)
23 SchedContextBind — placeholder (NotImplemented)
24 PageTableMap — implemented
25 FrameMap — implemented
26 FrameUnmap — implemented
27 IoMap — implemented
28 IoUnmap — implemented
29 IrqGet — placeholder (NotImplemented)
30 IrqSetNotification — placeholder (NotImplemented)
31 IrqAck — placeholder (NotImplemented)
32 DebugWrite — implemented
33 GetTime — implemented
34 Exit — implemented
35 Open — implemented
36 Read — implemented
37 Write — implemented
38 Close — implemented
39 Stat — implemented
40 Seek — implemented
41 TcbSetIpcBuffer — implemented
42 FrameWrite — implemented
43 BlockRead — implemented
44 BlockWrite — implemented
### END GENERATED syscalls

- `BlockRead`/`BlockWrite` (43/44): `a0` = cap slot `BlockDevice`,
  `a1` = LBA, `a2` = user buffer 512 byte; thiếu/sai cap trả `InvalidCap`.
- `CapMint` (11): `a0` source slot, `a1` TCB đích, `a2` slot đích,
  `a3` rights (chỉ giảm), `a4` badge; chỉ Endpoint/Notification **và** source
  phải có badge 0. Server nhận uid từ badge register (RISC-V a7, AArch64 x7,
  x86_64 r10), không dùng `value`.
- `GetTime` (33) trả thời gian toàn hệ thống theo tick 10 ms của BSP
  (`ticks[0]`), đơn điệu khi task chuyển hart; `ticks[hart]` chỉ dùng nội bộ
  cho timer/preempt/SMP harness.
- Badge 0 = không danh tính; `0xFFFF` = superuser; `CapCopy` giữ badge,
  `CapMint` chỉ nhận source badge 0.

Quy ước register:
RISC-V:
- number a7, args a0..a5
- return a0=error, a1=value, a2=label, a3..a6=data
- ecall

AArch64:
- number x8, args x0..x5
- return x0=error, x1=value, x2=label, x3..x6=data
- svc #0

x86_64:
- number rax, args rdi/rsi/rdx/rcx/r8/r9
- return rax=error, rdi=value, rsi=label, rdx/rcx/r8/r9=data
- int 0x80

Error chính:
Ok, InvalidCap, WrongType, NoRights, SlotOccupied, OutOfMemory,
InvalidArg, Alignment, AlreadyMapped, NotMapped, NoCaller,
ObjectDestroyed, MissingPageTable, IllegalOperation, NotImplemented,
WouldBlock, NotFound.

12. ROOT TASK VÀ USER DEMO
----------------------------
Runtime chỉ tạo duy nhất root TCB. Root task nhận `BootInfo` READ-only qua arg0
tại `ROOT_BOOTINFO_VA`, chứa:
- root/meta/data/server VSpace/Untyped slot do runtime cài;
- `user_code_first/count`: Frame cap code user.S READ|EXEC, không WRITE/GRANT;
- `free_slot_start/free_slot_end`: dải slot root dùng tuần tự cho object mới.
- boot Untyped 128 KiB/32 object chỉ kernel giữ; không cấp cho user

Root task dùng syscall, cấp slot tuần tự từ `free_slot_start`:
- Retype(metadata Untyped) -> Endpoint, TCB server/client, notification root
- CapCopy/CapMint:
  - endpoint -> server slot 1 READ
  - CapMint endpoint badge 1 -> client slot 1 WRITE; badge 2 -> slot 6 WRITE
  - BlockDevice cap (BootInfo v3 slot) -> server slot 2 READ|WRITE
  - client VSpace (dựng bên dưới) -> client slot 2 WRITE
  - Untyped con 16 KiB/4 object -> client slot 3 WRITE
  - notification -> client slot 5 READ|WRITE|GRANT
- TcbConfigure(server, entry, stack 0x1000005000, VSpace BootInfo.server, priority 5)
- Retype thêm 4 fault TCB (priority 6) và notification root
- TcbConfigure(client, entry, stack 0x1000009000, VSpace client, priority 5)
- TcbResume(server/client), fault1/2/3 root VSpace, fault4 client VSpace
- Wait trên notification root.
- Retype(data Untyped) → VSpace client; lặp PageTableMap + FrameMap cho code
  BootInfo RX; retype Frame client stack +0x8000 và fault4 stack +0x9000, map
  RW. Code, stack và DATA_VA (+0xa000) cùng bảng cấp cuối; log CLIENT VSPACE OK.
- Tạo Untyped con 16 KiB/4 object từ data Untyped; CapCopy con cho client slot
  3. TcbConfigure client với VSpace client; fault4 VSpace client, stack +0xa000.
- Fault4 ghi vào ROOT_ELF_STACK_BASE (không map trong VSpace client), sinh
  USER FAULT thứ tư tại 0x1000301000.
- Fault1 load 0x8000_0000; fault2 chạy lệnh đặc quyền; fault3 ghi vào stack
  server 0x1000002000; fault4 trong VSpace client ghi root stack 0x1000301000
  -> cả bốn log USER FAULT.
- Server VSpace riêng, code dùng chung Frame root, stack riêng +0x2000.
  Client dùng VSpace riêng và Untyped con 16 KiB/4 object từ 7.5; không còn cap
  tới root VSpace hay data Untyped của root.

Server A:
- cap endpoint READ.
- DebugWrite "Server" + "VFS SERVER OK".
- loop ReplyRecv(endpoint, recv_slot=4); nhánh label 0x1234 in IPC buffer,
  Signal(notification slot 4); nhánh VFS OPEN/READ/WRITE/CLOSE giữ bảng file
  static và trả reply label/data; panic nếu lỗi hệ thống.

Client B:
- cap endpoint WRITE, VSpace WRITE, data Untyped WRITE,
  notification READ|WRITE|GRANT.
- Retype data Untyped -> Frame slot 4; Frame object creator = client slot 3.
  Client CapRevoke(slot 3) chỉ hủy Frame do client tạo, không hủy object
  root/server tạo qua cap gốc của cùng Untyped.
- FrameMap, DebugWrite "Mapped".
- Gọi VFS IPC: Open /motd / Read / DebugWrite "ViFastOS ramfs".
- Gọi VFS IPC: Open /scratch O_CREATE|O_RDWR, Write qua IPC buffer, Close,
  Open lại, Read, DebugWrite "ViFastOS write", Close, in "VFS OK".
- FrameUnmap(vspace slot 2, VA, frame slot 4); thiếu cap Frame hoặc sai PA
  trả InvalidCap/NoRights, không thể phá mapping của task khác.
- DebugWrite "Client".
- loop Call với cap notification ở lần đầu; các Call sau không chuyển cap.
- Poll notification, in "Cap OK"/"Cap BAD", "Reply OK"/"Reply BAD".

Fault thread:
- DebugWrite "Fault"/"Fault2"; fault3 DebugWrite "Isolation".
- fault1 load từ 0x8000_0000; fault2 chạy lệnh đặc quyền; fault3 ghi vào
  stack server 0x1000002000 (không map trong root VSpace).
- kernel in "USER FAULT", chuyển TCB sang Faulted, chọn thread kế tiếp.
- QEMU vẫn in "BOOTINFO OK", "CLIENT VSPACE OK", "Server", "Reply OK",
  "Zero OK", "Fault4", "VFS SERVER OK", "VFS OK" và "ROOT SWITCH n>1".

Cap transfer:
- First Call: client gửi notification cap có GRANT vào server slot 4.
- Server nhận bản copy không GRANT, Signal, Reply.
- Client Poll nhận word.

13. SMP HIỆN TẠI
---------------
- RISC-V: SBI HSM start hart 1; timer SBI, BKL per-hart, idle WFI với
  SIE=0, reschedule IPI chỉ khi idle; ba worker affinity hart 1.
- AArch64: PSCI CPU_ON CPU1; GIC banked/timer riêng, DAIF.I=1, WFI polling
  `GICC_IAR`, per-hart BKL/current, TTBR0 theo VSpace; ba worker affinity
  CPU1 và reschedule SGI chỉ khi idle.
- x86_64: AP online qua trampoline 16→32→64-bit, INIT-SIPI-SIPI, stage 1..4;
  TSS/GS riêng, BKL + `pick_on(1)`, timer LAPIC calibrated 100 Hz; D4 idle
  `sti; hlt; cli`, vector 0x30/0x31 ở ring 0 chỉ trong cửa sổ idle.
- Scheduler dùng `pick_on(hart)` trên cả ba kiến trúc; x86 AP chạy worker
  thật và timer preempt ở ring 3.
- `ticks[hart]` là bộ đếm theo hart; `GetTime` đơn điệu toàn hệ thống dùng
  `ticks[0]` BSP (task chuyển hart không thấy lùi/nhảy); test-exit/PREEMPT OK
  và `B` cũng dùng `ticks[0]`. `SMP TIMER RATE (ap=T, bsp=B)` in trên cả 3
  kiến trúc và kiểm `B >= 5`, `B/2 <= T <= 2B`.
- BigKernelLock đã là ticket lock FIFO (`next`/`serving`) + RAII guard, bọc
  trap/syscall/IRQ + x86 exception/page-fault. Spin chờ khóa gọi
  `poll_maintenance()` để hart phụ xử lý IPI TLB/fence và ACK; assert IRQ tắt
  trước khi lấy khóa; assert BKL đã nhả trước khi về user.
- RISC-V CpuState nằm trong `sscratch` khi user, `tp` chỉ dùng trong kernel;
  x86 dùng `swapgs`/`IA32_KERNEL_GS_BASE`; AArch64 dùng `TPIDR_EL1` (user
  không ghi được `TPIDR_EL1`). User clobber tp/GS/TPIDR_EL0 được test bằng
  `PERCPU CLOBBER OK`.
- `current[hart]`, `ACTIVE_ROOT`, `ACTIVE_ASID` nằm trong CpuState; kernel có
  `MAX_HARTS=4`, `pick_on(hart)`/`current_on(hart)`; runqueue vẫn global.
- BootInfo v3 có `sched_harts: u32` và block-device slot; cả ba kiến trúc 0b11. Root tạo worker khi
  bit 1 bật; D1 cho RISC-V/AArch64 (WFI với interrupt bị che), D4 cho x86
  (cửa sổ `sti; hlt; cli`).
- RISC-V map EXEC gọi `fence.i` local + IPI maintenance remote fence; AArch64
  `ic ivau` + `dsb ish` đã broadcast.
- Mailbox maintenance đối xứng theo hart đích: `MAINT_SEQ/ACK/KIND`, TLB,
  I-cache, reschedule; sender giữ BKL và bỏ qua hart chưa online.
- Hart 1 RISC-V có `SCHED_READY`, timer SBI, vòng idle lấy BKL → `pick_on(1)`
  → nhả BKL → WFI khi SIE=0 → đọc `sip`; timer/IPI wakeups in `SMP IDLE OK`.
- Bốn worker affinity hart 1 (A/B/C/D), stack riêng; D làm demo TLB, A/B/C
  nhận diện qua endpoint label; in `SMP WORKERS OK (a=50, b=50, c=1)` và `SMP SCHED OK (hart0=N, hart1=M,
  resched_sent=S, resched_wake=W, idle_entries=I)`; post-worker timer qua
  `SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)`.
  affinity qua syscall 20 `TcbSetAffinity`.
- WFI với SIE=0 (sie.SSIE|STIE bật) đã ổn định 10/10 trong vòng 07; không mở
  interrupt trong kernel (D1).
- `static mut` toàn cục vẫn còn: page table, pool, CONTEXTS, KERNEL_PTR,
  KERNEL_STORAGE, FILES/FILE_COUNT, VirtIO QUEUE/BASE/READY.
  Chúng được bảo vệ bởi BKL khi đi qua trap, nhưng chưa phải per-CPU;
  server/virtio cần khóa riêng khi SMP.
- Chưa có per-hart runqueue; per-VA TLB shootdown đã có mask harts_loaded,
  `TLB_VA` mailbox và fallback full ASID.
- Panic-halt IPI đã dừng hart khác trên cả 3 kiến trúc; RISC-V panic dùng
  `sifive_test` mã `(code<<16)|0x3333`, QEMU rc=1; AArch64 rc=1; x86 rc=7.

14. TEST VÀ TIÊU CHÍ PASS
-------------------------
Lệnh:
    ./scripts/build.sh
    cargo test -p vifast-kernel
    cargo test -p vifast-arch
    cargo test -p vifast-pte
    cargo test -p vifast-elf
    cargo test -p vifast-runtime
    cargo test -p vifast-user
    cargo test -p vifast-acpi
    cargo test -p vifast-fs
    cargo test -p vifast-fsutil
    cargo fmt --all -- --check
    python3 scripts/test-qemu.py
    python3 scripts/test-qemu.py --arch riscv64 --feature test-boot-handoff
    python3 scripts/test-persist.py
    python3 scripts/test-harness-spec.py
    python3 scripts/gen-docs.py --check
    python3 scripts/check-docs.py

Host test hiện có: xem vùng `tests` sinh tự động. Danh sách nhóm kiểm tra:
- kernel/object sizes
- self-test
- capability rights monotonic
- revoke descendants
- VSpace W^X/overlap/unmap
- frame alignment
- syscall từ chối cap sai type/đã revoke và W^X
- chuyển cap yêu cầu slot đích rõ ràng, không fallback
- user pointer validation cho DebugWrite/Open/Read/Write
- Untyped byte accounting + OutOfMemory
- revoke Untyped destroy Frame + xoá mapping cũ
- revoke Untyped detach TCB đang BlockedRecv
- max_objects limit
- Untyped con: range/quota, sibling PA uniqueness, frame nằm trong dải con,
  revoke cha/con, nested child, cap không root, reparent TCB
- cap copy giữ parent chain
- Test TcbConfigure cũ đã xóa; thay bằng `tcb_configuration_sets_context_and_resumes`
- TcbResume từ chối TCB chưa configure
- Yield thật qua switch_blocked
- checked_add/overflow + buffer vắt qua hai page khác quyền
- self-revoke Untyped bị từ chối khi TCB hiện tại là con
- retype Frame/PageTable gọi zero_page qua PageTableOps
- generic page-table walk xoá leaf trên bảng giả lập host
- TCB quota 16 đầy trả OutOfMemory
- walk safety: user range/missing table/high leaf/valid PTE/snapshot
- PageTableMap gán level 1/2/3, FrameMap qua bảng mới
- PageTableMap từ chối bảng vượt user range, snapshot không đổi
- PageTableMap không thay các kernel slot trong PDPT/L1
- revoke bản copy Untyped không hủy object của cap gốc
- root revoke thu hồi cả object của cap con
- TCB bị hủy: `multiple_dying_tcbs_reparent_through_dead_intermediate_cap`,
  `root_flag_does_not_propagate_to_copies_or_ipc`,
  `root_tcb_cannot_be_destroyed_by_untyped_revoke`
- delete Untyped cap bị từ chối khi còn object/cap con; delete trả KResult
- build_task_space map code RX/stack RW/data RW, rollback khi thiếu trang
- build_task_space_shared_code map cùng Frame code vào root + server VSpace
- build_task_space_elf: PT_LOAD R/RX/RW, BSS zero, segment giữa trang,
  stack 4 trang + guard, thêm vào VSpace có sẵn và rollback khi thiếu trang;
  rollback so sánh snapshot toàn cây page table
- FrameUnmap đòi Frame cap đúng PA; chỉ VSpace WRITE hoặc sai frame bị từ chối
- FrameMap kiểm tra quyền mapping ⊆ quyền Frame cap; READ|EXEC cap không map RW
- W^X theo Frame: RW rồi RX cùng Frame ở VA/VSpace khác bị từ chối, snapshot
  page table không đổi; unmap/revoke VSpace giải phóng mapped_w/mapped_x
- mapping bắt buộc READ; mask rỗng/W-only/X-only trả InvalidArg
- shared code RX root/server: mapped_x=2, mapped_w=0
- PA uniqueness: Frame-Frame và PageTable-VSpace trùng dải bị từ chối;
  bố cục Frame/PageTable/VSpace rời nhau được chấp nhận
- FrameMap EXEC gọi sync_icache; map RW không gọi
- mapped_w/mapped_x tràn u16 trả OutOfMemory, không quay vòng
- PteFakeOps tách riêng RISC-V/x86_64/AArch64, chạy map/unmap,
  PageTableMap, validate, build; build không data rồi FrameMap tại +0xa000
- validate_user_range walk leaf, user bit, R/W, cross-page, high leaf bị từ chối
- hai VSpace dùng chung Frame: revoke Frame xoá leaf ở cả hai
- VSpace/PageTable cross-Untyped, kernel slots và retype page shortage
- revoke Untyped unmap Frame trước, rồi PageTable sâu → nông, không còn PTE
  trỏ tới trang đã free.
- cross-Untyped: revoke PageTable làm sạch reverse map và validate_user_range;
- cross-Untyped: revoke VSpace detach/zero PageTable, cho phép gắn lại.

Boot assert (không tính vào host test):
- x86 LoadedImage base/end nằm trong 0..1 GiB;
- RISC-V/AArch64 kernel image end + metadata/data/boot Untyped nằm trong kernel accessible range;
- walk leaf thật từ root: non-user, X cho trap entry, W cho stack/storage;
- offset USER_ENTRY_* khớp jump table trong user_image();
- boot Untyped còn ít nhất 2 trang + 2 object cho BootInfo.

`vifast-pte` host test:
- roundtrip/encode/decode 3 format;
- index biên;
- quyền R/W/X, RISC-V W-thiếu-R bị từ chối;
- AArch64 leaf L3 RO+X, RW+NX, có/không bit EL0 và leaf_is_user;
- dải user 3 kiến trúc không giao kernel identity;
- RISC-V < 256 GiB; x86 dùng chung PDPT[0] (identity) và PDPT[3] (LAPIC),
  AArch64 dùng chung L1[0..1];
- table_span từng level và biên.

QEMU test:
- RISC-V: -machine virt -smp 2 -bios default + virtio-blk-device
- AArch64: -machine virt,gic-version=2 -cpu max -smp 2
- x86_64: OVMF + -cpu max + -smp 2 + isa-debug-exit
- `test-qemu.py` tạo disk mới trong thư mục tạm riêng cho mỗi lần chạy,
  không dùng `/tmp/vifast-*.disk` cố định; vì vậy output chính chỉ cần
  `FS DISK OK`, không cần `FS PERSIST OK`.
- `scripts/test-persist.py` kiểm tra 6 phase: disk trắng format; host put
  nonce rồi guest đọc/ghi đảo; migrate `VF01`; replay journal hợp lệ; torn
  commit giữ dữ liệu cũ; superblock hỏng format lại; áp dụng cả 3 kiến trúc.
  Host `vifast-fsutil` có `mkfs/put/get/ls/fsck/inject-journal --valid|--torn`.

Điều kiện PASS (khớp `scripts/test-qemu.py`):
### BEGIN GENERATED pass
Điều kiện chung (mọi kiến trúc):
- `SELFTEST PASS` — kernel host self-test passed in guest
- `booting on` — architecture banner printed
- `BOOT UNTYPED` — boot untyped accounting printed
- `PREEMPT OK` — timer preemption progressed
- `Isolation` — fault isolation demo ran
- `Server` — user VFS server ran
- `Client` — user VFS client ran
- `Mapped` — client VSpace mapping succeeded
- `Cap OK` — capability transfer demo succeeded
- `ViFastOS ramfs` — ramfs read path ran
- `ViFastOS write` — ramfs write path ran
- `FS V2 OK` — FS v2 namespace/data demo completed
- `FS FSCK OK` — server fsck passed after demo
- `FS PERM OK` — badge-based FS permissions behaved
- `BADGE SPOOF DENIED OK` — rebadging and badge spoofing were denied
- `IPC BUF OK` — IPC buffer transfer succeeded
- `BLOCK DENIED OK` — client was denied BlockWrite without a device cap
- `Reply OK` — reply path succeeded
- `ROOT RUST` — root Rust ELF entered user mode
- `BOOTINFO OK` — root validated BootInfo v3
- `RUST MODULES OK` — server/client Rust ELF modules loaded
- `CLIENT VSPACE OK` — client VSpace built
- `FAULT EP OK` — fault endpoint delivered all fault records
- `ROOT READY` — root finished initialisation
- `Zero OK` — zero page / BSS path verified
- `BLOCK OK` — block device backend responded
- `FS DISK OK` — filesystem disk path responded

Mỗi dòng sau đúng một lần:
- `VFS SERVER OK` — exactly once
- `VFS OK` — exactly once
- `VFS STAT OK` — exactly once
- `IOMMU OK` — exactly once
- `PERCPU CLOBBER OK` — exactly once
- `TIME MONOTONIC OK` — exactly once
- `TLB REMAP OK` — exactly once

Cấm xuất hiện:
- `PERCPU CLOBBER BAD` — forbidden
- `VFS BAD` — forbidden
- `ROOT PANIC` — forbidden
- `STALL` — forbidden
- `BOOT HANDOFF FAIL` — forbidden
- `KERNEL IRQ OUTSIDE IDLE` — forbidden
- `AP PIC IRQ` — forbidden
- `BLOCK DENIED BAD` — forbidden
- `FS CORRUPT` — forbidden
- `FS V2 BAD` — forbidden
- `FS PERM BAD` — forbidden
- `BADGE SPOOF BAD` — forbidden
- `TIME MONOTONIC BAD` — forbidden
- `TLB REMAP BAD` — forbidden

Exit code:
- riscv64: 0
- aarch64: 0
- x86_64: 3

Theo kiến trúc:
- riscv64:
  - `SMP OK` — both harts online
  - `ipi=1` — maintenance IPI acknowledged
  - `SMP TIMER RATE (ap=T, bsp=B)` — independent BSP and AP timer rates
  - `SMP TIMER OK (hart1_ticks=T)` — secondary timer ran, exactly once
  - `SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)` — idle accounting, exactly once
  - `SMP PREEMPT OK (user_ticks=U, preempts=P)` — secondary timer preempted a busy user task, exactly once
  - `SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)` — secondary scheduling, exactly once
  - `SMP WORKERS OK (a=50, b=50, c=1)` — both yield workers and busy worker completed
  - `SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)` — post-worker idle timer proof
  - `TLB STATS (va_flush=2, ipi=1, skipped=1)` — per-VA TLB accounting
  - 4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000
  - ROOT SWITCH n > 1
  - N > 1, M >= 100, W > 0, W <= S, S <= I
  - U >= 3, P >= 1, U <= T
  - B >= 5 and B/2 <= T <= 2B
  - Tp >= 5, Wtp >= Tp - 2, Ip >= Tp - 2
- aarch64:
  - `SMP OK` — both CPUs online
  - `ipi=1` — maintenance IPI acknowledged
  - `SMP TIMER RATE (ap=T, bsp=B)` — independent BSP and AP timer rates
  - `SMP TIMER OK (hart1_ticks=T)` — secondary timer ran, exactly once
  - `SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)` — idle accounting, exactly once
  - `SMP PREEMPT OK (user_ticks=U, preempts=P)` — secondary timer preempted a busy user task, exactly once
  - `SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)` — secondary scheduling, exactly once
  - `SMP WORKERS OK (a=50, b=50, c=1)` — both yield workers and busy worker completed
  - `SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)` — post-worker idle timer proof
  - `TLB STATS (va_flush=2, ipi=0, skipped=0)` — per-VA TLB accounting
  - 4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000
  - ROOT SWITCH n > 1
  - T > 0, I >= 1
  - N > 1, M >= 100, W > 0, W <= S, S <= I
  - U >= 3, P >= 1, U <= T
  - B >= 5 and B/2 <= T <= 2B
  - Tp >= 5, Wtp >= Tp - 2, Ip >= Tp - 2
- x86_64:
  - `KERNEL DATA NX OK` — kernel data is non-executable
  - `ACPI MADT OK (cpus=C, bsp=ID, lapic=0x...)` — ACPI MADT parsed
  - `LAPIC OK` — BSP local APIC enabled
  - `AP TRAMPOLINE OK (page=0x...)` — AP trampoline allocated below 1 MiB
  - `AP REAL MODE OK (apic=...)` — AP executed real-mode trampoline
  - `AP LONG MODE OK (cpu=1)` — AP entered long mode
  - `SMP OK` — both CPUs online
  - `ipi=1` — AP maintenance IPI acknowledged
  - `SMP TIMER OK (hart1_ticks=T)` — AP LAPIC timer ran, exactly once
  - `SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)` — AP idle accounting, exactly once
  - `SMP TIMER RATE (ap=T, bsp=B)` — AP and BSP timer rates agree, exactly once
  - `SMP IPI COUNT (sent=X, seen=Y)` — AP maintenance IPI accounting, exactly once
  - `SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)` — AP scheduling, exactly once
  - `SMP WORKERS OK (a=50, b=50, c=1)` — both yield workers and busy worker completed
  - `SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)` — post-worker AP idle proof
  - `TLB STATS (va_flush=2, ipi=1, skipped=1)` — per-VA TLB accounting
  - 4 USER FAULT records with 0x80000000, 0x1000002000, 0x1000301000
  - ROOT SWITCH n > 1
  - ACPI MADT cpus == 2
  - T > 0, I >= 1
  - 1 <= Y <= X, Wt <= T, Ws <= Y
  - N > 1, M >= 100, W > 0, W <= S, S <= I
  - U >= 3, P >= 1, U <= T
  - B >= 5 and B/2 <= T <= 2B
  - Tp >= 5, Wtp >= Tp - 2, Ip >= Tp - 2

Thứ tự bắt buộc:
- x86_64: `AP REAL MODE OK` trước `AP LONG MODE OK`
- x86_64: `AP LONG MODE OK` trước `SMP OK`
### END GENERATED pass


Ghi chú: `FS PERSIST OK` không được `test-qemu.py` bắt buộc vì disk mới;
`scripts/test-persist.py` kiểm tra 2 lần boot với nonce duy nhất và test âm.


15. CÁC THAY ĐỔI HARDENING GẦN NHẤT
-------------------------------------
- Task 4 (lịch sử vòng 02): BigKernelLock trong `vifast-arch::bkl`, guard bọc
  trap/syscall/IRQ entry; hiện đã bật worker hart 1 RISC-V.
- Git repo đã có; commit hardening và root-task.
- RISC-V dùng kernel trap stack qua sscratch; sscratch=0 khi kernel;
  SUM=0, chỉ bật SUM quanh copy user.
- x86_64 tự dựng PML4/page table 4 KiB; bỏ VSpace.direct; code RX,
  data/stack NX; unmap thật; SMEP/SMAP nếu CPU hỗ trợ.
- AArch64 bật PAN nếu CPU hỗ trợ.
- User layout chung 64..128 GiB trong vifast-abi/vifast-pte; x86 identity map
  0..1 GiB không còn chồng với user range; vùng boot 2 MiB gồm các leaf 4 KiB.
- validate_user_range() walk page table thật cho mọi syscall có con trỏ.
- Untyped accounting cho Endpoint/Notification/Tcb; OutOfMemory khi hết.
- Root task + Retype/CapCopy/TcbConfigure/TcbResume; runtime không còn
  tạo TCB A/B hoặc cài cap demo.
- User demo chỉ chuyển notification cap ở Call đầu tiên.
- ObjRef/Object generation u32.
- Xoá legacy multiboot crate.
- vifast-user crate với syscall wrapper 3 kiến trúc.
- Revoke Untyped destroy object thật: unmap Frame, detach TCB, wake
  Endpoint/Notification bằng ObjectDestroyed, free object + tăng gen, rồi reset
  bump/objects.
- Mỗi Untyped có `max_objects`/`objects`/`reserved`; retype kiểm tra cả byte
  lẫn `objects + reserved + 1`; Untyped con giữ quota của con trong reserved.
- Yield thật qua `switch_blocked`; root block bằng `Wait` trên notification riêng.
- User fault tối giản: log `USER FAULT`, suspend TCB Faulted, chọn thread khác,
  không panic.
- `test-exit` là feature của binary test; runtime production không còn exit cứng
  sau 20 tick.
- Kernel fault từ S-mode/current EL/kernel error-code panic; user fault vẫn suspend.
- Retype Frame/PageTable zero page qua PageTableOps; user test ghi/revoke/retype/đọc = Zero OK.
- AArch64 từng có decode riêng trả leaf READ-only, làm Read của user fail;
  platform giờ chỉ gọi vifast-pte cho encode/decode/index/leaf_is_user.
- PteFakeOps dùng encoder/decoder thật của cả 3 format trong host test, tách
  ba test theo format để lỗi chỉ rõ kiến trúc.
- 4.3: runtime dựng root VSpace từ boot Untyped, cài slot 1, check kernel
  mappings trên root mới; 4.4 thay set_active_root bằng current_root + switch_root.
- 4.3b: FrameUnmap đòi thêm Frame cap đúng PA; VSpace WRITE một mình không
  thể gỡ mapping của frame không sở hữu.
- 4.4: Context không cache root; runtime đọc root từ tcb.vspace mỗi lần restore,
  switch CR3/TTBR0/satp + full flush khi khác, assert loaded root khi copy user.
  `ROOT SWITCH n` in khi test-exit; n=1 khi mọi task còn chung root.
- 4.5: server có VSpace riêng từ boot Untyped, code dùng chung Frame root,
  stack riêng +0x2000; root VSpace không map trang này. Fault3 ghi vào
  +0x2000 tạo USER FAULT thứ ba, server vẫn Reply OK; ROOT SWITCH n>1.
- Stack tops, stack bases, server VSpace slot vào vifast-abi::user_layout;
  user.S nhận qua global_asm const operand; không còn 0x10000 viết tay.
- Host test đếm leaf: server VSpace đúng 2 leaf user (code + stack +0x2000),
  root builder chỉ 1 leaf code trong test shared; extra mapping sẽ đỏ.
- vifast-elf parse ELF64 PT_LOAD host-test; đã nối boot flow 6.4.
- 6.2: `build_task_space_elf` host-test trên PteFakeOps 3 format; leaf đúng
  trang/quyền, guard không map, extension + rollback.
- 6.3: jump table user.S + USER_ENTRY_*; vifast-root ELF link 3 target tại
  ROOT_ELF_BASE; code model x86 pic/-no-pie, RISC-V medium, AArch static;
  build.sh kiểm tra ET_EXEC/segment/window, size/window và FP/SIMD.
- 6.4: runtime include root ELF, parse cửa sổ 1 MiB, map root ELF + stack Rust
  vào root VSpace, entry từ ELF, stack ROOT_ELF_STACK_TOP; bỏ root assembly;
  harness yêu cầu ROOT RUST/ROOT READY, không ROOT PANIC; từ 7.5 harness yêu cầu
  đúng 4 USER FAULT, thêm địa chỉ root Rust stack.
- Boot Untyped tăng lên 128 KiB/32 object; data Untyped giảm còn 48 object;
  tổng quota 5 + 32 + 48 + 32 = 117 <= 128. Log in used/free pages và objects.
- Không còn code_page()/build_page_tables()/bảng user tĩnh; init_platform chỉ
  dựng bảng kernel-only, bảng user do retype VSpace + PageTableMap tạo.
- Data +0xa000 không được map sẵn trong root VSpace; PT cuối đã có do code/stack
  cùng vùng 2 MiB, nên FrameMap đầu tiên của client vẫn thành công.
- PageTableOps mở rộng read/write PTE, encode/decode, install_kernel, flush,
  sync_icache; kernel walk generic, không gom RevokeReport.
- vifast-pte crate no_std chứa format 3 arch + host test; user VA constants
  và table_span() là nguồn duy nhất cho platform.
- FrameMap/FrameUnmap đi qua generic walk; platform không còn map/unmap riêng.
- 7.1b: W^X theo Frame; mapped_w/mapped_x cập nhật ở map/unmap/revoke/destroy,
  check_invariants đối chiếu đúng số leaf trong reverse map; mapping bắt buộc READ.
- Frame code dùng chung root/server map RX hai lần thành công (mapped_x=2).
- 7.1c: invariant PA duy nhất cho Frame/PageTable/VSpace root+aux và Untyped
  lồng theo owner; sync_icache khi map EXEC; counter dùng checked_add.
- 7.2: Retype Untyped con lấy size căn trang từ bump, giữ child max_objects
  trong `reserved`, con có bump/objects/reserved riêng; revoke cha thu hồi cây
  con theo thứ tự Frame → PageTable → VSpace → Untyped, revoke con chỉ reset
  con. Cap con không bao giờ root; PA uniqueness và quota được kiểm tra sau
  từng bước.
- 7.3: BootInfo 32 byte READ-only ở ROOT_BOOTINFO_VA; runtime cài code Frame
  caps READ|EXEC và truyền BootInfo qua arg0; root cấp slot động, kiểm tra
  magic/version/size và in BOOTINFO OK. Harness yêu cầu đúng một chuỗi này.
- 7.4/7.5: root dựng VSpace client từ data Untyped; code lấy từ Frame cap RX
  trong BootInfo, client/fault4 stack RW; helper map_with_tables lặp
  PageTableMap/FrameMap. Bảng cấp cuối phủ DATA_VA; log CLIENT VSPACE OK.
- 7.5: client nhận VSpace riêng slot 2 và Untyped con 16 KiB/4 object slot 3;
  root VSpace bỏ client stack; fault4 thread trong client VSpace ghi root Rust
  stack và tạo USER FAULT thứ tư. Harness yêu cầu đúng 4 fault; 3b hoàn tất.
  DATA UNTYPED lúc test-exit hiện tại: RISC-V 36/64 trang, AArch64 37/64,
  x86 41/64; object RISC-V/AArch 21/48, x86 25/48.
- RISC-V sync I-cache: fence.i local + maintenance IPI remote fence; AArch64
  dùng CTR_EL0 + dc cvau/dsb ish/ic ivau/dsb ish/isb (broadcast inner-shareable);
  x86 no-op vì I/D coherent.
- 8.2: ReplyRecv (syscall 4) reply caller hiện tại rồi Recv; không caller thì
  chỉ Recv; server user.S chuyển sang vòng ReplyRecv + Signal; 3 host test mới.
- 8.3: TCB fault endpoint (CapRef), TcbConfigure a5, message LABEL_FAULT,
  BlockedFault; handler queue/wake; root nhận fault4 và in FAULT EP OK addr=...
  x86 enter_user khôi phục đầy đủ register; 3 host test mới.
- 8.1: TcbSetIpcBuffer (41), IPC_BUFFER_WORDS=32, copy qua PA khi hai TCB có
  buffer; unmap/revoke xoá link. API/core + demo end-to-end 8.1b đã có.
- PageTableMap gắn page table ở entry invalid đầu tiên; level do kernel chọn.
- Retype VSpace cấp root + aux page, zero và gọi install_kernel thật.
- Revoke Untyped sắp thứ tự Frame → PageTable sâu nhất → PageTable cha → VSpace.
- Revoke PageTable/VSpace xoá reverse map trong span, zero trang bảng và gỡ
  metadata của bảng con; VSpace đang là root của thread hiện tại bị từ chối hủy.
- KERNEL_SHARED_SLOTS xác nhận AArch64 dùng L1[0..=1], không chỉ 0..1 GiB.
- Object.creator/CapRef + reparent chặn client revoke bản copy Untyped để phá object của task khác. Client nhận Untyped con từ 7.5; không còn CapCopy data Untyped.
- Runtime assert Untyped nằm trong kernel_accessible_range() của platform.
- Boot-time check walk leaf thật từ root: trap entry có X, stack/IST/GDT/TSS/IDT/
  KERNEL_STORAGE có W, tất cả non-user; kiểm tra toàn bộ kernel_accessible_range.
- Kernel static storage per platform; runtime không tạo Kernel trên stack.
- Runtime assert tổng max_objects Untyped + object runtime <= MAX_OBJECTS (128).
- Data Untyped 256 KiB/max48; metadata Untyped max32; boot Untyped 128 KiB/max32.
- vifast-kernel bỏ dependency vifast-arch; dùng Console trait.
- x86 page-fault/exception entry dùng efiapi rcx/rdx/r8; vector 0..31 có handler.
- #DF/NMI/#MC dùng IST1 và luôn panic, kể cả khi CS là user.
- Fault origin: RISC-V SPP, AArch64 vector current/lower, x86 error U/S.
- Fault1/2/3 (kernel VA, illegal instruction, stack server) và fault4 (root
  Rust stack từ VSpace client) có USER FAULT trên cả 3 arch; tổng 4 USER FAULT.
- Kernel object limits tăng 128/64/32; Kernel 597624 bytes dùng static storage;
  test harness cần stack 8 MiB (`.cargo/config.toml`).
- Host test hiện có: xem vùng `tests` sinh tự động.

16. NHỮNG HẠN CHẾ / NỢ KỸ THUẬT
--------------------------------
- Cả ba kiến trúc có scheduler trên CPU phụ và ba worker; x86 AP dùng BKL +
  `pick_on(1)`, timer LAPIC 100 Hz, D4 idle `sti; hlt; cli`.
- BKL ticket + RAII bọc trap/syscall/IRQ + x86 fault/page-fault; owner theo
  hart, assert nhả khóa trước user dùng `held_by(hart)`.
- TLB shootdown per-VA: mỗi VSpace có mask `harts_loaded`; không xóa bit chỉ
  vì hart switch sang VSpace khác. Unmap gửi maintenance `TLB_VA` (asid, VA,
  trang) tới đúng hart, AArch64 broadcast phần cứng (ipi=0), fallback full
  ASID khi dải >16 trang. `TLB STATS (va_flush=V, ipi=I, skipped=K)`; remote
  I-cache RISC-V qua IPI.
- x86 LINT0/LINT1 của AP bị mask; nếu AP vẫn nhận vector PIC thì panic
  `AP PIC IRQ` thay vì ack hộ BSP.
- Đã có first-fit range VA allocator; ASID/PCID đã tag theo VSpace (x86 PCID
  chỉ bật khi CPUID hỗ trợ; QEMU TCG fallback CR3).
- Chưa có free/destroy object/frame ngoài đường revoke Untyped; object table cố định 128.
- IOMMU mới có mô hình phần mềm (domain + reverse map + IoMap/IoUnmap);
  chưa có hardware IOMMU/VT-d/SMMU.
- `CapDelete` (13) vẫn placeholder ở syscall dù kernel có `delete_cap`;
  user chưa xóa được capability. Ghi nợ, chưa đổi ABI vòng này.
- vifast-elf + vifast-root ELF đã build/validate; server/client cũng là Rust ELF
  do runtime cấp qua BootInfo staging, root nạp vào VSpace riêng. IPC buffer
  core + demo end-to-end đã có (8.1/8.1b); chưa có shared-memory mapping dùng
  chung ngoài IPC buffer.
- Chưa có Reply mang capability; ReplyRecv và fault endpoint tối giản đã có.
- Kernel VFS fallback chỉ ramfs phẳng 8 file x 512 byte; fd vẫn nằm trong Tcb.
  Server dùng `vifast-fs` v2: journal 16 data block, 64 inode, file tối đa
  6 KiB, thư mục 12 block; chưa có cache block, indirect block, hard link
  protocol, symlink. Chưa có flush/barrier block cho phần cứng thật.
- R4: hart 1 spin BKL vẫn cao do worker c gọi `GetTime` liên tục; `GetTime`
  nay đọc BSP toàn hệ thống nhưng vẫn đi qua syscall/BKL, chưa tối ưu.
- RISC-V panic-halt dùng `sifive_test` mã `(1<<16)|0x3333`; QEMU thoát rc=1.
  Thành công ghi `0x5555`; không còn nợ R2.
- AArch64 có VirtIO-MMIO block, x86_64 có ATA PIO và RISC-V có VirtIO-MMIO
  block; FS v2 persistence đã có trên cả 3 kiến trúc.
- Chưa có driver network/GPU/input.
- Chưa có compositor, ViFastKey.
- Chưa có formal verification; đã có syscall fuzz 10.000 bước x 3 PTE format
  (S1), chưa phải fuzz diện rộng/ngẫu nhiên độc lập.
- RISC-V kernel accessible chỉ 4 MiB (0x8000_0000..0x8040_0000); runtime/assert
  kiểm tra kernel image + metadata/data/boot Untyped nằm trọn trong dải này.
- x86 identity map 0..1 GiB; nếu OVMF/RAM vượt mốc này phải lấy memory map.
- Kernel x86 static data đã NX; text vẫn X. Chưa có NX cho toàn bộ rodata/UEFI
  runtime pages.
- Root/server/client mỗi task một VSpace; root VSpace không còn stack client.
- Boot Untyped 128 KiB/32 object đã dùng cho root + server + BootInfo; log QEMU
  còn 11-13 trang và 13 object (RISC-V 13/13, AArch64/x86 11/13).
- Server VSpace cap slot 12 do runtime cài; root đọc từ BootInfo, không còn
  hard-code trong root task.
- Shared-code builder root/server chỉ nhận một trang code; builder ELF đã hỗ trợ
  nhiều PT_LOAD. Root ELF đã nạp ở 6.4.
- Root TCB bị cấm destroy; nếu vi phạm sẽ làm mất root cap của Untyped.
- Double-fetch nội dung user: BKL ngăn đổi mapping giữa validate/copy, nhưng
  hart khác vẫn có thể đổi nội dung buffer giữa hai lần đọc; các syscall copy
  một lần vào buffer kernel (hoặc cần pin/copy-once) khi rà soát SMP.
- Fault endpoint tối giản đã có (8.3): log/queue/BlockedFault + handler Recv;
  Reply để tiếp tục chưa được demo. Fault1/2/3 vẫn suspend.
- Self-revoke Untyped hiện trả IllegalOperation; supervisor hủy nhóm gồm chính
  nó sẽ cần destroy-xong-rồi-pick sau này.
- Revoke đi qua PageTableOps; chưa có batch/Preempted cho revoke rất lớn.
- Client nhận Untyped con riêng; root không còn CapCopy data Untyped cho client.
- TCB/VSpace chưa tách pool riêng; TCB có quota 16 nhưng vẫn nằm trong object table.
  Việc tách pool bị **cố ý hoãn** cho tới khi cần vượt 128 object hoặc trước SMP,
  vì static Kernel đã giải quyết áp lực stack và 3b chưa cần quá 128 object.
- `OBJ_TCB_BYTES=4096` chỉ là quota accounting, không phản ánh đúng `size_of::<Tcb>()`.
- Server fault khi client BlockedReply sẽ làm client chờ vô hạn; fault endpoint sẽ xử lý.

17. ĐỊNH HƯỚNG PHÁT TRIỂN ĐỀ XUẤT
----------------------------------
Thứ tự ưu tiên:

A. Userland / ABI trước
1. vifast-user wrappers + user layout (đã có bản đầu).
2. vifast-elf parser ELF64 PT_LOAD + host test (đã có).
3. Bước 6: root task Rust nạp từ ELF; stack 4 trang + guard — đã xong.
4. Bước 7: BootInfo (7.3), Untyped con (7.2), VSpace client (7.4), chuyển client/fault4 (7.5) — 3b hoàn tất.
5. Shared memory / IPC buffer (core + demo 8.1/8.1b đã có).
6. Reply capability; ReplyRecv/fault endpoint tối giản đã có.
7. Supervisor/pager tối giản.

B. Bộ nhớ
1. Physical frame allocator thật.
2. Page-table page allocator.
3. Range VA allocator.
4. Free/unmap toàn VSpace, thu hồi frame/object.
5. ASID/PCID — đã tag ASID/PCID theo VSpace (x86 conditional).
6. IOMMU — mô hình phần mềm đã có; hardware IOMMU/VT-d/SMMU còn thiếu.

C. SMP
1. x86 AP bring-up bằng ACPI MADT + LAPIC/SIPI — đã xong scheduler/worker;
   bước tiếp theo là per-hart runqueue/TLB theo VA.
2. Per-hart context/runqueue.
3. BKL trước, lock mịn sau.
4. IPI wakeup/reschedule — RISC-V IPI và AArch64 SGI đã chạy; chỉ gửi khi
   hart đích idle.
5. TLB shootdown đa hart — per-VA + mask harts_loaded đã có; full fallback giữ.
6. Timer per-hart — đã xong; BSP/AP rate độc lập, preempt 50/50 cả 3.

D. VFS / driver
1. VFS/FS v2 trong user-space server — đã xong (`vifast-fs`, journal,
   permission, fsutil).
2. Block driver VirtIO — đã có cho cả 3 kiến trúc.
3. FS bền vững — đã có 6 phase persistence/replay/migrate.
4. Network VirtIO + TCP/UDP tối giản.
5. Console/input driver.

E. Desktop / Space
1. Compositor.
2. ViFastKey.
3. Name server, supervisor, pager.
4. Security/verification.

18. QUYẾT ĐỊNH ĐÃ CHỐT
-----------------------
1. Ưu tiên userland/ELF trước SMP.
2. Giữ Platform trait; có thể tách vifast-platform-api sau.
3. Chuyển VSpace động khi làm ELF loader — đã xong ở Bước 4.
4. x86 AP đi theo ACPI MADT + LAPIC/SIPI; D4 giữ idle `sti; hlt; cli`.
5. Dùng BKL trước, tối ưu lock mịn sau khi đo tranh chấp.
6. FS chuyển ra user space khi có IPC buffer; ramfs kernel chỉ là tạm
   (VFS IPC bước đầu đã chạy, kernel ramfs chỉ còn fallback/selftest).
7. Shared memory/IPC buffer dùng Frame capability.
8. Fault endpoint gửi fault message và block thread; không panic kernel.
9. ELF loader dùng cho root task ngay ở Bước 6; không viết root flat binary
   trung gian. vifast-elf đã host-test parser và root ELF đã nạp ở 6.4.
10. Gom unsafe vào wrapper nhỏ: MMIO, CSR/MSR, PTE.
11. 3b: RISC-V copy root[KERNEL_SHARED_SLOTS]. x86/AArch64 dùng PML4/L0 và
    PDPT/L1 riêng cho từng VSpace; PDPT[0]/PDPT[3] x86 và L1[0..=1] AArch64
    trỏ bảng kernel dùng chung. User dùng entry riêng 64..127. TTBR1 để sau.
12. 3b: Context **không** cache VSpace root. Runtime đọc root từ TCB mỗi lần
    restore; nếu khác root đang nạp thì ghi satp/TTBR0/CR3 và full flush.
    read_user/write_user assert loaded root khớp current. Root switch có counter
    và log ROOT SWITCH n khi test-exit.
13. 3b: code/stack là mapping thường; TcbConfigure không kiểm tra entry/stack;
    entry sai sẽ fault và suspend. Window fields đã bị xoá ở Bước 5.
14. 3b: ảnh task là ELF boot module (PT_LOAD) page-aligned + Frame cap;
    BootInfo frame map read-only. Không dùng ảnh phẳng cho root.
15. 3b: chọn reverse map nhiều mapping, mỗi Mapping lưu Frame object; Frame giữ
    mapped_w/mapped_x để W^X đúng theo Frame. Revoke unmap mọi nơi.
16. Retype VSpace đã cấp root (+ PDPT/L1 phụ cho x86/AArch64), zero và gọi
    install_kernel. install_kernel chỉ copy KERNEL_SHARED_SLOTS; user slots
    do PageTableMap gắn. 4.3 root chạy trên VSpace retype; 4.5 server có VSpace
    riêng, code dùng chung Frame root, stack +0x2000 riêng. 7.5 client có VSpace
    riêng và Untyped con. Cap server nằm ở root slot 12 nhưng root đọc qua
    BootInfo; TcbConfigure(server) dùng slot này.
17. PageTableOps mở rộng read/write PTE, encode/decode, install_kernel, flush;
    thao tác ghi infallible; bỏ RevokeReport/UserAction::Revoked.
18. Retype VSpace/PageTable phải zero page; BootInfo định nghĩa trong vifast-abi
    có version/size và slot ranges; root không hard-code slot 1/2/3.
19. Self-revoke hiện từ chối; supervisor destroy-cả-nhóm là hạn chế đã ghi.
20. Page table thật là nguồn sự thật duy nhất; mappings chỉ reverse map.
21. Map/unmap chỉ trong user_va_range; không ghi bảng kernel dùng chung.
22. Chỉ invalid→valid và valid→invalid; dừng ở leaf cấp cao; barrier trong write_pte.
23. PageTableMap walk tới entry invalid đầu tiên, user không truyền level;
    toàn bộ table_span(level) phải nằm trong user_va_range.
24. Mỗi PageTable chỉ gắn một vị trí {vspace, va_base, level}; revoke theo thứ
    tự Frame → PageTable sâu nhất → cha → VSpace.
25. User VA range thật nằm trong vifast-pte: 64..128 GiB, không giao kernel
    identity; RISC-V nằm trong nửa thấp Sv39. Kernel slots dùng chung: x86
    PDPT[0]/PDPT[3], AArch64 L1[0..1], RISC-V root[0]/root[2]; PML4/L0 và
    PDPT/L1 là riêng mỗi VSpace.
26. Object lưu creator = CapRef { addr, id }; revoke chỉ khớp cap id chính xác.
    Khi TCB bị hủy, object creator và cap con được reparent lên cap cha còn
    sống gần nhất; không còn creator treo. delete Untyped cap trả KResult và bị từ chối
    khi còn object/cap con. Chỉ reset bump khi không còn object từ bất kỳ cap nào.
27. Runtime assert Untyped nằm trong kernel_accessible_range(); x86 assert
    LoadedImage base/end nằm trong 0..1 GiB. Boot check walk leaf thật từ root,
    kiểm tra non-user, X cho trap entry, W cho stack/storage và toàn bộ trang
    kernel accessible; chạy lại trên root mới trước lần switch root đầu tiên.
28. FrameUnmap cần VSpace cap WRITE **và** Frame cap có PA khớp mapping. Chỉ
    VSpace cap không đủ để unmap code/stack của task khác.
29. 4.5/6.4/7.5: root VSpace map stack fault1/2/3 + root Rust; client stack/
    fault4 stack chỉ có trong client VSpace. Fault3 ghi +0x2000 phải USER
    FAULT; fault4 ghi root Rust stack phải USER FAULT; server vẫn Reply OK.
    ROOT SWITCH n > 1.
30. Stack tops/bases và server VSpace slot là hằng trong vifast-abi; user.S
    nhận qua global_asm const, runtime dùng trực tiếp. Host test server VSpace
    đúng 2 leaf user (code + stack); root builder test đúng 1 leaf code.
31. vifast-elf parse/validate ELF64 PT_LOAD (machine/user-range tham số,
    file range, congruence, overlap, W^X, entry, segment cấm) đã host-test;
    root Rust đã nạp ELF ở 6.4; Untyped con xong 7.2; BootInfo xong 7.3; VSpace
    client xong 7.4; tách client/fault4 xong 7.5.
32. Root Rust ELF ở `ROOT_ELF_BASE = CODE_VA + 0x20_0000` (2 MiB kế tiếp), giữ
    user.S tại 64 GiB. Parser chỉ nhận cửa sổ
    `[ROOT_ELF_BASE, ROOT_ELF_STACK_GUARD)` 1 MiB, không dùng toàn dải user.
    Stack root 4 trang tại `ROOT_ELF_STACK_BASE`, một trang guard ngay dưới
    không map. Vùng mới cần PageTable cấp cuối riêng.
33. Code user không dùng FP/SIMD: RISC-V `riscv64imac-unknown-none-elf`
    (code-model medium), AArch64 `aarch64-unknown-none-softfloat` (static),
    x86_64 `x86_64-unknown-none` (relocation pic + link `-no-pie` để vẫn ET_EXEC).
    Kernel chưa lưu FP/SIMD context; build.sh disassemble root ELF và từ chối
    thanh ghi FP/SIMD. Linker dùng `-z max-page-size=4096`, `-z separate-code`,
    mỗi segment căn 4096.
34. 6.2 hoàn tất trên host: `build_task_space_elf` map PT_LOAD theo trang,
    copy filesz đúng offset, BSS nhờ retype zero, stack 4 trang + guard,
    rollback object/bump/PTE; test trên PteFakeOps cả 3 format. Đã nối boot ở 6.4.
35. 6.3: jump table tại đầu user.S với USER_ENTRY_*; root Rust ELF `_start`
    asm + logic root, linker ROOT_ELF_BASE, build/validate 3 target.
36. 6.4: root ELF được include_bytes! vào kernel, parse cửa sổ 1 MiB, map vào
    root VSpace cùng root ELF stack 4 trang + guard; root assembly và stack
    +0x4000 bị xoá. Entry root lấy từ ELF, stack ROOT_ELF_STACK_TOP. Harness
    kiểm tra ROOT RUST/ROOT READY, đúng 3 USER FAULT và không ROOT PANIC;
    7.5 nâng thành đúng 4 USER FAULT (thêm root Rust stack).
37. Frame cap là upper bound cho quyền FrameMap: mapping rights phải nằm trong
    cap rights. Test host xác nhận Frame cap READ|EXEC không map được RW.
    Root sẽ nhận Frame cap code user.S READ|EXEC (không WRITE) trong BootInfo
    ở Bước 7.3/7.4.
38. 7.1b: W^X theo Frame với mapped_w/mapped_x; FrameMap bắt buộc READ, nên
    W-only/X-only trả InvalidArg. FrameMap RW rồi RX cùng Frame ở VA/VSpace
    khác bị NoRights, snapshot page table không đổi; unmap hoặc revoke VSpace
    giải phóng bộ đếm; shared code RX vẫn map được ở hai VSpace. Bộ đếm được
    kiểm tra khớp reverse map trong check_invariants(). Map RW → unmap → map
    RX là chủ ý (mô hình W^X kiểu seL4), không phải lỗ hổng: tại một thời điểm
    Frame chỉ có một loại quyền.
39. 7.1c: PA uniqueness là invariant của check_invariants(), không chỉ test.
    Frame/PageTable/VSpace root+aux không được chồng dải; Untyped chỉ lồng
    nhau theo owner chain, con nằm trọn trong cha. Nhờ đó Mapping.frame vẫn
    đúng và hai Frame cùng PA không thể lách W^X. 7.2 chạy invariant sau mỗi
    bước tạo/revoke Untyped con.
40. 7.1c: PageTableOps::sync_icache(pa, size) được gọi khi mapping mới có EXEC.
    RISC-V fence.i; AArch64 dc cvau/dsb ish/ic ivau/dsb ish/isb; x86 no-op.
    Remote fence/IPI cho SMP đã có trên cả 3 kiến trúc; x86 dùng D4.
    Bộ đếm mapped_w/mapped_x dùng checked_add và trả OutOfMemory khi tràn u16.
41. 7.2: Untyped con dùng `reserved` của cha để giữ quota object; điều kiện
    `objects + reserved + child_max_objects + 1 <= max_objects`. Con lấy vùng
    căn trang từ bump, không zero vùng. Revoke cha thu hồi cây con post-order
    (Frame → PageTable sâu → VSpace → Untyped), rồi recount objects/reserved;
    revoke con chỉ reset con. Cap con/copy luôn root=false. Invariant PA/quota
    chạy sau mỗi bước; PA uniqueness vẫn đúng khi có nhiều con.
42. 7.3: BootInfo 32 byte `#[repr(C)]`, magic/version/size, slot ranges và
    code Frame caps. Runtime map READ-only tại `ROOT_BOOTINFO_VA`, truyền qua
    arg0; root cấp slot tuần tự từ `free_slot_start`, không dùng hằng slot.
    Code caps user.S là READ|EXEC, không WRITE/GRANT. Harness yêu cầu đúng một
    `BOOTINFO OK`.
43. 7.4: VSpace client, PageTable và stack được retype từ data Untyped của
    root; code dùng Frame cap RX trong BootInfo. Helper map_with_tables lặp
    PageTableMap/FrameMap đến khi hết MissingPageTable. Code/stack/DATA_VA
    cùng bảng cấp cuối. 7.4 chỉ log `CLIENT VSPACE OK`, chưa đổi TcbConfigure;
    DATA UNTYPED được log lại tại test-exit.
44. 7.5: client nhận VSpace riêng (slot 2) và Untyped con 16 KiB/4 object
    (slot 3); root VSpace bỏ client stack; client stack/fault4 stack nằm trong
    VSpace client. Fault4 ghi root Rust stack không map trong VSpace client.
    Harness yêu cầu đúng 4 USER FAULT, gồm 0x1000301000; `Zero OK` chạy qua
    Untyped con. Đây là mốc hoàn tất 3b.
45. 8.2: ReplyRecv dùng a0 endpoint READ, a1 recv slot, a2 label, a3..a5 data;
    reply caller hiện tại (nếu có) rồi Recv, không caller thì chỉ Recv. Server
    user.S vòng ReplyRecv + Signal; host test no-caller/order/stale epoch.
46. 8.3: TCB fault_ep CapRef đặt qua TcbConfigure a5; fault message LABEL_FAULT
    [addr,cause,0,0]; TCB BlockedFault; handler Recv queue/wake. Root priority
    cao nhận fault4, in FAULT EP OK addr=0x1000301000. x86 enter_user restore
    đủ register trước iretq.
47. 8.1: TcbSetIpcBuffer yêu cầu Frame map RW đúng VA trong VSpace TCB; copy
    32 word trực tiếp qua PA khi cả sender/receiver có buffer. Unmap/revoke
    mapping xoá ipc_buffer. Demo user.S buffer dài để 3c.

48. Vòng 03: Per-CPU state chỉ đọc từ vùng kernel-only (`sscratch`/`swapgs`/
    `TPIDR_EL1`); BKL là ticket lock FIFO có maintenance poll; kernel giữ
    `current[hart]` nhưng runqueue vẫn global; hart phụ chưa chạy scheduler.

49. Vòng 05: revoke Untyped đã kiểm tra current trước mutation; mailbox đối
    xứng theo hart; affinity TCB mặc định hart 0; RISC-V hart 1 idle/timer/IPI
    có `SMP IDLE OK` nhưng chưa chạy task; WFI được thay bằng idle spin.

50. Vòng 06: bundle một file; RISC-V hart 1 chạy worker thật với affinity
    `TcbSetAffinity`; `SMP SCHED OK`; AArch64 hart phụ dừng do exception
    `ELR=0x4008ebb4`, ESR=0; WFI chưa điều tra tiếp.

51. Vòng 07: hai worker RISC-V hart 1, đếm user entry mỗi lần về user;
    `SMP WORKERS OK`; WFI+SIE=0 ổn định. D1: không nhận interrupt khi ở kernel;
    AArch64 secondary diagnostics thử nhưng phải revert vì làm hỏng boot.

52. Vòng 08: hết bão IPI reschedule (`resched_sent=1`, `resched_wake=1`),
    BootInfo `sched_harts`, checker docs anchor; AArch64 3a vector an toàn.
53. Vòng 09: AArch64 CPU1 GIC banked/timer + masked WFI; `SMP TIMER OK`
    và `SMP IDLE OK`; R5: hủy tái hiện 1.0 vì đường DAIF.I=1 trong `smp.S`
    đã ổn định liên tục.
54. Vòng 10: D1 áp dụng cho cả RISC-V/AArch64; xử lý boot hart bất kỳ,
    runqueue hart phụ rỗng không exit, per-hart BKL owner, CPACR CPU1,
    post-worker idle timer; docs generator và harness spec.
55. Vòng 11: harness spec + case âm; post-worker `SMP POST IDLE OK`;
    boot-handoff feature; syscall status implemented/placeholder.
56. Vòng 12: ACPI MADT parser + LAPIC BSP map/enable; x86 vẫn AP chưa online.
57. Vòng 13: x86 AP trampoline, INIT-SIPI-SIPI, long mode, maintenance.
58. D2 (vòng 12): AP x86 IF=0 mọi lúc; idle poll mailbox + `pause`, không
    `hlt`/`sti; hlt`; IPI chỉ nhận qua vector khi AP đang ring 3. Đã thay bởi D4
    ở vòng 15.
59. Vòng 14: AP x86 TSS/GS riêng, IPI LAPIC thật qua IRR, LAPIC timer và idle
    poll; worker/scheduler trên AP chưa làm.
60. D3 (đã hủy ở vòng 15): đọc IRR khi IF=0 không xóa được bit pending; EOI
    không tác dụng lên IRR. Số liệu vòng 14 vì vậy không hợp lệ.
61. D4: x86 IF=1 ở ring 0 chỉ trong `sti; hlt; cli`; vector 0x30/0x31 khi
    kernel chỉ hợp lệ trong cửa sổ idle, không lấy BKL, EOI rồi iretq; ring 3
    giữ đường preempt hiện có. Cấm poll IRR để xử lý interrupt.
62. Vòng 15: Task 1 IRR diag, Task 2 TSC/PIT + LAPIC timer calibrated + D4,
    Task 3 BKL/`pick_on(1)` AP, Task 4 worker AP, Task 5 stage trampoline.
63. Vòng 15 (đóng): IRR DIAG không kết luận vì mâu thuẫn SDM/số liệu vòng 14;
    D3 đã hủy, D4 giữ nguyên.
64. Vòng 16: BKL owner theo hart + assert nhả khóa; mask LINT AP; worker c
    busy chứng minh preempt; panic-halt đa hart; BKL stats; docs R1/R7.
65. D5: FS v2 nằm trong `vifast-fs` no_std/no alloc/forbid unsafe; layout
    VF02, 64 inode, file 6 KiB, journal 16 data block + commit CRC32; protocol
    mở rộng; quyền theo badge endpoint; block syscall cần cap BlockDevice.
66. Vòng 17: D5 FS v2 + fsutil crash injection; server VFS v2; tick theo hart;
    block cap; CapMint badge/permission; persistence 6 phase cả 3 kiến trúc.
67. D6: badge 0 vô danh, superuser `0xFFFF`; CapMint chỉ mint từ source badge 0;
    CapCopy giữ badge; badge do kernel ghi vào thanh ghi riêng (RISC-V a7,
    AArch64 x7, x86_64 r10), không dùng chung `value`; server từ chối badge 0.
68. Vòng 18: RISC-V panic rc=1 qua `sifive_test` upper-16; sửa D6; GetTime
    toàn hệ thống; FS ảnh nhỏ ENOSPC/thư mục 1 block; TLB per-VA + mask
    harts_loaded; docs R6.

19. FILE / TÀI LIỆU LIÊN QUAN
-----------------------------
README.md
docs/00-hien-trang.md
docs/10-multi-arch.md
docs/11-hien-tai/00-index.md
docs/11-hien-tai/01-abi.md
docs/11-hien-tai/02-objects-capabilities.md
docs/11-hien-tai/03-ipc-scheduler.md
docs/11-hien-tai/04-memory-vspace.md
docs/11-hien-tai/05-vfs.md
docs/11-hien-tai/06-runtime-platform.md
docs/11-hien-tai/07-arch-ports.md
docs/11-hien-tai/08-boot-user-flow.md
docs/11-hien-tai/09-testing.md
docs/11-hien-tai/11-phase-3b-decisions.md
docs/11-hien-tai/12-vifast-pte.md

Mã chính:
crates/vifast-kernel/src/
crates/vifast-elf/src/lib.rs
crates/vifast-runtime/src/lib.rs
crates/vifast-arch-*/src/lib.rs
crates/vifast-platform-*/src/
crates/vifast-kernel-*/src/main.rs
crates/vifast-arch/src/bkl.rs (BKL) + CpuState/MAX_HARTS
crates/vifast-virtio/src/lib.rs
crates/vifast-acpi/src/lib.rs
crates/vifast-fs/src/lib.rs
crates/vifast-fsutil/src/main.rs
scripts/build.sh
scripts/test-qemu.py
scripts/test-persist.py
scripts/test-harness-spec.py
scripts/harness_spec.py
scripts/gen-docs.py
scripts/check-docs.py

20. KẾT LUẬN NGẮN
------------------
Đây là một microkernel bring-up đa kiến trúc có:
- lõi portable sạch (không unsafe/asm/hardware);
- platform layer riêng cho MMU/trap/timer/SMP;
- binary kernel mỏng;
- user mode, IPC, capability transfer, page mapping, user-space VFS và timer
  preemption;
- test QEMU 3 kiến trúc xanh.

Nó chưa phải OS hoàn chỉnh. Root và server/client đều chạy từ ELF Rust; mỗi
task có VSpace riêng; client dùng Untyped con. Đã có FS v2 journal hóa trong
user space, block cap, permission theo badge D6, persistence 6 phase cả 3 kiến
trúc, BKL ticket per-hart, tick BSP/AP độc lập, ASID/PCID, IPI/TLB per-VA
targeted + fallback full ASID, remote fence.i. Khoảng trống tiếp theo:
hardware IOMMU, x2APIC/per-hart runqueue, network/input, compositor/ViFastKey.
```

SHA-256 part1=fdcbb7921abf432ce0944721c438e0566aa2f752d7cfb9fae8a6042368846470 part3=282f2b7fe48c847a9decb59b3864292ea7ccf0c95cfe49a5a829ae0f628b469e
