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
