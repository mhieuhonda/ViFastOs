# TỔNG KẾT ĐÊM

## Commit đã merge vào main

- `b0a82cb` — 7.2 Untyped con.
- `ba17546` — 7.3 BootInfo + Frame cap code.
- `39fc83c` — 7.4 Root dựng VSpace client.
- `aeea6bc` — 7.5 Client tách hẳn + fault4 (kết thúc 3b).
- `c5119e7` — 8.2 ReplyRecv.
- `a18663f` — 8.3 Fault endpoint tối giản.
- `854744a` — 8.1 IPC buffer API/core.
- `49e7b1f` — 8.1b IPC buffer demo end-to-end.
- `82c9c11` — S2 x86 kernel data NX.
- `ea3793b` — 8.4 Rust server/client ELF modules.
- `6963f58` — 8.4b runtime boot modules via BootInfo.
- `562727d` — first-fit VSpace VA range allocator.
- `395a589` — notification capability transfer demo.
- `85fe2cf` — user-space VFS server over IPC.
- `826efed` — ASID/PCID VSpace tagging.
- `fffa7d6` — VFS STAT request.
- `687962c` — software IOMMU domains and DMA maps.
- `ed06f82` — SMP maintenance IPI and TLB shootdown.
- `f216966` — AArch64 VirtIO block backend.
- `1d4f0ea` — persistent VFS scratch on block storage.
- `fe569c1` — x86 ATA block backend for persistent VFS.
- `test: hermetic two-boot persistence check` — Task 2 (Agent 2).
- `refactor: add big kernel lock and per-cpu state groundwork` — Task 4 (Agent 2).
- `23d8dfb` — RISC-V VirtIO-MMIO block backend (vòng 02).
- `b51782b` — BKL audit + x86 fault path (vòng 02).
- `4d44b60` — per-CPU trap stacks/state (vòng 02).
- `74b7d75` — per-CPU security, ticket BKL, per-hart current (vòng 03).
- `ed40921` — S1 syscall fuzz 3 PTE formats.
- Các commit docs: `24d94fa`, `1473e65`, `32784d6`, `bde2254`, `2b57994`, `55c425b`.

## WIP còn dở

Không có nhánh `wip/*`. Repo sạch tại thời điểm tổng kết.

## CẦN QUYẾT ĐỊNH

- 8.4/8.4b đã có: server/client là crate Rust, build thành ELF riêng; runtime
  cấp module qua BootInfo, map bytes read-only vào root VSpace; root parse và nạp
  vào VSpace riêng bằng `FrameWrite`. Range VA allocator first-fit đã có.
- 8.5 đã có user-space VFS server: `/motd` và `/scratch` đi qua IPC
  request/reply; client không còn dùng syscall Open/Read/Write/Close của kernel.
  Kernel ramfs chỉ còn là fallback/selftest.
- 8.6 đã tag VSpace bằng ASID/PCID: RISC-V satp ASID + `sfence.vma` theo tag,
  AArch64 TTBR0 ASID + `tlbi aside1is`, x86 PCID nếu CPUID hỗ trợ (QEMU TCG
  hiện không bật PCID nên dùng CR3 fallback).
- Fault endpoint tối giản: message `LABEL_FAULT` data `[addr,cause,0,0]`, không
  có Reply-to-continue trong demo; fault1/2/3 vẫn suspend.
- Root task được nâng priority 15 để nhận fault4 khi message đến.
- AArch64 fault4 cần literal pool trước `user_program_end`; x86 `enter_user`
  khôi phục đủ register trước iretq.

## Số liệu cuối

- `vifast-kernel` 83/83, `vifast-arch` 2/2, PTE 11/11, ELF 17/17;
  runtime/user compile.
- QEMU 3/3 PASS ×3; 4 USER FAULT, có `0x80000000`, `0x1000002000`,
  `0x1000301000`; `FAULT EP OK`; `RUST MODULES OK`, `VFS SERVER OK`, `VFS OK`,
  `IOMMU OK`; RISC-V/AArch `SMP OK (... ipi=1)`; cả 3 `BLOCK OK`,
  `FS DISK OK`; persistence do `scripts/test-persist.py` kiểm tra.
- Boot Untyped: RISC-V 20/32 pages, AArch/x86 22/32; free 12/10 pages,
  12/12 objects.
- DATA UNTYPED lúc test-exit: RISC-V 36/64 pages, AArch64 37/64 pages,
  x86 41/64 pages; 21/48 object (x86 25/48).
- ROOT SWITCH lần chạy cuối: 35 / 35 / 35 (timer test-exit).
- Root ELF: RISC-V 12880 B, AArch64 12704 B, x86 16744 B; headroom RISC-V
  565200 B.
- SHA-256 `ViFastOs-architecture-summary.txt`: `f5a94d387c241cdc25b069d284cdf386ae0efa9839f68652344fe2d5cf4fb5ea` (cập nhật sau vòng 22.2; bản cũ của mốc này là `6603365d8ecd728b7c65e3c6097124443f01543f7097f3ab07d40ce86d0f6d93`).

---

# ViFastOs overnight log

Started from `c363e7d` (Step 7.1c). Working tree also contains its documentation
follow-up `62b31f4`.

## Start

- Host tests: `vifast-kernel` 61, `vifast-pte` 11, `vifast-elf` 17.
- QEMU: RISC-V, AArch64, x86_64 PASS three consecutive runs.
- Goal: implement the P0 sequence 7.2 → 7.5, then independent 8.2/8.3 or
  fallback work while keeping every commit green.

## CẦN QUYẾT ĐỊNH

(empty)

## 7.2 — Untyped con

- Commit `b0a82cb feat: add child untyped allocation and revoke`.
- `vifast-kernel` 69/69 host tests; PTE 11/11; ELF 17/17; runtime/user compile.
- `build.sh` PASS, no warnings; QEMU 3/3 PASS three consecutive runs
  (ROOT SWITCH 20923 / 38179 / 12394).
- Added `reserved` quota, child range from parent bump, recursive post-order
  revoke (Frame → PageTable → VSpace → Untyped), exact recount and cap root=false.
- 8 new tests cover quota/range snapshots, sibling PA uniqueness, child frame
  containment, RW mapping + zeroed PA reuse over all 3 PTE formats, child-only
  revoke, nested revoke, delete refusal, and TCB cap reparent.

## 7.3 — BootInfo và Frame cap code

- Commit `ba17546 feat: pass bootinfo and user code caps to root`.
- `vifast-kernel` 70/70; PTE 11/11; ELF 17/17; runtime/user compile.
- BootInfo 32 byte READ-only tại `0x1000306000`; root nhận qua arg0, in
  `BOOTINFO OK`. Harness giờ yêu cầu đúng một chuỗi này.
- Code Frame caps user.S nằm ở root slot 20, quyền READ|EXEC; root cấp object
  từ `free_slot_start=21` (BootInfo-driven) nên không còn hằng slot.
- QEMU 3/3 PASS ×3; ROOT SWITCH 44665 / 46524 / 14456 (lần chạy đầu).
  Boot Untyped: RISC-V 20/32 free 12, AArch64/x86 22/32 free 10.
- Root ELF mới: RISC-V 8688 B, AArch64/x86 8520 B; headroom RISC-V 626672 B.

## 7.4 — Root dựng VSpace client

- Commit `39fc83c feat: build private client vspace`.
- `vifast-kernel` 70/70; PTE 11/11; ELF 17/17; runtime/user compile.
- Root retype VSpace/page table/stack từ data Untyped; code map từ Frame cap RX
  trong BootInfo; helper `map_with_tables` lặp PageTableMap/FrameMap; log
  `CLIENT VSPACE OK`. Client chưa chuyển VSpace.
- `DATA UNTYPED` giờ được log lúc test-exit: RISC-V 6/64 trang, AArch64/x86
  7/64 trang, 5/48 object. Boot vẫn log 0/64 trước root chạy.
- QEMU 3/3 PASS ×3; ROOT SWITCH 41561 / 39230 / 19240 (lần chạy đầu).
- Root ELF: RISC-V 8704 B, AArch64/x86 8536 B; headroom vẫn 626672 B.

## 7.5 — Client tách hẳn + fault4 (kết thúc 3b)

- Commit `aeea6bc feat: isolate client vspace and add fault4`.
- `vifast-kernel` 70/70; PTE 11/11; ELF 17/17; runtime/user compile.
- Client nhận VSpace riêng slot 2 và Untyped con 64 KiB/8 object slot 3;
  root VSpace bỏ client stack; client/fault4 stack nằm trong VSpace client.
- Fault4 chạy trong VSpace client và ghi root Rust stack; sửa literal pool
  AArch64 bằng `.ltorg` trước `user_program_end` để địa chỉ 0x1000301000 thật
  sự được copy vào user image.
- Harness yêu cầu đúng 4 USER FAULT (0x80000000, 0x1000002000, 0x1000301000),
  `CLIENT VSPACE OK`, `BOOTINFO OK`; `Zero OK` vẫn chạy qua Untyped con.
- QEMU 3/3 PASS ×3; ROOT SWITCH 39826 / 42286 / 13030 (lần chạy đầu).
  DATA UNTYPED test-exit: RISC-V 21/64 trang, AArch/x86 22/64 trang, 6/48 object.
- Root ELF: RISC-V 8704 B, AArch64/x86 8536 B; headroom RISC-V 626672 B.

## 8.2 — ReplyRecv

- Commit `c5119e7 feat: add reply-recvsyscall and server loop`.
- `vifast-kernel` 73/73; PTE 11/11; ELF 17/17; runtime/user compile.
- Syscall 4: a0 endpoint READ, a1 recv slot, a2 label, a3..a5 data. Reply
  caller hiện tại trước rồi Recv; không caller chỉ Recv; stale epoch NoCaller.
- Server user.S cả 3 arch chuyển sang vòng ReplyRecv + Signal; harness giữ
  nguyên output cũ và 4 USER FAULT.
- QEMU 3/3 PASS ×3; ROOT SWITCH lần đầu 49450 / 37252 / 22755.
  Headroom RISC-V còn 622576 B.

## 8.3 — Fault endpoint tối giản

- Commit `a18663f feat: deliver user faults to endpoint`.
- `vifast-kernel` 76/76; PTE 11/11; ELF 17/17; runtime/user compile.
- TCB có `fault_ep: CapRef`; TcbConfigure a5 trỏ cap Endpoint trong cspace TCB
  đích. Fault message label `LABEL_FAULT`, data `[addr,cause,0,0]`; TCB
  `BlockedFault`; handler Recv queue/wake. Fault1/2/3 vẫn suspend.
- Root tạo fault endpoint, copy cap cho fault4 slot 6, cấu hình a5=6; root
  priority 15 để nhận message, in `FAULT EP OK addr=0x1000301000`.
- Sửa x86 `enter_user` khôi phục đủ register trước iretq (cần cho resume handler
  sau page fault); sửa AArch literal pool cho fault4.
- QEMU 3/3 PASS ×3; ROOT SWITCH lần đầu 49945 / 32607 / 17520.
  Data Untyped test-exit vẫn 21/22 trang, 6/48 object; headroom 618480 B.

## 8.1 — IPC buffer API/core

- Commit `854744a feat: add ipc buffer binding and copy`.
- TCB có `ipc_buffer`; syscall 41 `TcbSetIpcBuffer(tcb, frame, va)` yêu cầu
  Frame map RW đúng VA trong VSpace của TCB.
- Khi hai TCB đều có buffer, kernel copy `IPC_BUFFER_WORDS=32` trực tiếp qua PA;
  unmap/revoke mapping xoá liên kết.
- `vifast-kernel` 77/77; PTE 11/11; ELF 17/17; runtime/user compile.
- QEMU 3/3 PASS ×3; ROOT SWITCH 47861 / 38447 / 15977 (lần đầu).
- Chưa nối demo chuỗi dài trong user.S; đó là phần còn lại của 3c.

## 8.4 — Rust server/client ELF modules

- Commit `ea3793b feat: load rust server and client elf modules`.
- Thêm hai crate `vifast-server` và `vifast-client` (`no_std`, `no_main`), entry
  `_start` assembly ngắn, logic server/client bằng Rust dùng `vifast-user`.
- `build.sh` build hai ELF riêng cho RISC-V/AArch64/x86_64, strip, rồi nhúng bytes
  vào root ELF bằng `include_bytes!`; root parse `vifast-elf` và nạp từng PT_LOAD
  vào VSpace server/client, đúng quyền RX/RW.
- Thêm syscall `FrameWrite` (42) + `Runtime UserAction::FrameWrite` +
  `Kernel::copy_frame_bytes`: root copy thẳng bytes vào Frame qua kernel, không
  cần mapping staging. Đây cũng là đường copy đúng cache/PA cho AArch64.
- Server Rust dùng `ReplyRecv` + in IPC buffer; client Rust làm file/ramfs demo,
  FrameMap, zero-frame, IPC Call và `Cap OK`/`Reply OK`. `user.S` chỉ còn fault
  threads và jump table.
- Sửa `vifast-user::syscall` để khai báo tất cả argument registers bị clobber;
  lỗi này bị che bởi các debug `write` trước đó và làm root treo khi optimize.
- AArch64 `sync_icache` đổi sang `dc civac`; CONTEXTS tăng 32→128 vì object
  index dịch khi root ELF lớn hơn; Frame cap từ Retype có EXEC; allowlist user
  code mở đến 0x20000 cho module strings/IPC buffer.
- `vifast-kernel` 79/79; PTE 11/11; ELF 17/17; build không warning; QEMU 3/3
  PASS ×3. Root ELF RISC-V 26168 B, AArch64 29768 B, x86 33128 B; RISC-V
  headroom 573424 B.
- DATA UNTYPED test-exit: RISC-V 27/64 trang, AArch64/x86 28/64 trang, 12/48
  object. Harness yêu cầu thêm `RUST MODULES OK` đúng một lần.

## 8.11 — x86_64 ATA block backend

- Commit `fe569c1 feat: add x86 ata block backend for persistent vfs`.
- QEMU q35 harness gắn `isa-ide` + `ide-hd` ở `bus=ide.6,unit=0`; kernel dùng
  ATA PIO primary 0x1F0/0x3F6, LBA28, poll BSY/DRQ, flush sau write.
- x86 cũng chạy `BLOCK OK`, `FS DISK OK`, `FS PERSIST OK` và dùng cùng sector
  `VF01` của server VFS.
- RISC-V vẫn fallback RAM vì virtio-mmio ở 0x10001000 nằm ngoài PMP/OpenSBI
  mặc định; hướng còn lại là PCI virtio hoặc custom OpenSBI.
- `vifast-kernel` 83/83; QEMU 3/3 PASS.

## 8.27 — SMP hardening, preempt proof, panic halt (vòng 16)

- `7196a4e`: BKL owner theo hart + assert nhả khóa; AP mask LINT0/1 và panic `AP PIC IRQ`.
- Worker c busy `GetTime`, `SMP PREEMPT OK` U/P; W chỉ resched IPI; 50/50 cả 3.
- Panic-halt IPI: RISC SBI, AArch SGI2, x86 LAPIC 0x32; feature 5/5 mỗi kiến trúc.
- `BKL STATS` ở test-exit; harness 237 case âm; docs R1/R7.
- QEMU 10/10 all, x86/RISC/AArch 50/50; handoff 10/10; persist 3/3.

## 8.26 — x86 AP scheduler/worker, D4, stage (vòng 15)

- `58016c8`: TSC/PIT + LAPIC timer calibrated, D4 `sti; hlt; cli`, BKL/pick_on AP.
- AP worker A/B, `sched_harts=0b11`, `SMP WORKERS OK`, `SMP POST IDLE OK`; x86 50/50.
- Fix stack AP `TRAP_STACKS[1]+0x4000`; stage 1..4 + `test-ap-stall`.
- D3 hủy vì IRR không xóa khi IF=0; IRR DIAG thực tế after=0, isr=0.
- Docs R6/R7, ORDER generated, 201 negative cases; QEMU 10/10 all.

## 8.25 — x86 AP timer/IPI, docs (vòng 14)

- `6ed6cde`: AP TSS/GS riêng, IPI LAPIC thật qua IRR, thứ tự log.
- `03cb048`: AP LAPIC timer, idle IRR polling; x86 50/50.
- Task 3 worker/AP scheduler chưa làm; TSC calibration/stage byte còn nợ.
- Docs R6, D3, crate coverage; QEMU 10/10 all.

## 8.24 — x86 AP online (vòng 13)

- `78af406`: trang trampoline 0x9E000 <1 MiB; `AP TRAMPOLINE OK`.
- `dd07e6b`: INIT-SIPI-SIPI, AP real mode magic.
- `7dc8a94`: trampoline 16→32→64-bit, `AP LONG MODE OK`.
- `49c4522`: AP maintenance online, `SMP OK (harts=2, ipi=1)`.
- `03e8c1d`: PDPT[3] là kernel shared slot; LAPIC mapping supervisor/NX/UC.
- x86 50/50; all 10/10; feature 10/10; persist 3/3.

## 8.23 — ACPI MADT và LAPIC x86 (vòng 12)

- `1ff8644`: boot handoff hết retry in `BOOT HANDOFF FAIL` rồi SRST.
- `9b8f031`: crate `vifast-acpi` 8 test; x86 đọc RSDP từ UEFI config table,
  in `ACPI MADT OK (cpus=2, bsp=0, lapic=0xfee00000)`.
- `627f054`/`83875a7`: map/bật LAPIC BSP, `LAPIC OK (id=0)`.
- 3b/3c AP trampoline chưa làm; x86 vẫn `SMP SKIP`.
- QEMU 10/10 all; feature 10/10; persist 3/3; docs checker PASS.

## 8.22 — Harness spec, post-worker timer, boot handoff (vòng 11)

- `17aee2a`: khôi phục BLOCK/FS/SMP SKIP/exit code vào harness spec; 166 case âm.
- `d8424e5`: worker block Wait; `SMP POST IDLE OK` chứng minh timer WFI.
- `b122c05`: feature `test-boot-handoff`, retry HSM start, `BOOT HANDOFF OK`.
- `32c2e03`: `SYSCALL_STATUS` implemented/placeholder; generated status block;
  host test dispatch fallback (kernel 88/88).
- QEMU 10/10 all; RISC/AArch 50/50; feature 10/10; persist 3/3.

## 8.21 — RISC-V flake, AArch64 worker, generated docs (vòng 10)

- `060fb67` + `010cccd`: `--arch/--repeat`, QEMU monitor register dump and
  timeout artifacts.
- `386a2ad`: arbitrary OpenSBI boot hart promotes hart 0; secondary empty
  runqueue parks with BKL released instead of shutting down; test-exit waits
  for root/worker completion.
- `dbe3a39`: separate idle wake counters (`wake_timer`, `wake_sgi`, K).
- `cce094a`: AArch64 CPU1 per-hart BKL/current, GIC SGIs, scheduler and two
  workers; AArch64 50/50.
- `ec090c9`: summary commits/syscalls/tests/pass generated from git, ABI,
  build log and shared `harness_spec.py`.
- RISC-V 50/50 after fix; all-arch QEMU 10/10; persistence 3/3.

## 8.20 — AArch64 CPU1 timer/idle + docs checks (vòng 08/09)

- `e895bda`: idle flag + fire-and-forget reschedule; S=1/W=1, bão IPI hết.
- `dbe7675`: BootInfo v2 `sched_harts`, root không biết kiến trúc.
- `e3c4280`: AArch64 vector diagnostics an toàn.
- `c7c45ed`: AArch64 CPU1 DAIF mask, GIC banked, timer riêng, WFI polling
  `GICC_IAR`; `SMP TIMER OK` + `SMP IDLE OK`; chưa worker.
- `3631ed7`: docs checker exact commit order/headroom/positive strings.
- QEMU AArch 1.1/1.2 10/10; RISC Task 1 10/10; persist 3/3.

## 8.19 — Two workers, real counters, WFI idle (vòng 07)

- `e05775a`: hai worker affinity hart 1, stack A/B trong ABI, nhận diện label
  endpoint; `user_entries` tăng mỗi trap từ user; `SMP WORKERS OK`.
- `b7f26ef`: idle WFI với SIE=0, sie.SSIE|STIE; WFI 10/10, bỏ spin.
- `ce7dbc9`: giới hạn worker về RISC-V.
- AArch64 3a vector diagnostics thử nhưng boot hỏng; revert, không commit.
- Kernel 87/87, arch 5/5, headroom 458352; QEMU Task1/Task2 10/10.

## 8.18 — Single-file bundle + RISC-V secondary worker (vòng 06)

- `067b0d9`: `scripts/make-bundle.py`; bundle round 05; self-check summary
  SHA against overnight log.
- `b76e930`: root tạo `smp-worker` dùng chung root VSpace, stack riêng tại
  `WORKER_STACK_BASE`; `TcbSetAffinity` (syscall 20) đặt `0b10`; worker
  `Yield` 50 lần rồi loop; không Exit để tránh tắt máy.
- `user_syscall_on(tcb, hart, ...)` sửa Yield theo hart; trước đó Yield luôn
  tác động hart 0 làm pick_on(1) rỗng.
- `SMP SCHED OK (hart0=1, hart1=1, resched=1)` 10/10; resched IPI thật từ
  `TcbResume(worker)`.
- AArch64 secondary thử GIC/timer/worker: `KERNEL EXCEPTION ESR=0x0
  ELR=0x4008ebb4 FAR=0x0` trong `poll_maintenance`; đã revert, không commit.
- Kernel 87/87, arch 5/5, headroom 458352; QEMU 10/10; persist 3/3.

## 8.17 — SMP brownfield: revoke check, symmetric mailbox, RISC-V idle (vòng 05)

- `31f023b`: thêm host test revoke Untyped khi TCB current trên hart 1; kiểm
  tra current chạy sau collect (chỉ đọc), trước sort/destroy; snapshot
  object/cap/usage không đổi.
- `c4854d1`: mailbox maintenance theo hart đích (`MAINT_SEQ/ACK/KIND`), bỏ
  điều kiện hart 0/1 ở gửi/poll, sender assert giữ BKL, bỏ qua hart offline.
  Visitor TLB/I-cache; host test khóa hai chiều.
- `76f1c41`: TCB `affinity` mặc định hart 0; `pick_on` lọc affinity; hart 1
  RISC-V có timer SBI, idle loop BKL→pick→drop→wake; IPI reschedule qua
  mailbox; in `SMP IDLE OK (hart1_wakeups=K)`.
- WFI+SIE=0 làm mất wake/ACK trên QEMU/OpenSBI; idle dùng spin ngắn, 10/10
  QEMU. Đây là deviation có chủ đích, ghi ở report.
- Kernel 86/86, arch 5/5, headroom 458400; QEMU 10/10; persist 3/3.

## 8.16 — Per-CPU security, ticket BKL, per-hart current (vòng 03)

- Commit tích hợp `74b7d75` (Task 1–4 có phụ thuộc trap/BKL nên gộp).
- RISC-V: user `tp` không còn là state pointer; `sscratch` giữ CpuState khi
  user, trap entry lưu user sp/tp vào frame, dùng `s0` giữ CpuState qua C call.
- x86_64: `swapgs` + `IA32_KERNEL_GS_BASE`; user GS clobber không ảnh hưởng;
  `x86_reload_segments` reset GS base nên `prepare_user` publish lại.
- AArch64: `TPIDR_EL1` kernel-only; test clobber `TPIDR_EL0`.
- User test `PERCPU CLOBBER OK` cả 3 arch; harness bắt buộc đúng 1 lần.
- BKL: ticket lock FIFO, `lock_with(poll_maintenance)`; IRQ assert trước lock;
  IPI maintenance không lấy BKL; host test FIFO + mô phỏng deadlock PASS.
- Kernel: `current[hart]`, `pick_on/current_on`, runqueue global, invariant
  không TCB Running trên 2 hart; revoke TCB current bị từ chối.
- RISC-V `sync_icache` gửi maintenance IPI remote fence; AArch64 đã broadcast.
- QEMU 3/3 PASS ×3; persist 3/3; kernel 84/84; arch 4/4; headroom 458480.
- **Task 5 chưa làm**: hart phụ vẫn maintenance, chưa có `SMP SCHED OK`.

## 8.15 — BKL audit + x86 fault path + per-CPU groundwork (vòng 02)

- Commit `fix: make bkl cover all kernel entries and assert release on user return`.
- BKL bọc thêm x86 exception/page-fault; x86 `enter_user` đọc state per-CPU qua
  GS base và assert BKL đã nhả. RISC-V/AArch64 cũng assert trước enter_user.
- Mỗi trap entry assert IRQ đã tắt: RISC-V SIE, AArch64 DAIF.I, x86 RFLAGS.IF.
- IPI maintenance của hart phụ không lấy BKL; comment lý do tránh deadlock khi
  hart 0 đang giữ BKL và chờ ACK TLB shootdown.
- `BigKernelLock` giữ Acquire/Release ordering; chưa chuyển ticket lock.
- Task 4: tách trap stack/IST/TSS thành mảng `MAX_HARTS=4`; CPU state có
  `hart_id`, `current_tcb`, `trap_stack_top`; hart 0 đọc qua tp/TPIDR_EL1/GS.
- Host test `vifast-arch` 2/2; QEMU 3/3 PASS ×3; persist 3/3 PASS.
- `git show --stat` của Task 4 chỉ còn `vifast-arch` + 3 platform + docs.
- RISC-V kernel end `0x8038f0b0` / headroom `462672` byte (per-CPU stacks làm giảm).

## 8.14 — RISC-V VirtIO-MMIO block + docs sync (vòng 02)

- Commit `23d8dfb feat: riscv virtio-mmio block backend`.
- OpenSBI PMP Region07 cho S/U RWX toàn bộ; PMP không chặn. QEMU đặt device
  ở slot cuối `0x10008000`, các slot cách `0x1000`; `force-legacy=false`
  cho version 2/device 2.
- `vifast-virtio::init_range(start,end,stride)` scan toàn bộ; AArch64 giữ
  cửa sổ cũ `0x0a000000..0x0a004000`, stride `0x200`; RISC-V dùng
  `0x10001000..0x10009000`, stride `0x1000`. Legacy chỉ log
  `BLOCK SKIP (virtio legacy)`.
- RISC-V platform kiểm tra DMA range trong `kernel_accessible_range()`,
  implement `init_block/block_read/block_write`.
- Harness RISC-V thêm disk temp, `virtio-blk-device`, `force-legacy=false`,
  bật `require_block/require_disk`; `test-persist.py` chạy cả 3 kiến trúc.
- RISC-V kernel end `0x80377050`, headroom `561072` byte dưới `0x80400000`.
- `./scripts/build.sh` PASS; QEMU 3/3 PASS × 5; persist 3/3 PASS.

## 8.12 — Hermetic two-boot persistence check (Task 2)

- Commit: `test: hermetic two-boot persistence check` (Agent 2).
- Thêm `scripts/test-persist.py`: disk nằm trong temp dir riêng, xóa sau test.
- Phase 1 disk trắng: yêu cầu `FS DISK OK`, **không** có `FS PERSIST OK`;
  kiểm tra sector 0 có `VF01` + `ViFastOs write`.
- Phase 2 seed nonce ngẫu nhiên vào `VF01` sector 0; yêu cầu `FS PERSIST OK`
  và server in `FS PERSIST DATA <nonce>` đúng nonce.
- Phase âm magic sai: yêu cầu `FS DISK OK`, không có `FS PERSIST OK`.
- AArch64 và x86_64 PASS; RISC-V skip vì chưa có block backend.
- `test-qemu.py` nay tạo disk mới trong temp dir cho mỗi lần chạy, không dùng
  `/tmp/vifast-*.disk` cố định; vì vậy output chính chỉ cần `FS DISK OK`.
- `./scripts/build.sh` PASS; `vifast-kernel` 83/83; QEMU 3/3 PASS ×3; persist
  test PASS.

## 8.13 — BigKernelLock groundwork (Task 4)

- Commit: `refactor: add big kernel lock and per-cpu state groundwork`.
- Thêm `vifast-arch::bkl::BigKernelLock`: test-and-set AtomicBool + RAII guard;
  host test `vifast-arch` 2/2 PASS, gồm kiểm tra không re-enter khi đang giữ.
- Bọc `riscv_trap_handler`, `aarch64_trap_handler`/IRQ, `x86_trap_handler`/IRQ
  bằng BKL. Chưa bọc x86 exception/page-fault vì nhánh đó gọi `enter_user`
  trực tiếp; sẽ xử lý khi làm SMP.
- Chưa cho hart phụ chạy scheduler; single-hart behavior không đổi.
- Phân loại `static mut`:
  - boot-only/page-table: ROOT/KERNEL_L1/DEVICE_L1, L0/L1/L2, PML4/PDPT/PD,
    SPLIT_TABLES, GDT/IDT/TSS, NX/SMAP/PCID flags.
  - kernel state dưới BKL: KERNEL_PTR/KERNEL_STORAGE, ACTIVE_ROOT/ACTIVE_ASID,
    CONTEXTS, USER_DATA/METADATA/BOOT_POOL, timer interval.
  - per-CPU tương lai: TRAP_STACK/IST_STACK; hiện mỗi máy một bản.
  - user-space service: FILES/FILE_COUNT/DISK_OK/PERSISTED; cần khóa riêng
    khi server chạy đa luồng.
  - driver: VirtIO QUEUE/BASE/READY/LAST_*; một request/lần, cần khóa driver
    khi SMP.
- `./scripts/build.sh` PASS (thêm `cargo test -p vifast-arch`); QEMU 3/3 PASS ×3;
  persist test PASS.

## 8.10 — AArch64 VirtIO block + FS scratch bền vững

- Commit `f216966 feat: add aarch64 virtio block backend`.
- Commit `1d4f0ea feat: persist vfs scratch on block storage`.
- Thêm `vifast-virtio`: VirtIO-MMIO split queue một request, scan cửa sổ
  `0x0a000000..0x0a004000` stride `0x200` trên QEMU AArch64 virt.
- Kernel syscall `BlockRead`/`BlockWrite` (43/44); runtime validate/copy user
  sector 512 byte; AArch platform dùng VirtIO queue.
- Server VFS lưu `/scratch` vào sector 0 với magic `VF01`; lần boot sau đọc
  lại và in `FS PERSIST OK`. RISC-V/x86 chưa có block backend nên fallback
  `FS DISK RAM`.
- QEMU AArch dùng disk file 4 MiB và giữ nguyên giữa các lần chạy; harness
  yêu cầu `BLOCK OK` + `FS DISK OK`.
- `vifast-kernel` 83/83; QEMU 3/3 PASS.

## 8.9 — SMP maintenance IPI + full TLB shootdown

- Commit `ed06f82 feat: add smp maintenance ipi and tlb shootdown`.
- RISC-V dùng SBI IPI gửi hart phụ; secondary loop nhận pending, chạy
  `sfence.vma`/`fence.i`, tăng ack. AArch64 dùng GICv2 SGI 0 và
  `dsb ish`/`isb`.
- `flush_all_user` phát IPI và chờ ack trên RISC-V/AArch64; harness yêu cầu
  `SMP OK` kèm `ipi=1`.
- Đây là full TLB shootdown, chưa phải per-VA shootdown và chưa có scheduler
  đa hart; x86 vẫn SMP SKIP vì chưa có AP bootstrap.
- `vifast-kernel` 83/83; QEMU 3/3 PASS ×3.

## 8.8 — Software IOMMU domains

- Commit `687962c feat: add software iommu domains and dma maps`.
- Thêm object `ObjType::Iommu` + `IommuDomain` reverse map, tối đa 32 DMA
  mapping/domain; IOVA chỉ nhận vùng 4 GiB tại `IOMMU_IOVA_BASE`.
- Syscall `IoMap`/`IoUnmap`: map Frame vào domain, chỉ READ/WRITE, cấm EXEC,
  kiểm tra alignment/overlap; Frame bị destroy sẽ tự xoá DMA mapping.
- Root tạo domain từ data Untyped, map/unmap server IPC buffer và in
  `IOMMU OK`; harness yêu cầu đúng một chuỗi.
- Đây là mô hình IOMMU phần mềm; hardware IOMMU/VT-d/SMMU/IOMMU driver vẫn là
  phase riêng. `vifast-kernel` 83/83; QEMU 3/3 PASS.

## 8.7 — VFS STAT

- Commit `fffa7d6 feat: add vfs stat request`.
- Thêm `vfs::STAT`; server trả `OK` + size trong reply data0.
- Client kiểm tra `/scratch` sau read-back; in `VFS STAT OK`.
- Harness yêu cầu đúng một `VFS STAT OK`; QEMU 3/3 PASS.

## 8.6 — ASID/PCID cho VSpace

- Commit `826efed feat: tag vspaces with asid and pcid`.
- Kernel cấp ASID 12-bit không lặp trong phạm vi object table; `VSpace.asid`
  sống cùng object và `current_asid` lấy từ TCB hiện tại.
- Runtime truyền `(root, asid)` qua `Platform::switch_root`; platform chỉ
  invalidate tag của VSpace đích thay vì flush toàn bộ TLB user.
- RISC-V: `satp.ASID`, `sfence.vma zero, asid`.
- AArch64: `TCR_EL1.AS=1`, ASID trong TTBR0, `tlbi aside1is, asid`.
- x86_64: bật CR4.PCIDE khi CPUID leaf1 ECX bit17; CR3 mang PCID 12-bit. QEMU
  TCG hiện không expose PCID nên đường QEMU dùng CR3 fallback, vẫn đúng.
- Thêm test ASID khác 0/khác nhau; `vifast-kernel` 81/81; QEMU 3/3 PASS ×3.

## 8.5 — User-space VFS server qua IPC

- Commit `85fe2cf feat: add user-space vfs server over ipc`.
- `vifast-abi::vfs` thêm protocol request/reply: `OPEN`, `READ`, `WRITE`,
  `CLOSE`; reply label mang kết quả, tối đa 3 word dữ liệu.
- Thêm `vifast_user::call_message` và `reply_recv_message` để giữ label/data
  của reply/request thay vì chỉ error/value.
- Rust server giữ bảng 4 file trong static storage; `/motd` là file read-only
  khởi tạo, `/scratch` hỗ trợ create/write/read/close. Write dùng IPC buffer
  client → server; read trả tối đa 24 byte trong reply data.
- Client bỏ Open/Read/Write/Close syscall cho demo file và dùng VFS IPC; kernel
  ramfs vẫn được giữ cho selftest/ABI fallback.
- Harness thêm `VFS SERVER OK`, đúng một `VFS OK`, và cấm `VFS BAD`; toàn bộ
  legacy string và 4 USER FAULT vẫn giữ nguyên.
- `vifast-kernel` 80/80; PTE 11/11; ELF 17/17; build không warning; QEMU 3/3
  PASS ×3. DATA UNTYPED test-exit 35/36/40 trang, 20/24 object.

## 8.2b — Notification capability transfer demo

- Commit `395a589 feat: demo notification capability transfer over call`.
- Client dùng `call_with_cap` cho lần Call đầu, chuyển notification cap slot 5
  sang server recv slot 4; server `signal(4)`; client `Poll(5)` thấy tín hiệu.
- Các Call sau dùng `CAP_NONE`; server vẫn signal notification đã nhận.
- `vifast-kernel` 80/80; QEMU 3/3 PASS ×3.

## M1 — VSpace VA range allocator

- Commit `562727d feat: add first-fit vspace range allocator`.
- `VSpace::find_free_va` quét reverse-map 32 mapping để tìm khoảng trống đầu tiên
  trong `[base,end)`, thay vì chỉ bump `next_va`; `FrameMap` hint 0 dùng allocator.
- Hole được tái sử dụng sau `FrameUnmap`/revoke; mapping explicit vẫn giữ nguyên.
- Thêm test map explicit + hint 0 + unmap + map lại; `vifast-kernel` 80/80.
- QEMU 3/3 PASS ×3 sau thay đổi.

## 8.4b — Runtime boot modules qua BootInfo

- Commit `6963f58 refactor: supply rust modules through bootinfo`.
- BootInfo tăng 32 → 64 byte, thêm `server/client_elf_va/len`. Runtime cấp
  Frame từ data Untyped, copy bytes ELF bằng kernel, map read-only vào
  `ROOT_MODULE_SERVER_VA`/`ROOT_MODULE_CLIENT_VA`, rồi root parse các slice này.
- Root ELF không còn `include_bytes!` module; root ELF giảm còn RISC-V 12880 B,
  AArch64 12704 B, x86 16744 B. Build module ELF trước, kernel nhận env
  `VIFAST_SERVER_ELF`/`VIFAST_CLIENT_ELF`.
- Allowlist user-copy mở module staging window; `FrameWrite` copy bytes vào Frame
  con, không cần mapping staging phụ.
- QEMU 3/3 PASS ×3; `vifast-kernel` 79/79, PTE 11/11, ELF 17/17; data Untyped
  33/34 trang, 18/48 object; RISC-V headroom 569328 B.

## S1 — syscall fuzz trên 3 PTE format

- Commit `ed40921 test: fuzz syscalls across pte formats`.
- Thêm test deterministic 10.000 bước cho mỗi RISC-V/x86_64/AArch64 PTE fake:
  Retype, CapCopy, CapDelete, CapRevoke, FrameMap/Unmap, PageTableMap với seed
  cố định; chạy dưới `WALK_LOCK` để không tranh fake memory với test khác.
- Mỗi bước gọi `check_invariants()`; yêu cầu hơn 100 syscall thành công mỗi
  format để tránh test rỗng. Không panic, không vi phạm PA uniqueness/W^X/
  reverse-map.
- `vifast-kernel` 78/78; PTE 11/11; ELF 17/17; build không warning; QEMU 3/3
  PASS ×3.

## S2 — x86 kernel data NX

- Commit `82c9c11 feat: mark x86 kernel data non-executable`.
- Bật EFER.NXE sau khi CPUID xác nhận NX; tách các leaf 2 MiB chứa static data
  thành bảng 4 KiB, đánh dấu NX cho stack, GDT/TSS/IDT, context, boot/data/
  metadata pool, `Kernel` storage, page table và split-table pages.
- Text page vẫn executable; `check_kernel_mappings` kiểm tra thêm NX cho toàn bộ
  data range nếu NX được bật. QEMU x86 in `KERNEL DATA NX OK`, harness yêu cầu
  riêng x86.
- `vifast-kernel` 77/77; PTE 11/11; ELF 17/17; build không warning; QEMU 3/3
  PASS ×3. Các kiến trúc khác không đổi.

## 8.1b — IPC buffer demo end-to-end

- Commit `49e7b1f feat: demo ipc buffer transfer across tasks`.
- Root cấp hai Frame IPC buffer từ data Untyped, map RW vào VSpace server/client
  tại `IPC_BUF_SERVER_VA` / `IPC_BUF_CLIENT_VA`, rồi gọi `TcbSetIpcBuffer`.
- Client ghi `IPC BUF OK\n` vào buffer trước vòng Send; server đọc buffer sau
  ReplyRecv và in qua DebugWrite. Kernel copy 32 word giữa hai PA thật sự.
- Sửa lỗi nền tảng: `copy_frame_to_frame` trước đây dùng default no-op trong cả
  ba platform; nay copy trực tiếp PA. Host fake đã test từ trước nhưng QEMU chưa
  đi qua implementation thật.
- FrameMap đầu của client đổi hint 0 → `DATA_VA` để buffer root map không đẩy
  data frame ra ngoài allowlist của DebugWrite.
- Read/write user allowlist thêm hai IPC buffer VA cho cả ba platform.
- `vifast-kernel` 77/77; PTE 11/11; ELF 17/17; build không warning.
- QEMU 3/3 PASS ×3; ROOT SWITCH 23754 / 27659 / 9203 (lần đầu).
  Boot Untyped RISC-V 19/32, AArch/x86 21/32; data test-exit 23/64 RISC-V,
  24/64 AArch/x86, 8/48 object.

## 8.28 — FS v2, block cap, per-hart tick, CapMint (vòng 17)

- `85cbdb9`: tick theo hart; block cap; FS v2 journal; server VFS v2; fsutil.
- Host `vifast-kernel` 90/90, `vifast-fs` 7/7; QEMU 10/10 và 50/50 cả 3.
- `test-panic-hart1` 5/5 mỗi kiến trúc; AArch rc=1, x86 rc=7, RISC rc=0
  (QEMU sifive_test không mang exit-code — nợ R2).
- Persistence 6 phase × 3 PASS; harness 263 case âm.

## 8.29 — RISC-V panic, D6 badge, GetTime, FS small, per-VA TLB (vòng 18)

- `2851cdf`: RISC-V panic ghi `(1<<16)|0x3333`; QEMU rc=1 (5/5).
- `763f6ff`: D6 badge register a7/x7/r10, CapMint source badge0, server badge0 reject.
- `5e9c11c`: GetTime dùng `ticks[0]` toàn hệ thống.
- `2ea3944`: `vifast-fs` FormatOptions + ảnh 48/49 block, ENOSPC, dir 1 block.
- `111e072`: per-VA TLB + `harts_loaded`, TLB_VA mailbox, 50/50 cả 3.
- Final: build 0 warning; QEMU 10/10 và 50/50 cả 3; persist 6/6; panic 5/5;
  handoff 10/10; harness 300 case; RISC headroom `138672` B.

## 8.30 — Headroom, TLB mutation, CapDelete, suspend, FS v3, GetTime fast (vòng 19)

- `334e5c1`: root cspace 96 + task cspace 64; RISC headroom 392.624 B; guard 192 KiB.
- `4688353`: fuzz seed 100k × 3 PTE + invariant.
- `d8b762e`: FS v3 single-indirect, file 69 KiB, 12 host test, persist phase 7.
- `aa10090`: GetTime atomic fast path before BKL.
- `b029aac`: TLB mutation (`TLB REMAP BAD` 5/5 RISC/x86), CapDelete, TcbSuspend/TcbSetPriority.
- `8342553`: test-exit chờ FS LARGE để tránh race.
- Final: QEMU 10/10, 50/50 cả 3; mutation 5/5; panic 5/5; handoff 10/10;
  persist 7 phase ×3; harness 327 case; 0 warning; headroom 392.624 B.

## 8.45 — Per-hart scheduler, VFS bulk, hard link (vòng 20)

- `c37982e`: ready queue theo hart + work stealing; invariant chống trùng queue;
  host test home/steal/affinity move; worker E priority 5 để preempt ổn định.
- `925f633`: VFS READ qua IPC buffer, `MAX_IO` 24→128; client ghi/đọc 20 KiB thật;
  FS hard link + fsutil link + persist phase 7 đọc lại `/p/large2`.
- Final: QEMU 10/10, 50/50 cả 3; mutation 5/5 RISC/x86; panic 5/5;
  handoff 10/10; persist 7 phase ×3; harness 336 case; 0 warning;
  headroom 392.624 B; Kernel 613.720 B.

## 8.55 — VFS write-through block cache (vòng 21)

- `vifast-fs`: thêm `CachedDev<D>` 8 slot x 512 B, write-through, thống kê
  `hits/misses/writes`; clear cache sau format để không giữ superblock cũ.
- VFS server dùng `Fs<CachedDev<ServerDev>>`; không đổi ABI/userland.
- Host test mới: cache hit/write-through, eviction round-robin, bọc filesystem.
- `cargo test -p vifast-fs`: 19/19; QEMU 3/3; persist 7 phase ×3; 0 warning.

## 8.60 — Real fsync/block flush (vòng 21)

- Thêm `BlockFlush = 45`, `UserAction::BlockFlush`, `Platform::block_flush`.
- VirtIO: `VIRTIO_BLK_T_FLUSH` (type 4); ATA PIO: lệnh `0xE7`.
- `BlockDev::flush` + `Fs::fsync`; server `FSYNC` không còn trả OK giả.
- Client gọi FSYNC sau khi ghi file lớn, hard link và symlink.
- Host test kiểm `fsync` gọi flush; QEMU 3/3; persist 7 phase ×3; 0 warning.

## 8.65 — VF04 double-indirect (vòng 21)

- Format mới `VF04`: 10 direct + single-indirect + double-indirect; file tối
  đa 16.522 block (8,4 MiB lý thuyết, thực tế bị giới hạn bởi ảnh 4 MiB).
- `VF03` cũ vẫn mount với geometry 11 direct + single (69 KiB), không migrate
  âm thầm.
- `file_block`/`set_file_block`/`truncate`/`free_inode_blocks`/`fsck` xử lý
  bảng cấp hai; host test mới đọc/ghi 200 block, truncate qua ranh giới single
  và double, kiểm table được giải phóng.
- `cargo test -p vifast-fs`: 21/21; QEMU 3/3; persist phase 7 dùng file 80 KiB
  (double-indirect) trên cả 3 kiến trúc; 0 warning.

## 8.70 — Sleep/timeout primitive (vòng 21)

- Thêm `ThreadState::BlockedSleep`, `Tcb.wake_at` và syscall `Sleep = 46`.
- `tick()` gọi `wake_sleepers()`; task hết hạn được đưa vào ready queue với
  resume message `Ok`.
- User wrapper `vifast_user::sleep` poll `GetTime` khi không có task khác trên
  BSP để tránh runtime không có idle TCB; client in `SLEEP OK`.
- Host test mới: block, chưa hết hạn, wake đúng tick, resume message.
- QEMU 3/3 lặp 3 lần; harness thêm `SLEEP OK`; 0 warning.

## 8.75 — TCB restart + bind notification (vòng 21)

- `TcbBindNotification` (21): dùng `bind_notification`, kiểm tra cap TCB và
  Notification; test host.
- `TcbRestart` (18): reset entry/stack, clear pending/fd/wake, trả
  `UserAction::RestartTcb` để runtime dựng lại context; reject self/running.
- `cargo test -p vifast-kernel`: 104/104; ABI còn 6 placeholder (CNode,
  SchedContext, IRQ).

## 8.80 — BSP idle TCB + blocking Sleep (vòng 21)

- Root tạo một idle TCB hart 0 (`idle_main`, priority 0, stack
  `IDLE_STACK_BASE..TOP`); root VSpace map thêm một frame.
- Nhờ idle TCB, `BlockedSleep` không còn làm scheduler hết task chạy; user
  wrapper gọi syscall Sleep và chỉ fallback polling khi `WouldBlock`.
- Root tạo idle sau fault4; TCB/object quota vẫn đủ (DATA UNTYPED 63/64).
- QEMU `--repeat 3` 3/3 mỗi kiến trúc; persist 7 phase ×3; 0 warning.

## 8.85 — O_TRUNC + fsutil utilities (vòng 21)

- ABI thêm `O_TRUNC = 8`; VFS server cắt file về 0 khi mở, từ chối thư mục.
- Client dùng O_TRUNC cho file lớn/hard link/symlink để demo không phụ thuộc
  dữ liệu cũ trên ảnh đĩa.
- `vifast-fsutil` thêm `mkdir`, `rm`, `mv`, `stat`, `truncate`; test tay
  mkfs → mkdir → put → stat → mv → truncate → get → rm → fsck PASS.
- QEMU 3/3; 0 warning.

## 8.90 — Console input + shell (vòng 21)

- `Arch::getc` cho UART 16550/PL011/COM1; syscall `ConsoleRead = 47` trả byte
  hoặc `WouldBlock`.
- Client có shell polling: `help`, `time`, `echo`, `ls`, `cat`, `mkdir`, `rm`,
  `mv`, `ln`, `stat`, `sync`, `truncate`, `fsck`.
- QEMU test-exit vẫn dừng sau demo; shell in `shell> ` nhưng không cần input.
- Shell mới làm client ELF lớn hơn; tăng Data Untyped 256→288 KiB,
  `DATA_MAX_OBJECTS` 64→72, `MAX_OBJECTS` 136→144. RISC headroom còn 261.536 B.
- Build/QEMU `--repeat 3` 3/3 cả 3; persistence 7 phase ×3; 0 warning.

## 8.95 — Local regression gate (vòng 21)

- Thêm `scripts/check-all.sh` thay CI hosted: format, build/host test,
  QEMU 3 lần mỗi kiến trúc, persistence, harness âm, docs.
- README hướng dẫn chạy gate; không phụ thuộc GitHub Actions.

## 8.96 — SchedContext budget scheduler (vòng 22)

- ABI `ObjType::SchedContext` có handler thật; thêm `ThreadState::BlockedBudget`.
- Kernel lưu budget/period/remaining/period_start/bound; retype SchedContext,
  configure 1..period, bind một-một với TCB.
- `tick()` trừ budget của TCB đang chạy; hết budget thì block tới hết period,
  `wake_budgeters()` nạp lại budget và đưa về ready queue (không ghi resume).
- Destroy/revoke context tự unbind; TCB đang BlockedBudget được đánh thức.
- Root retype context 1000/1000, bind worker C, in `SCHED CTX OK`; harness
  yêu cầu dòng này.
- Host test 106/106; QEMU --repeat 3 3/3; persistence 7 phase ×3; 0 warning.

## 8.97 — IRQ objects + timer IRQ delivery (vòng 22)

- ABI thêm `ObjType::Irq`, logical `IRQ_TIMER=0`/`IRQ_UART=1`; status 3 syscall
  IRQ chuyển sang implemented.
- Kernel object IRQ có number, notification, pending; `irq_raise` coalesce
  edge, `IrqSetNotification` signal ngay nếu đang pending, `IrqAck` xoá pending.
- `IrqGet` chỉ root được gọi; root copy cap IRQ + notification cho client.
- Runtime raise `IRQ_TIMER` mỗi tick; client Wait/Ack và in `IRQ OK`; harness
  yêu cầu `IRQ OK`, cấm `IRQ BAD`.
- Host test 108/108; QEMU 3/3 (một lần), persistence giữ 7 phase ×3 dự kiến.
- UART external IRQ mới có API/logical number, chưa route PLIC/GIC/PIC.

## 8.98 — UART IRQ + IRQ-driven shell (vòng 22)

- `RxQueue` lock-free 64 byte trong `vifast-arch`, `stash_rx` cho cả 3 port.
- RISC-V: map PLIC 0x0c000000 + context 0x0c200000, source 10 context 1,
  claim/complete đúng offset +4; handler drain UART → `IRQ_UART`.
- AArch64: GIC SPI 33 target CPU0, PL011 RXIM; handler drain/ICR/EOI.
- x86_64: IDT vector 0x24, unmask PIC IRQ4, COM1 IER bit0; handler drain/EOI.
- Root tạo thêm UART IRQ + notification, copy cap cho client; shell block trên
  notification UART rồi Ack/ConsoleRead thay vì yield-poll.
- Manual `-serial stdio -monitor none` với kernel no-default: input được echo
  trên RISC-V, AArch64, x86_64.
- Host test `vifast-arch` 7/7; QEMU chuẩn 3/3 (không input); 0 warning.

## 8.99 — Flat CNode cspace (vòng 22)

- Thêm object CNode 64 slot phẳng (chưa guard/nhiều cấp), `Tcb.cnode`
  thay thế bảng cap nhúng khi được cài.
- `CNodeMint` (12): mint/derive cap từ cspace caller vào slot CNode.
- Syscall mới `CNodeInstall` (48): cài/bỏ CNode cho TCB
  Inactive/Suspended/Faulted; CNode đang dùng không destroy được.
- CNode revoke/delete/invariant/accounting được nối vào đường cap hiện có.
- Host test 110/110; QEMU 3/3; ABI 49/49 implemented, 0 placeholder.

## 8.995 — CNode demo trong QEMU (vòng 22)

- Root tạo CNode 64 slot, mint endpoint vào slot 1, CNodeInstall lên TCB
  demo và resume.
- Worker chỉ có một cap trong CNode, gửi `CNODE` label; root in `CNODE OK`.
- Harness yêu cầu đúng một `CNODE OK`; TCB/object quota còn dư
  (DATA UNTYPED 68/72 object 64/72).

## 8.997 — VFS O_APPEND (vòng 22)

- ABI thêm `O_APPEND = 0x10`; server giữ flags trong fd và ghi tại
  `Fs::info(ino).size`, bỏ qua offset client.
- Client test `/append`: ghi "a", ghi "b" tại offset 0, stat=2, đọc "ab";
  in `FS APPEND OK`, harness yêu cầu đúng một dòng và cấm `FS APPEND BAD`.
- QEMU --repeat 3 3/3; persistence 7 phase ×3; 0 warning.

## 8.998 — Write-back block data (vòng 22)

- `CachedDev` giữ dirty bit: data LBA >= DATA_START ghi vào cache; journal/
  metadata (< DATA_START) vẫn write-through để bảo toàn thứ tự.
- Eviction dirty ghi lại; `CachedDev::flush` flush dirty + inner device.
- `Fs::commit` flush trước khi xoá journal commit; `replay` cũng flush trước
  khi clear commit. Vì vậy crash không mất data sau khi journal đã commit.
- Host test `vifast-fs` 22/22; QEMU 3/3; persistence 7 phase ×3; 0 warning.

## 8.999 — Pager fault-resume demo (vòng 22)

- Pager TCB root VSpace đọc `PAGER_VA` chưa map; fault endpoint gửi root.
- Root map Frame bằng `map_with_tables`, rồi `TcbResume` pager; pager chạy
  lại entry, đọc value, gửi label; in `PAGER OK`.
- Harness yêu cầu 5 USER FAULT (4 fault cũ + pager) và `PAGER OK` đúng một lần;
  `PAGER BAD` bị cấm.
- QEMU --repeat 3 3/3; persistence 7 phase ×3; host kernel 111/111.

## 9.00 — User-space pager server (vòng 22)

- Tăng Data Untyped 288→304 KiB (`DATA_MAX_OBJECTS` 72→76,
  `MAX_OBJECTS` 144→148) để chứa server/stack/page table.
- Pager TCB dùng fault endpoint riêng; user-space pager server nhận message,
  FrameMap vào root VSpace, TcbResume pager, rồi nhận label và in `PAGER OK`.
- Root pre-map/unmap page để tạo page table trước; server không cần
  PageTable mới khi xử lý fault.
- QEMU 3/3; persistence 7 phase ×3; harness 387 case; 0 warning.

## 9.01 — TCB reap, O_EXCL/O_DIRECTORY, pager hai client (vòng 22.1)

- Thêm `TcbDestroy` (#49), fault message mang fault endpoint slot + TCB index;
  root reap 4 fault TCB + CNode demo, in `TCB REAP OK`.
- `O_EXCL` và `O_DIRECTORY` đầy đủ ở VFS server; test `FS OPEN FLAGS OK`.
- User-space pager server phục vụ hai client (`PAGER OK`, `PAGER2 OK`);
  harness nâng lên 6 fault record.
- Nới metadata lên 80 KiB, object table 147, TCB 18, data 76 page; vẫn còn
  headroom 200.016 byte.
- Host kernel 112/112; QEMU repeat 3/3; persistence 7×3; harness 417 case.

## 9.02 — VFS stat-by-path (vòng 22.2)

- Thêm request `STAT_PATH` (0x2011) vào protocol VFS server.
- Client có `vfs_stat_path`; test `/motd`, `/`, path không tồn tại.
- Output mới `FS STAT PATH OK`; harness 426 case.
- QEMU repeat 3/3, persistence 7×3; kernel 112/112.

## 9.03 — Vòng 23: ObjectDestroy + reclaim Untyped

- Pha A: sửa 4 warning `function_casts_as_integer` ở `vifast-root` (0 warning);
  viết lại `docs/11-hien-tai/09-testing.md`; bổ sung crate `vifast-fs`,
  `vifast-fsutil`, `vifast-virtio`, `vifast-acpi`, `vifast-server`,
  `vifast-client` vào bảng crate; sửa mô tả NUMERIC "5" → "6 USER FAULT"
  (giữ `accept()` = 6); gắn `[HISTORICAL]` cho mục changelog và sửa số liệu
  hiện tại trong summary; cập nhật SHA log + sinh bundle 21, 22.
- Pha B:
  - `ObjectDestroy` = syscall 50; chỉ nhận cap gốc (`cap.parent ==
    object.creator`), từ chối bản copy/mint (`NoRights`), còn cap khác
    (`HasDependents`), đang dùng (`Busy`); mã lỗi mới 20/21 ở cuối enum.
  - Hỗ trợ Frame/Endpoint/Notification/PageTable/VSpace/CNode/SchedContext;
    Tcb/Untyped/Irq/Iommu/BlockDevice trả `InvalidArg`.
  - Reclaim Untyped: `untyped_take` ưu tiên vùng free đúng kích thước, rồi
    first-fit, rồi bump; `untyped_reclaim` hạ watermark/gộp vùng, free list
    16 entry, `leaked` khi đầy; object lưu `alloc_offset/alloc_len`.
  - 15 host test mới (ObjectDestroy + reclaim + zero-on-reuse).
  - Demo root: 100 vòng Frame map/write/unmap/destroy + 10 VSpace; thử destroy
    Frame đang map (`Busy`) và slot cũ (`InvalidCap`); in
    `DESTROY OK (frames=100, vspaces=10, reused=100, leaked=0)`.
  - Harness thêm `DESTROY OK` (exact one), `DESTROY BAD` (forbidden): 435 case.
- Hạ boot stack RISC-V 256→192 KiB để giữ headroom sau khi thêm code; hiện
  `kernel_end=0x803d02b0`, headroom 261.456 byte.
- Sửa flaky phát hiện khi demo đặt trước `ROOT_READY`: 200 map/unmap sinh IPI
  TLB khi hart phụ đã load root VSpace, làm timeout RISC-V và lệch timer x86.
  Chuyển demo lên trước `spawn_idle` (chưa TCB phụ nào dùng root VSpace) và
  chọn VA trong span 2 MiB của root ELF; thêm free list nhỏ cho SlotAllocator
  để worker E trên x86 không hết slot.
- Pha C:
  - `-smp 4`: RISC-V/AArch64 boot PASS, chỉ 2 hart online (HSM/PSCI start 1
    hart phụ); x86_64 panic `AP EXCEPTION`/exit 7 vì mới có trampoline cho một
    AP.
  - Soak `--repeat 10`: 10/10 cả 3 kiến trúc, 0 flaky, 2m21s.
  - `docs/design/pager-v2.md`: protocol MAP/UNMAP/EVICT/FAULT_REPLY, cap
    VSpace/PageTable/TCB, evict = unmap + ObjectDestroy, test plan `PAGER3 OK`.
  - Khảo sát `static mut` AArch64: bảng biến/người ghi/bảo vệ trong báo cáo
    vòng 23; không có data race nếu giữ bất biến per-hart/1-writer.
- Số liệu cuối: kernel 127/127 (+1 fuzz ignored), tổng host test 192;
  harness 435; QEMU 3/3 ×3; persistence 7×3; 0 warning; `Kernel=710248`,
  `Object=4752`, `Tcb=4696`, `Cap=64`; SYSCALL_STATUS 51/51.
- SHA-256 `ViFastOs-architecture-summary.txt`: `b9d474384dc9e1e2304063ce40cb95bdd22d4ff507315292ad787cea3c2c52e8`.

## 9.04 — Vòng 24: củng cố v23, boot stack, fuzz reclaim

- A1 (R3): CONTEXTS cả 3 platform đủ `MAX_OBJECTS` (147) + const assert;
  sửa `check_kernel_mappings` x86 (trước khai 32 frame).
- A2 (R2): destroy VSpace flush `asid` theo `harts_loaded` trước khi trả bảng;
  `take_asid` bỏ qua ASID đang sống; 2 host test.
- A3 (R1): feature `test-destroy-smp` (root variant, build.sh 3 arch);
  demo 20 frame + 2 VSpace chạy sau khi worker C chạy trên hart 1, thử
  destroy Frame đang map → `Busy`; `DESTROY SMP OK (frames=20, vspaces=2)`;
  5/5 × 3 kiến trúc.
- A4 (R4): chênh 74/76 vs 69/76 là so sánh x86 với RISC-V; demo on/off giống
  hệt từng kiến trúc (69/66, 72/68, 74/70) — không rò rỉ. Ngân sách pager v2
  ~18 trang trong khi x86 trống 2, TCB 17/18 → pha B blocked.
- A5 (R5): paint 0xA5 boot stack; `BOOT STACK used=21736/196608` (RISC-V),
  `21800/262144` (AArch64), `21760/196608` (x86); harness bắt buộc U*2<=S +
  12 case âm (462 case).
- A6 (R8): fuzz Retype+ObjectDestroy+CapRevoke trộn kích thước, 4 object sống;
  2.000 bước thường, 100.000 bước `--ignored` (4,8 s), leak=0. Sửa cascade
  watermark; ghi nhận alignment padding là waste, không phải leak.
- A7 (R6): sửa summary mục 2/7/11/16/20 + root ELF x86 29.440; `check-docs`
  cấm 9 câu cũ và kiểm Busy/HasDependents. Chưa sinh bảng lỗi/giới hạn tự động.
- A8 (R7): `kernel_end=0x803c22b0`; make-bundle tự lấy docs commit từ git và
  thêm Part 4 (`docs/design/*.md` + SHA từng file).
- Pha B: BLOCKED do ngân sách trang/TCB; đề xuất data 368 KiB, boot 96 KiB,
  MAX_TCBS 22 cho vòng 25.
- C1: BLOCKED (multi-AP x86 cần trampoline thứ 2).
- C2: BLOCKED (refactor static mut cần vòng riêng).
- C3: soak 10/10 × 3 (thường) và 10/10 × 3 (feature), 0 flaky.
- C4: `docs/design/display-input-v1.md` (ramfb + virtio-keyboard + GOP).
- Số liệu: kernel 131/131 (+2 ignored), tổng host 196; harness 462;
  QEMU 3/3 × 3; persistence 7×3; 0 warning; headroom 253.264;
  `Kernel=710248`, `Object=4752`, `Tcb=4696`, `Cap=64`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `aa8e7f550fcf95dd2fdf32a4f75b9d1de3a440f35af4b96a17c03c56fd0cc86e`.

## 9.05 — Vòng 25: sổ sách, pager protocol, static mut từng phần

- A1: sửa mục 12/15/16, khôi phục HISTORICAL "7.5 đúng 4 USER FAULT" + dòng
  nâng lên 6 ở vòng 22.2; dời "74. Vòng 24" vào mục 18.
- A2: `check-docs --self-test` chèn 8 câu cấm vào bản tạm, tất cả phải FAIL;
  bảng `errors` (KError + số) và `limits` (MAX_*) sinh tự động từ code.
- A3: render "Boot stack", "Fault rule 6+F", "Feature test-destroy-smp" vào
  khối generated pass; status từ chối repeat≠3 hoặc thiếu kiến trúc.
- A4: BLOCKED — data 368 KiB cần +65.536 nhưng headroom còn 187.728; tăng
  MAX_OBJECTS 163 tốn thêm 76.032 → 144.464; giữ 147 slot thì data chỉ được
  76 object, không đủ cho pager. Không áp dụng thay đổi.
- B1: protocol pager v2 trong `vifast-abi` (badge, VA window, Fault/Map
  encode/decode, 4 host test); `pager-v2.md` sửa theo S7 (READ-only, Untyped
  con, ObjectDestroy+Retype, VFS badge riêng).
- B2–B4: BLOCKED do A4; B5: SKIPPED.
- C1: `TIMER_INTERVAL` → AtomicU64, `KERNEL_PTR` → wrapper UnsafeCell 3 nền
  tảng, 0 warning; phần static mut còn lại để vòng sau (PARTIAL).
- C3: sửa display-input (fw_cfg/ramfb PA liền tục, GOP ngoài 1 GiB, x86 PS/2,
  screendump checksum).
- C2: soak 10/10 ×3 thường + 10/10 ×3 feature; sau đó repeat 3 chuẩn.
- Số liệu: host 200 (kernel 131, abi 4); harness 462; QEMU 3/3 ×3; feature
  5/5 ×3; persistence 7×3; 0 warning; headroom 253.264 (`kernel_end=0x803c22b0`).
- SHA-256 `ViFastOs-architecture-summary.txt`: `e381d5a20a921c325a868d8ccef79630823e3db69a6308ee7e7e0e61c81e6444`.

## 9.06 — Vòng 27: M0.2 sổ sách + đo M1.1

- C1: khôi phục HISTORICAL "tổng 4 USER FAULT"; đính chính hai fault
  0x1001000000/0x1001001000 là PAGER_VA/PAGER2_VA (xác minh từ
  vifast-root); sửa boot Untyped 24/27/28 trang.
- C2: khối limits về mục 7, ghi kèm crate; giải thích MAX_FDS/MAX_FD/MAX_FILES.
- C3: status docs đọc `target/qemu-status-standard.json`, chặn file cũ hơn build.
- M1.1 PARTIAL: host test in variant (Object 4.752, Tcb 4.696, CNode 4.096,
  VSpace 3.136, Kernel 710.248) + `CTX SIZE` 280/288/160. POOL HWM BLOCKED:
  thêm bộ đếm vào kernel làm AArch64/x86 in KERNEL EXCEPTION sau BOOTINFO OK
  (3 biến thể đã thử, đều revert).
- Baseline 3/3 ×3; check-all lần 1 timeout persistence x86 phase 7, chạy lại
  persistence PASS 3/3, check-all lần 2 ALL CHECKS PASS.
- Harness 462; host 201 (kernel 132, abi 4); 0 warning; headroom 253.264.
- SHA-256 `ViFastOs-architecture-summary.txt`: `6945f686b739f9296955dfdf5995b1d802d0bbb6ca45934b4b147c237dd01d0a`.

## 9.07 — Vòng 28: M1.2 xong, thử M1.3

- M1.2a/b/c: thêm context slot dense, cấp slot mọi đường tạo TCB, runtime +
  platform dùng slot. Giải được ROOT ERR 16 (runtime còn index CONTEXTS theo
  tcb.index). Headroom 253.264 → 290.128 (`kernel_end=0x803b92b0`).
- M1.1 HWM: thử biến thể 4 (runtime read-only sampling) vẫn lỗi AArch64 →
  BLOCKED 2 phiên, ghi inbox.
- M1.3 TCB pool thử lần 1: `Kind::Tcb(TcbId)` + `Kernel.tcbs[22]`, host
  134/134, Object 4.152, Kernel 706.576, headroom 286.032; QEMU RISC-V 3/3
  nhưng AArch64 0/3 (KERNEL EXCEPTION vector=4) và x86 2/3 → revert.
- Baseline sau revert: QEMU 3/3 ×3; host 203; harness 462; 0 warning.
- SHA-256 `ViFastOs-architecture-summary.txt`: `5c8acc6a410634a1b2e8bda644b28eb68c8a310d656dc2a13b5d12083c6bce58`.

## 9.08 — Vòng 29: baseline + đo layout, khoanh vùng kẹt AArch64

- Cây dở CNode pool (`Kind::CNode(CNodeId)` + `Kernel.cnodes[8]`, stash riêng):
  RISC-V 3/3 nhưng AArch64 0/3 `KERNEL EXCEPTION vector=4 ELR=0x4009201c`
  sau `BOOTINFO OK`; baseline sạch AArch64 PASS → lỗi do layout, không phải logic.
- Phát hiện: tách CNode riêng không giảm `Object` (CNode 4.096 vẫn nằm trong
  pool) mà phình `Kernel` 710.248 → 743.016 (+32.768). Hướng đúng là giảm
  `MAX_CAPS_TASK` hoặc tách Tcb/VSpace.
- Bước layout probe: host test in thêm `LAYOUT objs=0 root_cspace=698544
  current=705552 size=710248`; QEMU 3/3 ×3, persist 7×3, harness 462, 0 warning.
- Xem `docs/agent2-report-29.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `2dd522f6d5579fdd89ac32501928895586dc0ce04fc1b10ee71f33173ed6e2ad`.

## 9.09 — Vòng 30: root cause AArch64 + guard tràn static

- Root cause M1.1/M1.3: `Kernel` pool 743.016 B làm `KERNEL_STORAGE`
  (`0x400B7000` .data) tràn qua `BKL` (`0x40164670`), `KERNEL_PTR`
  (`0x40171120`); `kernel.reset()` ghi đè → `KERNEL_PTR=0` → DABT
  `FAR=0xb4410` (= offset `current`) sau `BOOTINFO OK`. Baseline 710.248 B
  vừa khít trước BKL 8 byte.
- Giữ hunk runtime `set_kernel` sớm (xanh 3/3); revert toàn bộ pool logic.
- Thêm `KERNEL_STATIC_BUDGET=716.800` trong `boot()`: exit rõ ràng thay vì
  exception mù khi `Kernel` phình.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 462, host 203, 0 warning.
- Xem `docs/agent2-report-30.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `9865606d3980d5122a5600a0a50bc5b8c2a0eaa47ef8707b689dd367da522c5e`.

## 9.10 — Vòng 31: giảm MAX_CAPS_TASK 64→48

- Audit: user tối đa slot 11, CNode dest 1, host test tới 36 → 48 an toàn.
- Object 4.752→3.728, Tcb 4.696→3.672, CNode 4.096→3.072;
  Kernel 710.248→559.720 (tiết kiệm 150.528 B, dưới budget 716.800).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 462, host 203, 0 warning.
- Docs: `00-hien-trang`, `11-hien-tai/02`, `README` (2 chỗ) 64→48.
- Xem `docs/agent2-report-31.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `b20a8d12514acb51509132ae1094529cf442a578698293717b15f66fdea82785`.

## 9.11 — Vòng 32: budget dùng chung, mở mục tiêu dài hạn

- Thêm `vifast_kernel::KERNEL_STATIC_BUDGET=716.800`; runtime dùng chung,
  bỏ const cục bộ. Không đổi hành vi.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 462, host 203, 0 warning.
- Mở goal dài hạn M1→M3: static an toàn → pager v2 → net → display/input.
- Xem `docs/agent2-report-32.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `46350433f8736abeeae093c695b63f1400ad948ad6249d0bb5f7c9a685c63867`.

## 9.12 — Vòng 33: check tràn static AArch64 trong build

- `scripts/build.sh` đo `KERNEL_STORAGE` + size vs `BKL` bằng `llvm-nm`,
  fail nếu đè nhau. Hiện khít 8 byte, PASS. Guard 2 lớp với runtime budget.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 462, host 203, 0 warning.
- Xem `docs/agent2-report-33.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `92519e179d77cada3fa19455fab26a3578a6bbe80e07a8981e7254c60cae5aa2`.

## 9.13 — Vòng 34: pager evict demo (pha 2 bước 1)

- Server unmap `PAGER2_VA` sau lần đọc đầu; client đọc lại → fault lần 2 →
  map lại cùng frame. Marker `PAGER EVICT OK` exact-one, `PAGER EVICT BAD`
  forbidden. Fault 6→7 (`PAGER2_VA` ×2); harness 462→471 cases.
- PAGER3 thử nhưng stash (hết slot root AArch64/x86); evict không tốn slot.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 471, host 203, 0 warning.
- Xem `docs/agent2-report-34.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `172cbed0e2d7f6f1a9baf0653476a7f1011ccfd4b0ff2dcaf039b4ed89c0be0c`.

## 9.14 — Vòng 35: pager retype frame mới (pha 2 bước 2)

- Root cấp Untyped con 4 KiB/2 obj từ meta cho pager slot 7.
- Evict đầy đủ: unmap + destroy + retype frame mới + frame_write lại giá trị.
  Dùng meta vì data chật trên x86.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 471, host 203, 0 warning.
- Xem `docs/agent2-report-35.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `3eddf2395734c016b160f37d11e4b6defab9b0cc05aec55b2aa3181ae5b154f5`.

## 9.15 — Vòng 36: nâng trần slot root 96→128

- Root spawn pager/VFS cạn slot 96 (blocker PAGER3 vòng 34). Tăng root rẻ
  hơn tăng task (1 root, +2 KiB Kernel 559.720→561.768, dưới budget).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 471, host 203, 0 warning.
- Docs: `00-hien-trang`, `11-hien-tai/02`, `README` 96→128.
- Xem `docs/agent2-report-36.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `e33a9de11077d730c69b410217391d07a22ef8eec7060f90a24166e4c32a055b`.

## 9.16 — Vòng 37: PAGER3 client thứ ba (pha 2 bước 3)

- ABI stack 0x1e000/0x1f000; `pager3_client` fault `0x1001002000`;
  server slot 8/9, marker `PAGER3 OK/BAD`. Gộp từ stash vòng 34.
- Harness: fault 7→8, 471→483 cases. Docs `00-hien-trang` 2→3 client.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-37.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `6400792c50b07a3ed35806364d70dd0be08516703009dcd12613fcd0e510cb69`.

## 9.17 — Vòng 38: tổng kết pha 2 pager (không đổi code)

- Đánh giá demand-load qua VFS: pager cần endpoint + IPC buffer + slot
  (~4 slot root/task), rủi ro cao khi slot đã chật → dừng ở MVP.
- Pha 2 đạt: 3 client, evict unmap + destroy + retype, `PAGER EVICT OK`.
  Chưa có: nạp file qua VFS, PAGER3 evict, dirty tracking.
- Không đổi code; cổng giữ nguyên vòng 37.
- Xem `docs/agent2-report-38.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `1e16538530324ebb1f80d9ad33087160a2c997499cebbd93fd745e9c17541793`.

## 9.18 — Vòng 39: VirtIO scan API (pha 3 bước 1)

- `vifast-virtio`: `DEVICE_NET/BLOCK` + `scan_devices` duyệt MMIO window.
  Platform AArch64/RISC-V log `VIRTIO DEV` mỗi slot trong `init_block`.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-39.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `1899e78bc39ebefb3c3ec64c37f2a1574d5cae15cf8fed1b044b04db5e21baaf`.

## 9.19 — Vòng 40: giữ scan API, hoãn log MMIO (treo QEMU)

- Giữ `DEVICE_NET/BLOCK` + `scan_devices` + `net_mac` (API sạch).
- TẮT gọi scan trong `init_block` (đọc MMIO sau init treo cả khi không gắn
  net); QEMU revert blk-only; gate xanh lại 3/3×3.
- Bài học: probe net TRƯỚC init blk hoặc dùng monitor `info qdm`.
- Xem `docs/agent2-report-40.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `55c171597dc3781b348309ecc0c411d652917e9d793dcac9f00d5e60ac1c5500`.

## 9.20 — Vòng 41: xác minh treo MMIO (không đổi code)

- Gắn net: hiện `VIRTIO NET` một lần rồi treo sau `VIRTIO DEV`; revert.
- Probe trước init không log do serial test khác đường đọc; cần monitor.
- Giữ nguyên code, gate xanh (làm mới status repeat 3).
- Xem `docs/agent2-report-41.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `f15f4482c7082bbd92c5e80f803c74094a01c8e13b59bae04ab1f03c085511e2`.

## 9.21 — Vòng 42: probe net trước init (pha 3 bước 2)

- Serial test đúng ở `$TMPDIR` (đọc nhầm `/tmp` là nguyên nhân vòng 39–41).
- RISC-V probe 3 slot: net device=1 ở 0x10007000; AArch64 quét 32 slot:
  net ở 0xa003c00, blk ở 0xa003e00. QEMU gắn virtio-net cả 2 arch.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-42.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `f015d4ba5b09004a2a58aed1322b4799164c268a64f47d862fe5289ecf427091`.

## 9.22 — Vòng 43: log MAC net cả 2 arch (pha 3 bước 3)

- Gọi `net_mac` trong probe trước init; cả hai log
  `VIRTIO NET mac=52:54:00:12:34:56` (QEMU mặc định).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-43.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `bd6265fb5fffc8db8b5d36f9f6db1de980c87115bbbcffcfdcb440c7a519bb63`.

## 9.23 — Vòng 44: đọc feature bits net (pha 3 bước 4)

- `net_features` đọc DeviceFeatures 0x10/0x14 read-only; cả hai
  `mac=52:54:00:12:34:56 feats=0x30bf8024` (MAC/STATUS/MTU/MQ/...).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-44.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `ebb4936c8aa9af5583942306c3d4bf18b98254a5283c52263ebaa06c44cef1de`.

## 9.24 — Vòng 45: NetQueue static riêng (pha 3 bước 5)

- Đo trước: Kernel 561.768/budget 716.800 nhưng khe hở AArch64 chỉ 8B →
  NetQueue ~4 KiB đặt ngoài Kernel (vùng .bss sau BKL).
- `net_queue_info` + log `VIRTIO NETQ addr/size` cả 2 arch; khe hở giữ 8B.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-45.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `9c8b26bbd682d5e90b46601bd95d475251111eacbd948445a3647519d613d0f4`.

## 9.25 — Vòng 46: init TX queue net (pha 3 bước 6)

- `net_init_tx`: reset/ACK/accept MAC+STATUS+VERSION_1, gắn queue TX(1) vào
  NET_QUEUE, DRIVER_OK; cả hai NET READY accepted=0x10020.
- Khe hở static giữ 8B; cổng QEMU 3/3 ×3, persist 7×3, harness 483, host
  203, 0 warning.
- Xem `docs/agent2-report-46.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `5604fb94955d8a09d317599bfbee1d0d6e7a9bac51272f684b998c2e64d3d3ce`.

## 9.26 — Vòng 47: gửi ARP đầu tiên (pha 3 bước 7)

- `net_send_arp`: Ethernet broadcast + ARP who-has 10.0.2.2 tell 10.0.2.15,
  2 descriptors, kick TX, poll used; cả hai `sent len=0` (QEMU consumed).
- Khe hở static giữ 8B; cổng QEMU 3/3 ×3, persist 7×3, harness 483, host
  203, 0 warning.
- Xem `docs/agent2-report-47.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `941152e9203a693e89ce3e508b2d34d68fa9b97fe2db413bb0ed3a038374e00a`.

## 9.27 — Vòng 48: thử RX, hoãn sau DRIVER_OK (pha 3 bước 8)

- Thêm `NET_RX_QUEUE` + `net_init_rx` + `net_recv_arp` (API giữ lại).
- Gắn RX sau DRIVER_OK của TX → QEMU kill rc=-9 timeout; revert TX-only.
- Bài học: gắn mọi queue trước DRIVER_OK (vòng sau `net_init_both`).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-48.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `f12d22651cf27963c7d5ff44a3e6133c3ef2f27589c166f994d5e6cb23621440`.

## 9.28 — Vòng 49: gắn cả 2 queue trước DRIVER_OK (pha 3 bước 9)

- `net_init_both`: gắn RX(0, post buffer) + TX(1) rồi mới DRIVER_OK;
  `net_init_tx` thành wrapper. Rút bound RX 20M→2M (20M trên QEMU >20s).
- Cả hai READY + ARP sent, NO REPLY trung thực; hạ tầng đúng.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-49.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `6fea44ff7fb81ff6f5cca79a0a3e54903c38922bdb434f3bd5976319b65af11a`.

## 9.29 — Vòng 50: ARP 60B + chẩn đoán RX (pha 3 bước 10)

- Padding ARP 60B, rút TX poll 2M; sửa comment parse (hdr/buf riêng);
  thêm `net_recv_peek` + log PEEK — xác minh không có gói nào đến.
- Vòng sau kiểm tra backend user-net ngoài guest (monitor/host ping).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-50.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `5510e63083e5c1327925228ca51dca7e742d62b07aaca2e096de195398b09a6d`.

## 9.30 — Vòng 51: xác minh backend net + chốt pha 3 MVP (không đổi code)

- Monitor `info network`: nic 52:54:00:12:34:56 + user 10.0.2.0 restrict=off;
  chuỗi probe→TX đã kiểm chứng; reply để vòng sau (dump/timing).
- Cổng giữ nguyên vòng 50 (không đổi code guest).
- Xem `docs/agent2-report-51.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `bd0bf7da8232f42c880e068dfbd2bca36560c706eea41579fc6c7488da2f0707`.

## 9.31 — Vòng 52: ramfb + fw_cfg sống (pha 4 bước 1)

- QEMU gắn `-device ramfb` cả 2 arch; probe fw_cfg read-only, cả hai
  `sig=0x554d4551` — kênh sống, sẵn đọc `etc/ramfb` vòng sau.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-52.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `0936b21d6997f828f2ee97e713ddf89ef49bca8a1c132d1e344261d49665521c`.

## 9.32 — Vòng 53: ghi selector fw_cfg treo, revert an toàn (pha 4 bước 2)

- Ghi selector 0x19 treo RISC-V rc=-9; fw_cfg MMIO không có signature cố
  định. Revert sig-only cả 2 arch, gate xanh lại.
- Vòng sau: fw_cfg DMA hoặc device tree framebuffer.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-53.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `e99f190acf754fd303deacd25842338b2c52c71224f68e6a438c84b5cbcb900f`.

## 9.33 — Vòng 54: DTB đọc được trên RISC-V (pha 4 bước 3)

- Probe DTB magic trong `init_platform` (runtime cấm unsafe).
- RISC-V `0xd00dfeed` đúng; ARM `0x0` (chưa map, cần map vòng sau).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-54.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `761ffef48af497fffde29d1348f4d31084ef3e989d800bf59f727ccb8e51e063`.

## 9.34 — Vòng 55: quét FDT treo, giữ header-only (pha 4 bước 4)

- Header FDT OK (size=10319); quét struct tìm framebuffer treo rc=-9 cả bản
  bounded → revert header-only, gate xanh.
- Bài học: parse FDT sau MMU ổn định hoặc copy DTB vào vùng đã map.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-55.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `7d555265f89b421cbe6ca113d5e1184193fa7fddd9dca6548a8472cbb2f7d8ed`.

## 9.35 — Vòng 56: header FDT đầy đủ (pha 4 bước 5)

- Log thêm strings_off=5716 + version=17 (FDT v17 chuẩn) trong vùng an toàn.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-56.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `cc76cc1be70ebe452d425c5e1ddeef7ea4f23eff10f4b56bc55ecc5d81c608d0`.

## 9.36 — Vòng 57: tổng kết pha 4 display (không đổi code)

- Chốt hạ tầng probe: ramfb + fw_cfg + DTB header RISC-V v17.
- Parse sâu để vòng sau (user-space hoặc sau MMU ổn định).
- Cổng giữ nguyên vòng 56.
- Xem `docs/agent2-report-57.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `40a4ddeba0373b0c9e86f4977b5ded7d04e1a18655e45328a3720518947a3633`.

## 9.37 — Vòng 58: xác minh TX bằng packet dump

- `VIFAST_NET_DUMP=1` opt-in filter-dump; pcap chứng minh ARP who-has
  10.0.2.2 đúng chuẩn 60B đã lên dây. Gate mặc định không đổi.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-58.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `946ffb025a4610f2bc9e658d9a0964fcb758b421388803c118968a4512cd7211`.

## 9.38 — Vòng 59: khóa net vào harness gate

- `VIRTIO NET READY` exact-one riscv64+aarch64 (x86 loại trừ); fixture đúng
  base thật; harness âm giữ 483.
- Cổng: QEMU 3/3 ×3, persist 7×3, host 203, 0 warning (2 flaky đơn lẻ do tải
  máy, chạy lại đều 3/3).
- Xem `docs/agent2-report-59.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `d74aad27c89873aed58efefca23d83232ab733ae2155e08732bbb09333c901a5`.

## 9.39 — Vòng 60: fmt cleanup + đánh giá M1→M3

- `cargo fmt` 3 file (whitespace-only); `fmt --check` PASS lại.
- Đánh giá: M1 + MVP các pha đạt (bằng chứng đầy đủ); demand-load/RX-reply/
  vẽ-pixel mở có lý do; goal giữ active.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 483, host 203, 0 warning.
- Xem `docs/agent2-report-60.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `c33cb6900fe23e0ce53310772c21cbe247616dab45913c752cdd7784474c6e79`.

## 9.40 — Vòng 61: khóa input task vào harness (pha 4 input)

- Shell in `SHELL READY` khi vào vòng lặp; exact-one cả 3 arch.
- Harness âm 483→489. Serial chardev 2 chiều để vòng sau.
- Cổng: QEMU 3/3 ×3, persist 7×3, host 203, 0 warning.
- Xem `docs/agent2-report-61.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `15a5c6d54897f48c39b61b3d354868a7e3bed95df2632a5d3ff4ebd5a8ee4243`.

## 9.41 — Vòng 62: input end-to-end opt-in (pha 4 input)

- `--interactive`: chardev socket, gửi `help`+`\r`, yêu cầu output help —
  riscv64 4/4, aarch64 4/4. Sửa 3 lỗi: AF_UNIX 104B, `-serial` mồ côi,
  `\n` mất trên PL011.
- Cổng chuẩn không đổi: QEMU 3/3 ×3, persist 7×3, harness 489, host 203.
- Xem `docs/agent2-report-62.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `0dd7f62ae3f804db9e062a33d2d2ba070df6439d83570ec8538333367b61ce7d`.

## 9.42 — Vòng 63: net TX+RX end-to-end, ping gateway (ĐỘT PHÁ)

- Tìm 3 bug gốc: header 10B (không 12), QueueReady=1 (không index), RX kick
  + re-post. Giờ guest nhận `PING REPLY from 10.0.2.2` cả riscv64/aarch64.
- Harness khóa `VIRTIO NET PING REPLY`; pcap 6 gói khớp log.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 489, host 203, 0 warning.
- Xem `docs/agent2-report-63.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `e5c41d8646f5040c9533b32ec5cfaa9034f2681416d95051ea0a4a0f92ab68c5`.

## 9.43 — Vòng 64: UDP + DNS hai chiều

- `net_send_udp`/`net_recv_udp` + DNS query example.com qua slirp; nhận
  `DNS REPLY len=69` cả riscv64/aarch64. Bug: thiếu re-post RX sau khi nhận.
- Harness khóa `VIRTIO NET DNS sent`; reply best-effort (không phụ thuộc DNS host).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 489, host 203, 0 warning.
- Xem `docs/agent2-report-64.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `184612c9cee45493f9f05021c29268b46e8a8e78e891447b4a8ec6714bea420a`.

## 9.44 — Vòng 65: fw_cfg âm có căn cứ + test checksum

- Thử DMA / selector u16 / selector byte → đều treo; selector đúng +0x04
  (vòng 53 ghi nhầm +0x08 = DMA addr). Kết luận: vùng fw_cfg chỉ đọc trong
  cấu hình này → display phải đi virtio-gpu.
- 3 unit test checksum RFC 1071, thêm `cargo test -p vifast-virtio` vào gate;
  host test 204→207.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 489, 0 warning.
- Xem `docs/agent2-report-65.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `503a84d2afcb72bb1bb9d99868b8e140fe89247a02e62589d9656f4f3a949523`.

## 9.45 — Vòng 66: M4 display MVP (virtio-gpu + pixel thật)

- Driver virtio-gpu tối thiểu; gradient 160x120; screendump xác nhận pixel
  đúng cả riscv64/aarch64. Framebuffer 320x240 vượt headroom nên giảm còn
  160x120.
- Harness khóa `VIRTIO GPU READY` + `VIRTIO GPU DRAW OK`; `--gpu-check` opt-in.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 489, host 207, 0 warning.
- Xem `docs/agent2-report-66.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `5089e083d5553519e3ec51cfc6d1ed9fc935b70ef4edb44275a33b7b122a26a6`.

## 9.46 — Vòng 67: M2 demand-load

- Trang PAGER4 chỉ được cấp + nạp nội dung lúc fault; `DEMAND LOAD OK` trên
  riscv64/aarch64. Sửa 3 lỗi: cap thiếu WRITE, x86 hết Untyped (dùng cfg),
  harness fault-count quá chặt (nới thành kiểm tra hiện diện).
- Thêm `VIFAST_KEEP_SERIAL=1` để điều tra flake.
- Cổng: QEMU 3/3 ×3 (2 lần), persist 7×3, harness 494, host 207, 0 warning.
- Xem `docs/agent2-report-67.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `2f7437bb0b34f002729d44810026e5452dff8ccab9592f0c5bf4ab375ae8da1a`.

## 9.47 — Vòng 68: DNS A record (resolve tên miền thật)

- `parse_dns_a` + `net_recv_dns_a`; guest nhận `DNS A 104.20.23.154` (riscv)
  và `172.66.147.243` (aarch64) cho example.com.
- 2 unit test DNS; host 207→209. Marker best-effort (phụ thuộc DNS host).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 494, 0 warning.
- Xem `docs/agent2-report-68.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `e05c22484dec8e3be673d5112a5b80f5867e3bc024685ea027a783fbd7ada488`.

## 9.48 — Vòng 69: demand-load cả 3 arch + soak runner

- Server tự retype Frame từ Untyped riêng lúc fault (không cần root cấp) →
  x86_64 có `DEMAND LOAD OK`; harness bỏ loại trừ x86 (494→495).
- Thêm `scripts/soak.py` (chạy lặp, lưu artifact lần hỏng).
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 495, 0 warning.
- Xem `docs/agent2-report-69.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `9099e13df51d79247d1659524c8a62b3fe6756933403a87bd919d23a31f361f8`.

## 9.49 — Vòng 70: virtio-input (bàn phím)

- Driver virtio-input device 18; phím vào RxQueue UART từ timer IRQ nên shell
  cũ dùng được. `--input-check` PASS 2/2 cả riscv64/aarch64.
- Harness khóa `VIRTIO INPUT READY` (494→498). Tăng bound TX/GPU + đảo thứ tự
  GPU trước net để hết flake.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 498, 0 warning.
- Xem `docs/agent2-report-70.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `b5409d77ec304c05c25ca42045e733494179d80b99c437e2385bd38ca35ee299`.

## 9.50 — Vòng 71: demand-load nạp từ VFS

- Pager server có IPC buffer + cap VFS; nạp `/page.bin` (4 KiB) theo khối
  128 B lúc fault, kiểm tra tail → `DEMAND VFSLOAD OK` (riscv64/aarch64).
- x86 giữ ảnh tĩnh; sửa panic do static 4 KiB làm server ELF phình.
- Harness 498→501. Cổng: QEMU 3/3 ×3, persist 7×3, 0 warning.
- Xem `docs/agent2-report-71.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `183d806fd4de471fb371e3979a0a8f59debb0a0b8d3ad64c33e84f7fcc504080`.

## 9.51 — Vòng 72: thử x86 VFS demand-load (kết quả âm)

- Thử expose boot Untyped (slot 14) để cấp frame IPC buffer cho pager trên
  x86_64: retype frame được nhưng `map_with_tables` cần thêm PageTable từ
  data/meta Untyped (đã cạn) → panic OutOfMemory. Đã revert, giữ VFS
  demand-load cho riscv64/aarch64; x86 giữ ảnh tĩnh.
- Gate xanh trở lại: QEMU 3/3.

## 9.52 — Vòng 73: boot splash "ViFastOs"

- Font 5x7 + `gpu_draw_text`; hộp tối + chữ trắng trên nền gradient.
  `--gpu-check` verify pixel chữ (255,255,255) và hộp (16,16,40) — PASS cả
  riscv64/aarch64. Ảnh: `docs/assets/vifast-display-160x120.png`.
- Cổng: QEMU 3/3 ×3, persist 7×3, harness 501, 0 warning.
- Xem `docs/agent2-report-73.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `06ca36cdf758509427a32a3599b1316c76f1f396119d4b18edf6339549de30b6`.

## 9.53 — Vòng 74: script gate tổng hợp

- `scripts/gate.sh` chạy build + QEMU 3/3×3 + persist + harness + fmt + docs,
  in PASS/FAIL từng bước và trả mã lỗi nếu có bước hỏng; `--quick` bỏ QEMU/persist.
- Đã chạy full: all steps passed.

## 9.54 — Vòng 75: virtio-rng (entropy)

- Driver entropy device 4; guest đọc được byte ngẫu nhiên thật (hai lần đọc
  khác nhau) — `VIRTIO RNG OK` trên riscv64/aarch64. Harness 501→504.
- Cổng qua `scripts/gate.sh`: all steps passed.
- Xem `docs/agent2-report-75.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `ad5ef32545916eadc267d15813883a80afdca4f33654eda85f4ed148d67af1ca`.

## 9.55 — Vòng 76: ổn định GPU aarch64 + doc VirtIO

- Soak 15 vòng: aarch64 chỉ 7/15 do `VIRTIO GPU INIT FAIL`. Sửa: `gpu_init`
  retry 4 lần bound ngắn. Đo lại 10/10 và soak sạch 8/8 cả 3 arch.
- Thêm `docs/11-hien-tai/13-virtio.md` (driver, thứ tự init, lỗi đã gặp, marker).
- Xem `docs/agent2-report-76.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `03ec0f0518cb1087a2eeb718ea15645e20ae878480fc9ee20e18b960a3e87df8`.

## 9.56 — Vòng 77: TCP echo server

- `net_send_tcp`/`net_recv_tcp` + echo server cổng 8080; `--tcp-check` qua
  hostfwd echo `ping-from-host` đúng trên riscv64/aarch64.
- Harness khóa `VIRTIO TCP LISTEN OK` (504). Giới hạn: 1 kết nối, không
  retransmit, chưa thường trú.
- Xem `docs/agent2-report-77.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `5baa7ace06f15a199134934b5e99957cc6c297453e8ead2d9cec18ba5aff1603`.

## 9.57 — Vòng 78: rò rỉ RX + TCP tin cậy

- Bug thật: `RX_SEEN = used.idx` làm mất buffer RX chưa đăng lại → aarch64
  LISTEN mà không ACCEPT. Sửa tiến từng gói + `net_rx_refill(8)`.
- TCP: ISN từ rng, xử lý RST, cửa sổ 400×25k thoát sớm sau FIN; `--tcp-check`
  dùng chardev socket (không buffer) → aarch64 5/5, riscv 3/3.
- x86 chốt chặn: huge page 2 MiB cần tách PMD; data/meta/boot Untyped đều
  không đủ. `gate.sh --with-tcp`: all steps passed.
- Xem `docs/agent2-report-78.md`.

## 9.58 — Vòng 79: input check và TCP early-out

- TCP demo bỏ qua hẳn khi không có net device (`net_ready()`), trước đó nó
  vẫn chờ đủ cửa sổ và làm `--input-check` hỏng (guest thoát trước khi phím
  tới). Gõ "help" qua sendkey không khả thi (guest sống ~2s, QEMU auto-repeat)
  nên test quay lại chuẩn một phím.
- Kết quả: riscv 2/2, aarch64 3/3 cho `--input-check`; `gate.sh --with-input`
  PASS; soak 10 vòng 100% cả 3 arch.
- Thêm cờ `--with-input` cho `gate.sh`; cập nhật đầu tài liệu bàn giao.
- SHA-256 `ViFastOs-architecture-summary.txt`: `b45f0f08c91643957d68749634d8bf32a84dd8371cb5cdd6993ebec5c210e525`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `17b418dc0bb683496485cbe1d230412b3d4add07f67c49ad4300ed0319cde999`.

## 9.59 — Vòng 80: audit P0 + TCP thường trú

- Audit subagent (DeepSeek V4.1 Flash) tìm P0-1 `FrameWrite` thiếu validate,
  P0-2 underflow TCP RX, P1-2 RX descriptor treo trùng → đã sửa, có test host.
- TCP chuyển sang `tcp_pump` trong timer IRQ + `service_window()` 2.5s trước
  khi thoát; phát hiện aarch64 chưa từng gọi pump. TCP 3/3 cả 2 arch.
- `gate.sh` 3m22s xanh; `--with-tcp --with-input` xanh.
- Xem `docs/agent2-report-80.md` và `docs/audits/2026-11-07-round79.md`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `b3bc500917d1471dbfa10af5ab6186eca35ddfd27da369d86ed3157ea0e60760`.

## 9.60 — Vòng 81: siết guard static AArch64

- `KERNEL_STATIC_BUDGET` từ 716_800 (lỏng 155 KB) về 566_272 = Kernel hiện tại
  + 4.5 KB, để runtime chặn sớm thay vì để build/linker chạm giới hạn thật.
- `build.sh` in `gap=` giữa `KERNEL_STORAGE end` và `BKL` và cảnh báo khi dưới
  4 KiB (hiện còn 8 byte).
- QEMU 3/3, harness 504 case xanh.
- SHA-256 `ViFastOs-architecture-summary.txt`: `c222865bef13adbbe6f65a74b9d6d8881a0ee34af0925963662fd628a4f56ea7`.

## 9.61 — Vòng 82: P1-1 và P1-4

- Retype thất bại giờ trả byte về Untyped (trước đó đốt 4 KiB mỗi lần hỏng);
  test host mới, kernel 135 test.
- `check-docs` so hash nguồn (`source_sha256`) thay mtime: rebuild không đổi
  mã vẫn PASS, sửa mã thì báo "kernel sources changed".
- Gate đầy đủ xanh.
- SHA-256 `ViFastOs-architecture-summary.txt`: `e6bfcce0d8a91e868e203083e0b3b463613a219416386495f07c2c2bfcfc9abe`.

## 9.65 — Vòng 86: P0 notification + cld + gate

- Fix P0: waiter notification thứ hai không còn ghi đè (thêm `waiter2`,
  `signal` chỉ đánh thức waiter đang block, `detach_tcb` gỡ cả hai) + 2 test.
- Thêm `cld` ở 6 entry x86; gate chạy kernel test tường minh, đếm test, và
  `--quick` báo "QEMU and persistence were SKIPPED".
- Test mới xác nhận destroy server đánh thức client BlockedReply (audit P1-3
  là false positive).
- SHA-256 `ViFastOs-architecture-summary.txt`: `c54166250b05ebb2ec2f9263124a0f9a04ca6778aec5dd83a8f1d1dea6b4ef12`.

## 9.62 — Vòng 83: P0 filesystem từ audit #2

- Dir entry length và dir size trên đĩa không còn làm panic VFS server; bitmap
  kiểm biên; 2 test ảnh hỏng (FS 24 test).
- RX init đặt `RX_OUTSTANDING=1`; TCP kẹp `ip_total` theo used-ring len;
  gỡ `service_window` sai (IRQ mask, không được gọi).
- Client kẹp readdir; fingerprint docs phủ thêm script test/harness.
- Gate `--with-tcp --with-input` xanh.
- SHA-256 `ViFastOs-architecture-summary.txt`: `0110dd90b36117a83fe66ae7051e39c7d4eee5426c8594ebccceb2deba699bb6`.

## 9.63 — Vòng 84: P2 (dead code, assert biên dịch, slot)

- Xoá `net_recv_udp`; `Kernel` vượt ngân sách giờ fail lúc biên dịch.
- 53 chỗ slot không hợp lệ trả `InvalidArg` thay vì rơi về slot 0; test mới.
- Kernel 136 test; QEMU: riscv64 3/3, aarch64 3/3, x86_64 3/3 sau khi sửa
  điều kiện ghi status chuẩn (chỉ ghi khi chạy đủ ba arch).
- SHA-256 `ViFastOs-architecture-summary.txt`: `b9d474384dc9e1e2304063ce40cb95bdd22d4ff507315292ad787cea3c2c52e8`.

## 9.64 — Vòng 85: quét PCI trên x86_64

- `pci_scan()` qua 0xCF8/0xCFC: `PCI SCAN OK devices=4 first=8086:29c0
  class=0600` (host bridge Q35) — marker bắt buộc của x86_64, harness 507 case.
- Đây là nền tảng cho virtio-pci (x86 hiện chưa có virtio).
- Gate đầy đủ xanh.
- SHA-256 `ViFastOs-architecture-summary.txt`: `e6bfcce0d8a91e868e203083e0b3b463613a219416386495f07c2c2bfcfc9abe`.
- SHA-256 `ViFastOs-architecture-summary.txt`: `fb66a3619780ee3dc67f23242d0a799c3b116449e6c4cf6135a5aa2c19ff2d85`.

## 9.66 — Vòng 87: đóng rò rỉ FP/SIMD

- aarch64: CPACR FPEN=0b01 (EL0 trap); x86_64: tắt OSFXSR/OSXMMEXCPT cho BSP
  và AP. Kiểm bằng llvm-objdump: mọi ELF user (và kernel x86) đều 0 lệnh
  FP/SIMD, nên chặn an toàn; lưu trữ per-task sẽ vượt ngân sách static.
- `soak.py` coi `(0,0)` là hỏng thay vì PASS.
- SHA-256 `ViFastOs-architecture-summary.txt`: `d5bfd9b420576780c615275aaddf0d963ad35de241328f27c5b678daf87f33ef`.

## 9.67 — Vòng 88: P0 notification (hàng đợi waiter)

- Audit #4 chỉ ra P0 trong chính bản vá vòng 86: `wait_as` đặt state trước khi
  kiểm slot, `WouldBlock` để task kẹt Blocked vĩnh viễn.
- Gốc rễ các lần sửa hỏng boot: root park 4 worker trên cùng notification
  (`vifast-root/src/main.rs:420,455,469,511`) — mọi thiết kế "WouldBlock" hay
  "2 slot" đều đổi timing SMP. Nay dùng hàng đợi 6 slot, `signal` đánh thức
  tất cả; 140 test kernel.
- FS `read_dir_entry` giữ index khi gặp entry hỏng (25 test); x86 có IDT[0xFF]
  cho vector spurious; `Platform::send_ipi` gửi thật.
- SHA-256 `ViFastOs-architecture-summary.txt`: `2252cbd595d3bb479a43b5d4df8411841c78043bb3f99cfeab3e588a8c1c8243`.

## 9.68 — Vòng 89: P1 phần cứng (PIC EOI, SGI 2, idle race)

- x86 `ack_timer` chỉ EOI PIC trên hart 0; aarch64 bật SGI 2 (panic halt không
  còn là dead code); `ap_idle_loop` kiểm lại queue sau khi set cờ idle.
- `enqueue` bỏ rơi TCB: xét lại — mỗi hàng chứa MAX_TCBS=18 = tổng TCB nên
  không thể xảy ra; ghi chú tại chỗ.
- SHA-256 `ViFastOs-architecture-summary.txt`: `f141eb6c8e69737481ba364ed168d9349b6ee028c37679ed76fcfabf06acd61a`.

## 9.69 — Vòng 90: fault resume lưu frame

- `fault::suspend` lưu frame vào slot của TCB fault; thêm cổng
  `Platform::fault_frame_complete()` (x86 = false vì trap.S đẩy vector/error
  trước SAVE_REGS — lưu frame rỗng gây GP).
- QEMU 3 arch 2/2; gate đầy đủ xanh.
- Audit #5 lỗi hai lần phía subagent; sẽ chạy lại vòng sau.
- SHA-256 `ViFastOs-architecture-summary.txt`: `2bff5071ef04b103702cb65cb86e7263d2ca31f89b6ed8cdb2ea24536f800e39`.

## 9.70 — Vòng 91-92: fault frame x86 + marker FP trong harness

- x86: `TrapFrame::from_raw` dựng frame đầy đủ từ stack thô (exc: vector+error
  trước rip; pf: chỉ error) → `fault_frame_complete() = true`, pager cứu được
  task; const assert `size_of::<TrapFrame>() == 160`, `rip@120`, `cs@128`.
- Harness thêm marker bắt buộc `EL0 FP TRAP` (aarch64) và `SSE USER OFF` (x86)
  → 508 case; `AP_HART` + assert cho AP.
- Soak 12 vòng: 12/12 cả ba kiến trúc.
- SHA-256 `ViFastOs-architecture-summary.txt`: `05b0f380b725f1ec1031924ea335c30ef399b54098bf61b543512f0fadbc8fd4`.

## 9.71 — Vòng 93: NX static + input end-to-end

- `kernel_data_ranges()` thêm `AP_GDT`, `GDT_PTR`, `IDT_PTR`, `AP_GDT_PTR`,
  `CPU_STATES` → 19 vùng, hết executable dưới NX.
- `--input-check` yêu cầu cả `VIRTIO KEY 18` và echo `shell> e`: phím phải tới
  user shell, không chỉ driver. riscv 1/1, aarch64 2/2.
- Display tương tác hoãn: gọi `gpu_command` trong IRQ giữ BKL có nguy cơ deadlock,
  cần display service/deferred work.
- SHA-256 `ViFastOs-architecture-summary.txt`: `2f9809319ed47728e46ac5d530321426ef7319ae662d8868979ae7a1146e6ea5`.

## 9.72 — Vòng 94: NX trung thực + input không false-green

- `PD_HIGH` vào danh sách NX (20 vùng); hết slot split ⇒ exit(1) thay vì tắt NX
  âm thầm (trước đây self-check vẫn xanh khi toàn bộ data executable).
- `--input-check` x86 in SKIP thay vì trả True; lọc auto-repeat `value==2`;
  client ack IRQ ngay sau khi đọc byte (tránh coalesce).
- QEMU 3 arch 2/2; input check aarch64 2/2, riscv 1/1.
- SHA-256 `ViFastOs-architecture-summary.txt`: `3c70ac64c9c2b1f731980beca3c8f86be0c7aba8def3612c4d40e42ed5e5f475`.

## 9.73 — Vòng 95: màn hình tương tác

- `gpu_echo_key(code)` + font chữ số + flush vùng nhỏ; hook IRQ virtio-input
  (aarch64/riscv) log `INPUT ECHO <code>`.
- `--input-check` đòi `VIRTIO KEY 18` + `shell> e` + `INPUT ECHO 18`, và thêm
  `-device virtio-gpu-device` (trước đây không có GPU nên không thể vẽ).
- aarch64 2/2, riscv64 2/2; gate đầy đủ xanh.
- SHA-256 `ViFastOs-architecture-summary.txt`: `87b8c1137c31e3abe327473d97f906d8fbda30b3b720e7d657c9344122ddd72e`.

## 9.74 — Vòng 96: image range x86 + hết flake DNS

- UEFI ghi `(base, SizeOfImage)` vào platform → `kernel_image_end()` thật;
  `check_loaded_image` siết 4 GiB → 1 GiB.
- DNS demo thử lại 8 lần khi TX ring drain; soak 12/12 cả ba kiến trúc (trước
  đó 19/20 vì `VIRTIO NET DNS sent` mất).
- Audit #8: kernel x86 dùng lld-link/COFF, không thể dùng linker script;
  hướng đúng là đọc PE section table + NX toàn ảnh trừ text.
- SHA-256 `ViFastOs-architecture-summary.txt`: `92ae61a482b93504fdf7e37e1c1b739dc30e9f5dc2dd5eb97858caefbf49acb3`.

## 9.75 — Vòng 97: NX toàn ảnh + K1/K2/K3/N1

- NX mọi trang ảnh trừ 3 dải text PE (`IMAGE TEXT NX OK ranges=3`), có
  `reload_cr3()` sau khi sửa PTE; `kernel_wx_ok()` kiểm hai chiều.
- K1: `wake_error` không enqueue task đang Running. K3: enqueue chọn hart ít
  việc nhất. N1: DNS/ICMP kẹp theo frame thật. K2: test chuỗi A→B→C.
- Kernel 145 test; QEMU 3/3 ×3 arch; gate đầy đủ xanh.
- SHA-256 `ViFastOs-architecture-summary.txt`: `99d677e205d00b9b9f4a6b86889c97f2040e66b5006f9f9a114fbd86e328e803`.

## 9.76 — Vòng 98: x86 có bàn phím PS/2

- Thêm `irq1_entry`, IDT 0x21, PIC IMR 0xEC, i8042 theo thứ tự an toàn
  (AD→flush→CCB→AE→F4+ACK→unmask), chặn auto-repeat, bỏ 0xE0, đọc hết byte.
- `--input-check` x86 chạy thật: `PS2 KEY 18` + `shell> e` (2/2 PASS).
- NX: `image_pages_nx()` sau IDT, miễn trampoline AP; `make_ready` chỉ đánh thức
  từ Inactive/Ready/Blocked* (K4, +2 test).
- SHA-256 `ViFastOs-architecture-summary.txt`: `eb1e085e210400cee6fb8edce817947b7811c198894754b3835b47144d6ea23c`.

## 9.77 — Vòng 99–100: K32–K75, console lock, soak-verify 20/20

- Vòng 101 (commit 7fe404c): vá 37/44 lỗi K32–K75 + 12 test hồi quy.
  Bài học resume-BlockedFault (pager flow gãy khi siết bừa, đã sửa).
- Vòng 105 (commit 7a45f30): spinlock console Tx chống dính ký tự SMP
  (soak-20 x86 fail 2/20 harness oan) — soak-verify lại 20/20 cả 3 arch.
- Kernel 156 test; QEMU 3/3×3; persist 7×3; harness 508; gate full
  --with-tcp --with-input xanh.
- SHA-256 `ViFastOs-architecture-summary.txt`: `8eb5a24612eb494f763a8087a8afef3ef7e4ef26d7f6fddbd75de8780702ca30`.

## 9.78 — Vòng 1–23 (mốc cuối tuần): Exit/Wait/Reply-cap + soak 10/10

- Mốc 1: Exit theo task (`Exited`, `exit_current_on`), `TcbWait` (syscall 51),
  chặn destroy khi có waiter, exit-demo QEMU worker F (`WAIT REAP OK`×3 arch),
  6 test hồi quy (exit-wake, wait, sleep-exit, destroy-sạch, restart-tái-sinh,
  syscall-lỗi-value-0, validate-range, group-revoke).
- Mốc 5: Reply mang cap GRANT + 4 fix park/runq họ K32/K35.
- Mốc 3/8/9/11: journal txn limit, shell 256B+lịch sử+Ctrl, FS→ELF→parse,
  runbook 6 sự cố.
- Soak 10/10×3 arch + harness mỗi vòng (514 cases); kernel 166, FS 27.
- 2 bài học hạ tầng: x86 cạn data pages cuối boot (spawn tối giản);
  `TcbWait` race Ok/ObjectDestroyed đều đúng.

## 9.79 — Vòng 84: parse ELF metadata + exec-from-FS groundwork + gate 2 máy

- `vifast-elf`: tách `parse_impl` và thêm `parse_metadata` (chỉ cần header +
  program header table, không cần thân segment) + test
  `metadata_parse_accepts_header_plus_phdrs_only`; `parse` giữ nguyên hành vi
  (vẫn `FileRange` khi thiếu bytes).
- `vifast-root`: `spawn_elf_from_fs` nạp từng page thẳng từ file vào frame
  đích (`root_vfs_copy_to_frame`, READ 128B → `frame_write`) — KHÔNG staging,
  không tốn frame ngoài image; wired vào `run <path>` với fallback demo.
- Bài học bộ nhớ: DATA untyped gần cạn (85/86 page, 82/86 object cuối boot).
  Staging 8 frame làm spawn fail `map_frame_fallible`; FS-spawn gặp
  `OutOfMemory` (e=5) vì free pool ≈ 0. Không thể tăng pool: headroom riscv
  chỉ 2 416 B (cần ≥192 KiB) và gap aarch64 = 8 B. Hướng đúng: thu hồi demo
  TCB/VSpace sau khi hết demo.
- Bài học boot: gọi VFS blocking ở cuối boot (tạo `/prog.elf`) làm mất toàn
  bộ task (`USER EXIT` sau TCB REAP OK); tạo file phải on-demand trong đường
  `run`.
- Hạ tầng: `ps` (Linux x86_64, Tailscale) cài rustup + 4 target bare-metal,
  qemu-system-{riscv64,aarch64,x86_64}, OVMF symlink; `scripts/par-gate.sh`
  chia việc local (riscv64+aarch64) / remote (x86_64), rsync trừ `target/`.
  Gate 3/3×3 hai máy song song xanh.

## 9.80 — Vòng 89: EXEC-THẬT TỪ FILESYSTEM chạy được (`RUN FS OK`)

- `runfs [path]` (shell) → root tạo `/prog.elf` on-demand rồi `spawn_elf_from_fs`
  nạp **từng page** từ file vào frame đích, map quyền theo segment, tạo TCB và
  resume: riscv64 + aarch64 in `ROOT PROG OK` rồi `RUN FS OK`.
- Hai bug thật đã tìm ra (đều là bug hợp đồng ABI, không phải flake):
  1. Copy path từ IPC buffer **sau** khi các call VFS khác đã ghi đè buffer →
     server nhận `ELF…` thay vì đường dẫn → OPEN trả EINVAL(8). Sửa: copy path
     NGAY đầu dispatch, trước mọi call VFS.
  2. `frame_write` từ chính IPC buffer bị `validate_user_range` từ chối
     (InvalidArg 6) dù đọc/ghi buffer ở userspace bình thường; nguồn tĩnh hợp lệ.
     Sửa: READ → buffer cục bộ 256B → `frame_write` từ buffer đó.
- Giới hạn bộ nhớ xác nhận lại: DATA untyped còn ~6 page cuối boot; exec-thật
  ELF demo (1 page code + stack) vừa đủ. Vòng sau: thu hồi demo TCB/VSpace để
  nâng trần trước khi exec client ELF đầy đủ (5 page).
- `--interactive` gửi thêm `runfs` (best-effort vì riscv64 có flake nhận byte
  UART ở chế độ socket, tái hiện cả trên HEAD).

## 9.81 — Vòng 90: điều tra thu hồi bộ nhớ pager + par-gate --full

- Thử thu hồi 3 pager client (6 frame + 3 TCB) sau `DEMAND LOAD OK` để nâng
  trần exec-thật. Kết quả điều tra (giữ lại làm bài học, code đã revert):
  1. `object_destroy` từ **pager server** trả `HasDependents` (0/6 destroy được)
     vì root còn cap gốc; phải `cap_revoke` ở chính root rồi mới destroy.
  2. Copy cap stack frame vào pager server phải tránh slot 11 — đó là
     `PAGER_SERVER_VFS_ENDPOINT`; đè lên làm boot in `PAGER VFS CAP BAD`.
  3. `send()` từ pager server tới endpoint của root trong nhánh
     `DEMAND LOAD OK` làm task pager dừng ngay sau khi in marker (không in
     được dòng kế tiếp) — cần điều tra thêm trước khi dùng làm tín hiệu
     thu hồi; chưa dùng.
- Chốt: chưa đổi hành vi hệ thống; giữ gate xanh ở `e9c1557`.
- Hạ tầng: `scripts/par-gate.sh --full` chạy song song local (riscv64+aarch64
  QEMU + persist + harness + gen-docs/check-docs + fmt) và remote x86_64
  (QEMU + persist qua SSH) — một lệnh, ~½ thời gian.

## 9.82 — Vòng 91: thu hồi demo pager (`PAGER RECLAIM OK`)

- Pager server báo root khi xong demo demand-load (cap tới fault endpoint của
  root ở slot 15, `PAGER_DONE_LABEL`); root `cap_revoke` → `frame_unmap` →
  `object_destroy`/`tcb_destroy` cho 3 pager client (3 TCB + 6 frame).
  `PAGER RECLAIM OK` trên riscv64 và aarch64; DATA untyped objects 77 → 71.
- Bài học ABI: `object_destroy` trả `HasDependents` nếu còn cap dẫn xuất —
  phải `cap_revoke` ở chủ sở hữu trước. `frame_unmap` trả `NotMapped` là hợp
  lệ (pager2 evict demo đã unmap trang của nó trước).
- Sửa lỗi shell `runfs <path>`: cắt từ index 6 (trước là 7, mất dấu `/`).
- Ghi chú hạ tầng: kernel nhúng `*.stripped` do `build.sh` sinh — build tay
  bằng `cargo build` không cập nhật ELF nhúng (dễ test nhầm bản cũ).
- Gate: QEMU 3/3×3, persist 7×3, harness 514, docs PASS, fmt sạch, 0 warning,
  headroom riscv giữ 199024 B.

## 9.83 — Vòng 92: exec-from-FS + thu hồi pager vào CỔNG CHUẨN (3 arch)

- Root tự chạy exec-thật từ FS ngay sau khi thu hồi pager (`PAGER_DONE`):
  ghi ELF demo thành `/prog.elf`, `spawn_elf_from_fs` nạp từng page từ file,
  in `RUN FS OK`. Không cần nhập bàn phím → chạy tự động trong cổng chuẩn.
- `PAGER RECLAIM OK` + `RUN FS OK` vào `harness_spec.COMMON`; fixture của
  `test-harness-spec.py` sinh từ chính COMMON nên chỉ tăng 514 → 520 ca.
- Sửa: cap tới fault endpoint của root cho pager server phải copy NGOÀI khối
  `enable_vfs` — trước đó x86_64 không có cap nên không thu hồi/không exec.
- Kết quả: QEMU 3/3×3 PASS với cả hai marker; persist 7×3; harness 520; docs
  PASS; fmt sạch; 0 warning; headroom riscv 199 024 B.

## 9.84 — Vòng 93: chương trình từ FS in ra console ("HI DSK")

- `EXEC_DEMO_ELF` giờ là chương trình thật: in chuỗi bằng `DebugWrite(32)`
  rồi `Exit(34)`, mã hoá theo từng arch (riscv64 `auipc/addi/ecall`, aarch64
  `adr/movz/svc`, x86_64 `lea/mov/int 0x80`), chuỗi ở offset 0x40 trong trang.
- Hai bản: `EXEC_DEMO_ELF` ("HI FS", dùng cho demo trong RAM) và
  `EXEC_DISK_ELF` ("HI DSK", ghi xuống FS cho exec-from-FS). Nhờ vậy chuỗi
  `HI DSK` trên serial là **bằng chứng chương trình vừa chạy được nạp từ đĩa**,
  phân biệt hẳn với bản RAM.
- riscv64: serial có `HI FS` → `RUN FS OK` → `HI DSK` (đã xác nhận).
- aarch64: `HI FS` (RAM) chạy đúng, nhưng bản trên đĩa chưa in — cần điều tra
  tiếp (nghi vấn nạp/thi hành trang code sau khi ghi file), chưa đưa vào
  COMMON để tránh làm đỏ gate.
- Gate giữ xanh: QEMU 2/2×3 PASS, harness 520.

## 9.85 — Vòng 94: `HI DSK` vào cổng chuẩn — exec từ đĩa in ra trên cả 3 arch

- Vá bản ghi xuống FS thay vì giữ ảnh ELF thứ hai (headroom riscv chỉ còn
  ~2.4 KiB, thêm 4 KiB là vượt ngưỡng 192 KiB): chỉ sửa 2 chỗ — lệnh nạp độ
  dài (riscv `addi a1`, aarch64 `movz x1`, x86 `mov esi`) và chuỗi ở 0x1040.
- Kết quả: riscv64, aarch64, x86_64 đều in `HI FS` (bản RAM) rồi `HI DSK`
  (bản nạp từ filesystem) — chứng minh chương trình thực thi được đọc lên
  từ đĩa theo từng trang.
- `HI DSK` vào `harness_spec.COMMON`: harness 520 → 523 ca; mọi lần chạy
  QEMU trên mọi arch giờ đều phải thấy marker này.
- Gate: QEMU 3/3×3 PASS, persist 7×3 PASS, harness 523, docs PASS, fmt sạch,
  0 warning, headroom riscv 199 024 B.

## 9.86 — Vòng 95: truyền tham số cho chương trình nạp từ FS (argv tối giản)

- Chương trình demo không còn chuỗi nhúng: nó đọc chuỗi tham số ở
  `entry + 0x200` (riscv `auipc+addi`, aarch64 `adr`, x86 `lea rip`) rồi
  `DebugWrite` 6 byte. Root ghi chuỗi vào **trang code của chương trình**
  trước khi map: bản RAM → `HI FS`, bản trên đĩa → `HI DK`.
- Cùng một binary trên đĩa in nội dung khác nhau tuỳ tham số root truyền —
  minh hoạ argv/exec thật, không cần sửa kernel.
- Bài học W^X: `frame_write` lên frame **đã map executable** bị kernel từ
  chối; phải ghi tham số TRƯỚC `frame_map` (đã chuyển vào vòng nạp segment).
- Bài học mã hoá lệnh: `adr` aarch64 là `0x10000000 | (immhi<<5) | rd`
  (imm=0x200 → `0x10001000`), tự suy ra sai một lần (`0x10010000`) nên
  aarch64 không in — đã sửa và xác nhận cả 3 arch.
- Marker gate đổi thành `HI DK` (6 byte, có newline nên không dính dòng sau).
- Gate: QEMU 3/3×3 PASS, persist 7×3 PASS, harness 523, docs PASS, fmt sạch,
  0 warning.

## 9.87 — Vòng 96: điều tra argv động + headroom (chưa đổi hành vi)

- Thiết kế argv: chương trình đọc cặp `(ptr, len)` ở `entry+0x80` rồi
  `DebugWrite(ptr,len)` → độ dài ĐỘNG; root ghi cặp + chuỗi vào trang code.
  Hai điểm phải nhớ: (1) phải ghi tham số **sau khi copy nội dung segment**
  (nếu ghi trước sẽ bị nội dung file ghi đè); (2) phải ghi **trước frame_map**
  vì kernel chặn `frame_write` lên frame đã map executable (W^X).
- Thử thu nhỏ ảnh ELF demo từ `0x1000+0x48` xuống `0x200` (code ở 0x80, vùng
  tham số 0x100/0x120): headroom riscv tăng 199 024 → **203 120 B**, nhưng
  chương trình không in — cần rà lại `p_offset/p_vaddr/e_entry` và vùng đè
  giữa segment và tham số trước khi dùng lại.
- Thử sửa `runfs <path>` (cắt từ index 6) hai lần: mỗi lần build xong thì
  riscv64 timeout 0/3 ở cổng (`shell>` rồi treo), revert lại thì 2/2–3/3 PASS.
  Chưa kết luận nhân quả — ghi nhận để điều tra bằng bisect sạch.
- Kết luận vòng: giữ nguyên hành vi hệ thống ở `9fd145a`; gate xanh
  (riscv 3/3, aarch64 3/3, x86 3/3 sau rerun), harness 523.

## 9.88 — Vòng 97: sửa `runfs <path>` (index 6) + xác nhận flake riscv

- Shell `runfs <path>`: cắt từ index 6 (trước là 7 nên mất dấu `/`). Kiểm
  chứng tay: `runfs /prog.elf` → root nhận đúng path, nạp lại từ FS và in
  `HI DK`.
- Flake riscv64: hai lần chạy cổng đầy đủ cho 0/3 dù serial đã tới
  `SHELL READY` (timeout 20 s), trong khi chạy riêng `--repeat 5` thì 5/5.
  Chạy lại cổng đầy đủ khi máy rảnh: **3/3** — kết luận đây là flake do tải
  máy (QEMU bị kill trước khi kịp thoát), không phải lỗi logic.
  Bài học: khi 0/3 hàng loạt, kiểm tra `ps qemu` + chạy lại trước khi nghi
  commit.
- Ảnh ELF 0x200 (tiết kiệm 4 KiB headroom) vẫn chưa chạy được: spawn OK
  (`EXEC OK`, `RUN FS OK`) nhưng chương trình không in, kể cả khi in chuỗi
  cố định — cần trace phía kernel (entry/segment/icache) trước khi dùng lại.
- Gate: QEMU 3/3×3, persist 7×3, harness 523, docs PASS, fmt sạch, 0 warning,
  headroom riscv 199 024 B.

## 9.89 — Vòng 98: chẩn đoán flake + retry có kiểm soát

- `test-qemu.py`: khi timeout, phân biệt hai trường hợp và in rõ:
  - guest **đã in đủ marker** → "nghi máy host quá tải; chạy lại";
  - thiếu marker → liệt kê 4 marker thiếu đầu tiên (chỉ đúng chỗ hỏng).
- Thêm `VIFAST_RETRY_TIMEOUT=N`: chạy lại tối đa N lần **chỉ khi timeout**
  (lỗi marker/exit code vẫn fail ngay). Mặc định 0 — cổng chuẩn không đổi.
  `scripts/par-gate.sh` bật mặc định 1 cho vòng phát triển.
- Lý do: các vòng trước gặp 0/3 riscv64 hàng loạt dù serial đã tới
  `SHELL READY`; chạy lại khi máy rảnh thì 3/3. Giờ log tự nói rõ là flake.
- Gate: QEMU 3/3×3, persist 7×3, harness 523, docs PASS, fmt sạch, 0 warning.

## 9.90 — Vòng 99: chẩn đoán timeout cho persist + bỏ thử nghiệm giữ slot exec

- `test-persist.py`: mọi assertion timeout giờ kèm `timeout_hint()` — nếu
  guest đã in đủ marker thì báo "nghi máy host quá tải", nếu thiếu thì liệt
  kê marker thiếu. Nhờ đó lần fail x86_64 "large boot" hiện rõ là **không có
  marker nào** (boot không chạy), khác hẳn lỗi logic.
- Thử giữ slot của exec demo lúc boot để `run` sau này restart TCB cũ thay vì
  cấp mới (chống rò rỉ ~4 frame mỗi lần `run`): QEMU 3/3×3 vẫn xanh nhưng
  **persist riscv64 "blank format" timeout** (tái hiện 2 lần), revert thì
  persist PASS 3/3 → đã bỏ thay đổi, ghi lại làm bài học (thay đổi nhỏ ở
  đường boot vẫn có thể phá persist dù QEMU xanh).
- Gate cuối vòng: QEMU 3/3×3, persist 3/3 (sau rerun), harness 523, docs PASS,
  fmt sạch, 0 warning.

## 9.91 — Vòng 100: kiểm chứng parse cho entry lệch trang + bỏ thử thu hồi server

- Thêm test `accepts_unaligned_entry_inside_segment` (vifast-elf): ảnh kiểu
  "thu gọn" (PT_LOAD offset 0x1000, filesz 0x100, e_entry = vaddr+0x80) được
  parse chấp nhận; loader map tại page_base và code rơi đúng offset 0x80.
  Kết luận: **parse không phải nút thắt** — nếu ảnh 0x200 không chạy thì lỗi
  nằm ở đường thực thi phía kernel/runtime, không phải kiểm tra ELF.
- Thử thu hồi pager server (TCB + stack + IPC frame) sau PAGER_DONE:
  `PAGER SRV 0/0/0` (destroy OK cả 3) nhưng DATA untyped chỉ giảm
  objects 76 → 74, pages giữ 80; đồng thời **headroom destroy-smp tụt
  199 024 → 194 928** (dưới ngưỡng 192 KiB) → build fail. Đã revert.
- Bài học lặp lại: mọi thêm mã vào root đều ăn vào headroom destroy-smp
  (chỉ còn ~2.4 KiB); muốn thêm tính năng phải giải phóng chỗ trước.
- Gate cuối vòng: QEMU 3/3×3 (x86 rerun), persist 3/3, harness 523 

## 9.92 — Vòng 100b: serialize persist trong par-gate

- `par-gate.sh --full` chạy persist x86_64 ở **cả hai máy cùng lúc** → boot
  UEFI timeout không in marker nào (tái hiện 2 lần, đúng như cảnh báo mới
  của `timeout_hint`: "thiếu marker: SELFTEST PASS, FS DISK OK, ...").
- Sửa: persist/harness/docs chạy **sau khi remote xong** (tuần tự), QEMU
  song song vẫn giữ. `par-gate --full --repeat 1` PASS cả hai máy.

## 9.93 — Vòng 101: ảnh ELF demo 0x180 (headroom +4 KiB) — tìm ra bug thứ tự ghi

- Ảnh demo thu từ `0x1000+0x48` xuống **0x180 byte**: segment đặt tại file
  offset `0x120` với `p_vaddr = EXEC_DEMO_CODE + 0x120` (congruent mod 0x1000),
  code ở 0x120, chuỗi tham số ở 0x140. Headroom riscv **199 024 → 203 120 B**
  (destroy-smp cũng 203 120, hết đỏ).
- Nguyên nhân các lần trước ảnh nhỏ "không chạy": **ghi tham số TRƯỚC copy
  segment**, nên nội dung file (toàn 0) ghi đè chuỗi → chương trình in 6 byte
  NUL (vô hình). Sửa: ghi tham số **sau copy, trước frame_map** (W^X).
  Sau khi sửa, cả 3 arch in `HI FS` (RAM) và `HI DK` (nạp từ FS).
- Bài học: với ảnh nhỏ, vùng tham số nằm TRONG segment — thứ tự copy/args là
  hợp đồng bắt buộc, không phải chi tiết phụ.
- Gate: QEMU 3/3×3, persist 3/3, harness 523, elf 19 test, docs PASS, fmt
  sạch, 0 warning, headroom 203 120 B.

## 9.94 — Vòng 102: argv động (`runfs <path> <args>` → chương trình in tham số)

- Chương trình demo đọc **cặp (ptr, len)** ở `entry+0x20` rồi `DebugWrite`,
  nên độ dài là động; root ghi cặp ở frame 0x140 và chuỗi ở 0x160.
- Shell: `runfs <path> <args>` gửi `path\0args\0`; root tách args sau NUL và
  truyền vào `spawn_elf_from_fs`. Nhánh restart cũng exec-thật từ FS khi path
  tồn tại (trước chỉ restart TCB cũ), kèm args.
- Kiểm chứng: `runfs /prog.elf hello` → serial in `RUN FS OK` rồi **`hello`**
  (cùng binary trên đĩa, nội dung do tham số quyết định).
- Hai bug tự gây trong vòng, đều do thứ tự ghi:
  1. `write_program_args` gọi TRƯỚC copy segment → bị nội dung file ghi đè;
  2. trong `spawn_elf_from_fs` có HAI lời gọi (một với `msg`, một với default)
     → bản default ghi đè sau. Sửa: một lời gọi duy nhất, sau copy, trước map.
- Gate: QEMU 3/3×3, persist 3/3, harness 523, docs PASS, fmt sạch,
  headroom riscv 203 120 B (destroy-smp 199 024 B).

## 9.95 — Vòng 103: đưa argv vào cổng chuẩn (marker `HIARG`)

- Selftest sau thu hồi pager gọi `spawn_elf_from_fs("/prog.elf", "HIARG\n")`:
  chuỗi này **không có trong ảnh trên đĩa**, chỉ đến từ tham số root truyền →
  `HIARG` xuất hiện trên serial là bằng chứng đồng thời của exec-từ-FS và argv.
- `harness_spec.COMMON`: thay marker `HI DK` bằng `HIARG` (vẫn 523 ca).
- Runbook thêm mục 3b mô tả `runfs <path> [args...]`, định dạng
  `path\0args\0` và cách root ghi cặp (ptr,len).
- Kết quả: riscv64/aarch64/x86_64 đều in `HI FS` (ảnh RAM) và `HIARG` (exec
  từ FS + argv).
- Gate: QEMU 3/3×3, persist 3/3, harness 523, docs PASS, fmt sạch, 0 warning.

## 9.96 — Vòng 103b: ghi nhận flake persist trong par-gate

- `par-gate --full` lần đầu báo local FAIL: persist riscv64 "blank boot"
  (thiếu `FS DISK OK`, `FS FORMAT OK`, `SHELL READY`) trong khi
  `SELFTEST PASS` đã có → boot dừng sớm, không phải lỗi marker mới.
- Chạy lại `test-persist.py` một mình: **3/3 PASS**. Kết luận: flake do tải
  máy khi QEMU local + persist remote chạy chồng; `timeout_hint()` đã chỉ
  đúng nhóm marker thiếu để phân biệt.

## 9.97 — Vòng 104: thu hồi pager server + `exec` là bí danh của `runfs`

- Thu hồi pager server sau demo demand-load: `PAGER SRV 0/0/0` trên cả 3
  arch (TCB + stack frame + IPC frame destroy OK), objects untyped 76 → 74.
  Headroom giữ 203 120 B (main) / 199 024 B (destroy-smp) — vẫn xanh.
- Shell: `exec <path> [args...]` dịch tiền tố thành `runfs ...` rồi xử lý
  lại → hai lệnh dùng chung một đường mã; `help` liệt kê cả hai.
- Nhánh RUN khi TCB đã tồn tại: đổi từ "restart TCB cũ" sang **destroy +
  nạp lại từ file kèm args**, fail-êm về demo copy (an toàn như trước).
- Phát hiện còn tồn: `runfs` lần 2 trong cùng phiên rơi về demo copy
  (`RUN OK`) — spawn từ FS lần hai không cấp được object. Đây là **rò rỉ/hụt
  bộ nhớ sau destroy_exec**, không phải lỗi tham số; ghi nhận cho vòng sau.
- Dọn 0 warning (bỏ `exec_tcb` không dùng, đổi binding `Some(_tcb)`).
- Gate: QEMU 3/3×3, persist 3/3, harness 523, kernel 183, docs PASS, fmt sạch.

## 9.98 — Vòng 105: chẩn đoán vì sao `runfs` lần 2 thất bại

- Dựng lại thí nghiệm với **kernel mới thật** (bài học: chuỗi
  `cargo build … >/dev/null && cp` đã nuốt lỗi build nên nhiều lần chạy
  kernel CŨ — mọi kết luận trước đó phải xác minh bằng `strings` trên binary).
- Kết quả có instrument: lần 1 `RUN FS OK` + in đúng tham số (`aa`); lần 2
  dừng ở **F6 = `map_frame_fallible` thất bại** → không đủ bộ nhớ Untyped để
  retype frame/page-table, và `destroy_exec` không trả trang về pool
  (`DATA UNTYPED pages` không giảm).
- Thử giữ vspace (chỉ dọn TCB + frame) để tái dùng page table: vẫn F6 vì
  frame cũ bị destroy khi đang mapped → map địa chỉ cũ thất bại.
- Kết luận hướng đúng (vòng sau): **chuyển vùng tham số sang trang stack
  (RW)** để `runfs` lặp chỉ cần ghi args + restart TCB, không cấp object;
  hoặc lưu frame/VAs và unmap trước khi destroy rồi tái dùng frame.
- Cây làm việc trả về HEAD `9806d5c`; gate xanh (QEMU 2/2×3, build 0 warning,
  headroom 203 120/199 024 B).

## 9.99 — Vòng 105b: thử đưa tham số sang trang stack (chưa xong)

- Ý tưởng: tham số nằm trong trang STACK (RW) thay vì trang code (W^X) → `runfs`
  lặp chỉ cần ghi lại args rồi `tcb_restart`, không cấp object mới.
- Đã dựng: `EXEC_ARGS_VA = EXEC_STACK_BASE + 0x100`, chương trình đọc cặp
  `(ptr,len)` bằng `auipc 0x22 ; addi -0x20` (riscv), `adr #0x21FE0`
  (aarch64), `lea rsi,[rip+0x21FD9]` (x86); root ghi cặp/chuỗi vào frame
  stack sau khi map.
- Kết quả: spawn OK (`EXEC OK`, `RUN FS OK`) nhưng chương trình **không in** và
  **không fault** tại vùng args ⇒ đọc được nhưng nội dung là 0 (hoặc len=0).
  Cần kiểm chứng `frame_write` vào frame đã map RW (nghi khác biệt so với ghi
  trước khi map) trước khi thử tiếp.
- Đã revert; cây về HEAD `dda76c4`, gate xanh (QEMU 2/2×3, persist 3/3,
  harness 523, kernel 183, docs PASS, fmt sạch, 0 warning).

## 9.100 — Vòng 106: kiểm chứng vùng args trong trang stack (âm tính)

- Chương trình chẩn đoán in **1 byte** tại `EXEC_ARGS_VA + 0x40` (byte đầu chuỗi
  "HI FS") để tách bạch "ghi args hỏng" với "đọc sai địa chỉ".
- Kết quả: `EXEC OK` + `RUN FS OK` nhưng **không có byte nào** trên serial,
  bất kể ghi args TRƯỚC hay SAU `frame_map` của trang stack. Vậy frame_write
  không phải nghi phạm chính; nhiều khả năng **địa chỉ chương trình tính ra
  không trùng `EXEC_ARGS_VA`** (kiểm tra lại `EXEC_DEMO_CODE`/`CODE_VA` thực
  tế của từng arch, đặc biệt giới hạn `auipc/addi` 20+12 bit).
- Đã revert toàn bộ thí nghiệm; cây về HEAD `52a5984`, gate xanh
  (QEMU 2/2×3, persist 3/3, harness 523, docs PASS, fmt sạch, 0 warning).

## 9.101 — Vòng 107: thu hẹp nghi phạm sang `map_frame_fallible`

- In `image.entry` thật từ root: `ENT 68719481120` = `0x10_0000_1120` — **đúng**
  như tính tay, nên giả thuyết "địa chỉ chương trình lệch" bị loại.
- Thử lại vùng args trong trang stack với chương trình đọc cặp `(ptr,len)`:
  `EXEC OK` (nghĩa là **cả hai `frame_write` đều trả `Ok`**) nhưng chương
  trình vẫn không in gì ⇒ nội dung ghi vào frame **không hiện ra ở VA đã map**.
- Nghi phạm mới, hẹp: `map_frame_fallible`/`map_with_tables` — nhiều khả năng
  khi phải tạo page table mới, đường map dùng frame khác (hoặc bỏ qua frame
  truyền vào). Cần đọc lại hàm này và kiểm tra bằng một test host nếu có thể.
- Gate xanh sau khi revert: QEMU 2/2×3, persist 3/3, harness 523, docs PASS,
  fmt sạch, 0 warning, headroom 203 120/199 024 B.

## 9.102 — Vòng 108: ghi giới hạn `runfs` lặp vào runbook (thay vì đoán tiếp)

- Chương trình sentinel `A … args … B` **không in cả `A`** trong bản thí nghiệm
  ⇒ khi vùng args chuyển sang trang stack thì bản thân chương trình không chạy
  nữa (khác hẳn bản đang xanh in `HI FS`), tức lỗi nằm ở **đường spawn**, không
  phải ở phép đọc.
- Để không mất thêm vòng đoán: ghi rõ giới hạn hiện tại vào runbook
  `docs/11-hien-tai/16-van-hanh.md` mục 3b — `runfs`/`exec` chạy **1 lần mỗi
  phiên**, lần 2 rơi về demo copy do cạn Untyped; kèm cách theo dõi
  (`DATA UNTYPED pages=…/86`) và hai hướng sửa (tái dùng frame+PT, hoặc trả
  trang khi destroy_exec).
- Gate: QEMU 2/2×3, persist 3/3, harness 523, docs PASS, fmt sạch, 0 warning,
  headroom 203 120/199 024 B.

## 9.103 — Vòng 108b: khoá hành vi thu hồi pager bằng marker cổng chuẩn

- Thêm `("PAGER SRV 0/0/0", …)` vào `harness_spec.COMMON`: cổng chuẩn giờ
  **bắt buộc** thấy dòng này trên cả 3 arch, tức việc thu hồi pager server
  (TCB + stack + IPC frame) không thể bị lặng lẽ bỏ qua về sau.
- Harness 523 → **526 ca** (fixture tự sinh từ COMMON), QEMU 3/3×3, persist
  3/3, docs PASS, fmt sạch, 0 warning.

## 9.104 — Vòng 109: kiểm tra toàn vẹn kích thước file exec trong cổng chuẩn

- Selftest sau thu hồi pager giờ **STAT `/prog.elf` và so kích thước** với
  ảnh ELF dựng sẵn (trước đây chỉ verify 64 byte đầu): in `PROG SIZE OK` hoặc
  `PROG SIZE BAD <size>`. Bắt được lỗi ghi thiếu/ghi đè âm thầm của đường VFS
  mà verify prefix không thấy.
- Thêm `("PROG SIZE OK", …)` vào `harness_spec.COMMON` → cổng chuẩn bắt buộc
  thấy trên cả 3 arch. Harness **526 → 529 ca**.
- Gate: QEMU 3/3×3, persist 3/3, harness 529, kernel 183, docs PASS, fmt sạch,
  0 warning, headroom 203 120/199 024 B.

## 9.105 — Vòng 110: kiểm tra nội dung CUỐI file exec (không chỉ kích thước)

- Thêm `root_vfs_verify_tail()`: STAT để lấy kích thước, rồi đọc và so khớp
  **32 byte cuối** của `/prog.elf` với ảnh ELF gốc — bắt được lỗi ghi hụt ở
  giữa/cuối file mà kiểm tra kích thước + prefix bỏ lọt.
- Selftest in `PROG TAIL OK` / `PROG TAIL BAD`; thêm marker vào
  `harness_spec.COMMON` → cổng chuẩn bắt buộc. Harness **529 → 532 ca**.
- Gate: QEMU 3/3×3, persist 3/3, harness 532, docs PASS, fmt sạch, 0 warning,
  headroom 203 120/199 024 B.

## 9.106 — Vòng 111: kiểm tra byte GIỮA file exec (phủ lỗi ghi lệch chunk)

- `root_vfs_verify_at(path, offset, expected)`: so khớp một đoạn bất kỳ ở
  offset tuyệt đối (STAT trước để chặn đọc ngoài file).
- Selftest kiểm tra thêm **16 byte quanh `len/2`** và in `PROG MID OK/BAD`.
  Giờ cổng phủ cả **đầu** (`PROG SIZE OK` + prefix verify cũ), **giữa**
  (`PROG MID OK`) và **cuối** (`PROG TAIL OK`) của ảnh ELF ghi xuống FS.
- Thêm marker vào `COMMON` → harness **532 → 535 ca**.
- Gate: QEMU 3/3×3, persist 3/3, harness 535, docs PASS, fmt sạch, 0 warning.

## 9.107 — Vòng 112: bisect sạch — ghi stack OK, đọc stack hỏng

Thí nghiệm tách đôi (giữ nguyên chương trình cũ ở bước 1):

- **B1**: thêm `frame_write(stack_frame, 0x100, "HI FS\n")` song song với việc
  ghi trang code như cũ → **xanh**, không có `STACK ARGS BAD`. Kết luận: ghi
  vào frame stack **thành công và không phá gì**.
- **B2**: đổi chương trình sang đọc chuỗi từ `EXEC_ARGS_VA` (= `0x10_0002_3100`)
  với `addi a0, a2, 0; addi a1, x0, 6` → `EXEC OK` nhưng **không in gì**.
- Suy luận: `frame_write` báo Ok và địa chỉ đã kiểm chứng bằng tay
  (`auipc 0x22; addi -0x20`), nên khả năng cao nhất là **trang stack không được
  map (hoặc map vào vspace khác) trong vspace của task** ⇒ đọc → fault → task
  chết im lặng (exec TCB không có fault endpoint nên không có dấu vết nào).
- Bước kiểm chứng kế tiếp (vòng sau, rất rẻ): cho chương trình đọc 1 byte tại
  `EXEC_STACK_TOP - 8` (nơi SP trỏ tới, chắc chắn phải map) và so với đọc tại
  `EXEC_STACK_BASE + 0x100`; nếu cả hai đều fault thì lỗi ở `frame_map` cho
  trang thứ hai của cùng vspace — một bug thật đáng sửa.
- Đã revert B1/B2; cây về HEAD `6fdad21`, gate xanh (QEMU 2/2×3, build 0 warning).

## 9.108 — Vòng 112b: đọc ĐỈNH stack cũng fault ⇒ nghi vấn lớn hơn

- Thí nghiệm quyết định: root ghi `"HI FS\n"` vào **offset 0xFF8** của frame
  stack (tức `EXEC_STACK_TOP - 8`, nơi SP trỏ tới) và chương trình đọc đúng
  địa chỉ đó (`auipc 0x23; addi -0x128` → `0x10_0002_3FF8`).
- Kết quả: `EXEC OK`, **không** có `TOP WRITE BAD` (frame_write Ok) nhưng
  chương trình **vẫn không in gì** — giống hệt trường hợp đọc `base+0x100`.
- Ý nghĩa: không phải "sai offset trong trang" mà là **trang stack không truy
  cập được từ chương trình exec** (đọc → fault → task chết im lặng vì exec
  TCB không có fault endpoint). Đây là **bug thật**: mọi chương trình thật
  dùng stack sẽ chết ngay, dù demo hiện tại (không dùng stack) vẫn in được.
- Bước tiếp theo (vòng sau): in kết quả `map_frame_fallible` cho trang stack
  ngay trong spawn, và kiểm tra `tcb_configure` có đúng vspace không — nếu map
  Ok mà vẫn fault thì nghi vấn đề **gắn page table vào vspace/TLB** khi map
  trang thứ hai trong cùng vspace.
- Đã revert; cây về HEAD `e2a376c`, build 0 warning.

## 9.109 — Vòng 113: bài học mã hoá + thí nghiệm gắn fault endpoint

- **Bài học quan trọng về viết test trong asm tay**: `auipc` lấy địa chỉ **của
  chính lệnh đó**, không phải `entry`. Các test trước đặt `auipc a0, 0` ở giữa
  chương trình nên `a0 = entry + n*4 + off` → in ra NUL (vô hình) hoặc sai
  vùng, khiến ta tưởng "không truy cập được". Từ nay mọi test phải đặt `auipc`
  ở **lệnh đầu tiên** hoặc trừ bù đúng số byte đã trôi.
- Thử gắn fault endpoint cho TCB exec (`tcb_configure_fault`) để mọi lỗi bộ
  nhớ được báo về root: **không dùng được** vì TCB exec không có cspace/cap
  chứa endpoint (các demo fault phải `copy()` endpoint vào cspace nạn nhân
  trước). Muốn làm phải cấp cspace cho exec — việc lớn hơn, hoãn.
- Trạng thái cây: revert toàn bộ, về HEAD `002ab78`; gate xanh (QEMU 2/2×3,
  0 warning, headroom 199 024 B cả hai bản).

## 9.110 — Vòng 114: xác nhận lỗi "exec không đọc được stack" bằng đo đạc chuẩn

Ba bước kiểm chứng độc lập (riscv64), loại trừ mọi sai sót mã hoá:

1. **Đường đo cũ đúng**: chương trình `auipc a2,0 ; addi a2,a2,0x40` (auipc ở
   lệnh ĐẦU) đọc `entry+0x40` → in `HI FS` ✔ (chứng minh khung test chuẩn).
2. **Hằng số layout đúng**: root in `STK 68719620096/68719624192` =
   `0x10_0002_3000`/`0x10_0002_4000` — khớp `CODE_VA+0x23000/0x24000`.
3. **Đọc stack vẫn hỏng**: chương trình `auipc a2,0x23 ; addi a2,a2,-0x128`
   (`auipc` ở lệnh đầu) đọc `EXEC_STACK_TOP-8` → **không in gì**, trong khi
   `frame_write` vào đúng offset đó trả `Ok`.

⇒ Lỗi nằm ở phía **map trang stack trong vspace của task**, không phải test.
Ghi thành "lỗi đang mở" trong runbook (mục mới) kèm 3 hướng điều tra: klog
trong `map_frame`, cấp cspace tối thiểu cho exec để nhận FAULT record, hoặc
thử map stack ở VA khác để xem có phải lỗi VA-specific.

- Gate: QEMU 2/2×3, build 0 warning, docs PASS, headroom 199 024 B.

## 9.111 — Vòng 115: PTE riscv set sẵn A/D (đúng đặc tả) — không phải nguyên nhân

- Đọc `vifast-pte`: `encode_leaf` cho riscv chỉ set `V|U|R|W|X`, **thiếu bit
  A (accessed) và D (dirty)**. Kernel không có handler A/D fault ⇒ trên CPU
  không có Svadu, **mọi truy cập** (kể cả fetch) sẽ fault.
- Sửa: luôn set `PTE_A | PTE_D` trong leaf (đúng cho kernel "set once"),
  test pte vẫn xanh (11 test). Đây là cải thiện robustness thật.
- Kiểm chứng giả thuyết: chạy lại test đọc `EXEC_STACK_TOP-8` sau khi sửa →
  **vẫn không in** ⇒ A/D **không phải** nguyên nhân lỗi stack (QEMU đang tự
  cập nhật A/D).
- Trạng thái: giữ thay đổi PTE; lỗi "exec không đọc được stack" vẫn mở, đã ghi
  trong runbook với 3 hướng điều tra phía kernel.
- Gate: QEMU 3/3×3, persist 3/3, harness 535, kernel 183, docs PASS, fmt sạch,
  0 warning, headroom 199 024 B.
