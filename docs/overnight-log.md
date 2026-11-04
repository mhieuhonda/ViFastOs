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
