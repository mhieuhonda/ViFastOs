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

- `vifast-kernel` 83/83, PTE 11/11, ELF 17/17; runtime/user compile.
- QEMU 3/3 PASS ×3; 4 USER FAULT, có `0x80000000`, `0x1000002000`,
  `0x1000301000`; `FAULT EP OK`; `RUST MODULES OK`, `VFS SERVER OK`, `VFS OK`,
  `IOMMU OK`; RISC-V/AArch `SMP OK (... ipi=1)`; AArch `BLOCK OK`,
  `FS DISK OK`, `FS PERSIST OK`.
- Boot Untyped: RISC-V 20/32 pages, AArch/x86 22/32; free 12/10 pages,
  12/12 objects.
- DATA UNTYPED lúc test-exit: RISC-V 36/64 pages, AArch64 37/64 pages,
  x86 41/64 pages; 21/48 object (x86 25/48).
- ROOT SWITCH lần chạy cuối: 35 / 35 / 35 (timer test-exit).
- Root ELF: RISC-V 12880 B, AArch64 12704 B, x86 16744 B; headroom RISC-V
  565200 B.
- SHA-256 `ViFastOs-architecture-summary.txt`: `d3e79c680b03697cd75f95dc3d8293cd7c7c23c3cdf878a0b3577ff65f9b718a`.

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
