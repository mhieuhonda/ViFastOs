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
- SHA-256 `ViFastOs-architecture-summary.txt`: `6603365d8ecd728b7c65e3c6097124443f01543f7097f3ab07d40ce86d0f6d93`.

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
