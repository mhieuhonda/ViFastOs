# ViFastOs — Báo cáo Agent 2, vòng 17

**Ngày:** 2026-11-05
**Baseline:** `34db2e0` / bundle vòng 16
**Code commit:** `85cbdb9` (Task 1–7 trong một commit vì cùng sửa ABI/kernel/platform/runtime; test riêng từng task bên dưới)
**Docs commit:** `docs: round 17`

## 1. Bảng task

| Task | Kết quả | Commit / bằng chứng |
|---|---|---|
| 1. Tick theo hart | ✅ | `85cbdb9`; `B=20` 10/10 mỗi kiến trúc; 50/50 |
| 2. Panic mã lỗi | ⚠️ một phần | AArch/x86 đạt; RISC-V QEMU vẫn rc=0 |
| 3. Chặn BlockRead/Write trực tiếp | ✅ | `BlockDevice` cap; client `BLOCK DENIED OK` |
| 4. `vifast-fs` journal + crash test | ✅ | host 7/7; 66 điểm crash; seed `0x12345678` |
| 5. Server trên FS v2 | ✅ | `FS V2 OK`, `FS FSCK OK`; server ELF 45.480 B |
| 6. Persistence/fsutil | ✅ | 6 phase × 3 kiến trúc |
| 7. CapMint/badge/quyền | ✅ | cap badge 1/2; `FS PERM OK` |
| 8. Docs | ✅ | checklist R3/R4/D5; bundle Part 2 lấy code commit từ git |

Task 2 ghi rõ là **chưa đủ** theo R2: QEMU `sifive_test` của RISC-V không mang
exit-code; đã đóng gói và log phần này thay vì che.

## 2. Test

```text
./scripts/build.sh                 PASS, 0 warning
vifast-kernel                      90/90
vifast-arch                        6/6
vifast-pte                         11/11
vifast-elf                         17/17
vifast-acpi                        8/8
vifast-fs                          7/7
vifast-fsutil                      0/0 (build host binary PASS)
QEMU --repeat 10                   10/10 cả 3 kiến trúc
riscv64 --repeat 50                50/50
aarch64 --repeat 50                50/50
x86_64 --repeat 50                 50/50
test-boot-handoff --repeat 10      10/10
test-panic-hart1 --repeat 5        5/5 mỗi kiến trúc (mã thoát xem Task 2)
test-persist.py                    6/6 phase cả 3 kiến trúc
test-harness-spec.py               263 case âm PASS
cargo fmt --check                  PASS
check-docs.py                      PASS
```

RISC-V headroom: `400912` byte, `kernel_end=0x8039e1f0`.

## 3. Task 1 — tick theo hart

`Kernel.ticks` đổi thành `[u64; MAX_HARTS]`, `GetTime` đọc `ticks[hart]`;
test-exit, `PREEMPT OK` và `B` dùng `ticks[0]` BSP. `SMP TIMER RATE` in cả
RISC-V/AArch64/x86; harness kiểm `B >= 5`, `B/2 <= T <= 2B`, mỗi dòng một lần.

10 lần đầu, `T` từ `SMP TIMER OK`, `B` từ `SMP TIMER RATE`:

```text
RISC-V  B=20,20,20,20,20,20,20,20,20,20   T=21,20,21,21,20,21,21,21,21,21
AArch64 B=20,20,20,20,20,20,20,20,20,20   T=20,20,20,20,20,20,21,20,20,20
x86_64  B=20,20,20,20,20,20,20,20,20,20   T=18,18,18,18,18,18,18,18,18,18
```

## 4. Task 2 — panic exit

- **AArch64:** semihosting `SYS_EXIT_EXTENDED`, block `[0x20026, code]`, QEMU
  `-semihosting`; log thực tế 5/5 `rc=1`.
- **x86_64:** giữ `isa-debug-exit`; 5/5 `rc=7`.
- **RISC-V:** map `sifive_test` tại `0x10_0000`, page table thấp kernel-only,
  panic ghi `FINISHER_FAIL=0x3333`; QEMU 11.1.3 thoát với `rc=0` vì thiết bị
  này không có trường mã thoát. Đã thử semihosting `SYS_EXIT_EXTENDED`/`SYS_EXIT`
  trước đó, QEMU RISC-V cũng trả 0; không dùng wrapper giả để đổi mã.
  Đây là **R2 chưa đạt** trên RISC-V, không phải flake; panic log vẫn đúng
  `PANIC (hart=1)` và không có output hart 0 sau dòng đó.

## 5. Task 3 — block capability

Trước đây 43/44 chỉ kiểm pointer, không lookup cap (baseline
`crates/vifast-kernel/src/lib.rs:3162-3175`). Nay:

- `ObjType::BlockDevice=10`; runtime retype từ metadata Untyped và cài
  `BLOCK_DEVICE_SLOT=13`; BootInfo v3 có trường `block_device`.
- Root copy cap đó chỉ cho server slot 2; client không có cap.
- Syscall args mới: `a0=cap`, `a1=LBA`, `a2=buffer`; lookup cap trước pointer.
- Host test `block_syscalls_require_a_block_device_capability` kiểm no-cap /
  wrong-type / cap đúng; QEMU client in `BLOCK DENIED OK` đúng một lần.

## 6. Task 4 — `vifast-fs`

Host test `vifast-fs` 7/7:

```text
basic_namespace_and_data
journal_replay_and_torn_commit
crash_every_write_keeps_fs_consistent
errors_and_limits
legacy_migration
crash_every_write_for_mutations
random_2000_operations_with_checks
```

- Crash suite 6 thao tác (create/write/truncate/unlink/rename/mkdir), tổng
  `66` điểm crash từ `k=0` tới toàn bộ số write; sau mỗi crash mount lại và
  `fsck` sạch, trạng thái cũ hoặc mới đầy đủ.
- Random: 2000 thao tác, seed cố định `0x12345678`, fsck mỗi 100 bước và crash
  ngẫu nhiên tại các bước chia 37.
- Torn journal: commit CRC hỏng, mount bỏ qua, dữ liệu cũ còn.
- Migrate `VF01`, file tối đa 6 KiB, inode cạn, tên dài, EACCES/ENOTEMPTY/
  ENOENT/EEXIST/EISDIR/ENOTDIR/EINVAL.
- **Giới hạn test:** với 64 inode cố định, thư mục 12 block không thể đầy bằng
  protocol (tối đa 64 entry); ENOSPC vật lý 4 MiB cũng không đạt bằng file
  6 KiB. Đã test ENOSPC qua file >6 KiB và inode cạn; hai trường hợp “đầy
  thư mục/đầy đĩa vật lý” ghi vào nợ thay vì tạo test giả.

## 7. Task 5 — server VFS v2

- Server dùng `Fs<ServerDev>` trong `.bss`; journal 16 data block (8 KiB)
  không nằm trên stack; server stack tăng từ 1 lên 3 trang để chứa call frame
  của FS (vẫn không dùng stack cho journal).
- Server ELF stripped: RISC-V `45.640` byte, AArch64/x86 `45.480` byte.
- Data Untyped còn lại sau boot/test: RISC-V `45/64` dùng (19 trang free),
  AArch64 `46/64` (18 free), x86_64 `48/64` (16 free).
- RISC-V headroom: `400912` byte.
- Output giữ `FS DISK OK`, `VFS SERVER OK`, `ViFastOS ramfs`,
  `ViFastOS write`, `VFS OK`, `VFS STAT OK`; thêm `FS V2 OK`, `FS FSCK OK`,
  `FS PERM OK`; cấm `FS CORRUPT`, `FS V2 BAD`, `FS PERM BAD`.

## 8. Task 6 — persistence log một kiến trúc

RISC-V:

```text
phase 1 blank format PASS       -> FS FORMAT OK, không FS PERSIST OK, host fsck
phase 2 two-way persistence     -> đọc n1, ghi n2 đảo, host get n2
phase 3 migration PASS          -> FS MIGRATE OK, /scratch giữ nonce, LBA0=0
phase 4 valid journal replay    -> FS RECOVER OK (replayed=1), /j mới
phase 5 torn journal PASS       -> FS RECOVER OK (replayed=0), /j cũ
phase 6 corrupt superblock      -> format lại, không PERSIST OK
```

## 9. Task 7 — badge

`Message.badge` được điền từ endpoint cap lúc Send/Call; nhánh blocked
`Platform::write_message` đưa badge vào `value`, nên server đọc `request.value`
làm uid. Root dùng `CapMint` slot 11 tạo endpoint badge 1 (client slot 1) và
badge 2 (slot 6). Client tạo `/priv` bằng badge 1, badge 2 đọc được, ghi bị
`EACCES`, `UNLINK /priv` bị `EACCES`; `FS PERM OK` đúng một lần. Host test
`capmint_copies_endpoint_with_badge` kiểm badge 7.

## 10. Nợ và lệch

- R2 RISC-V rc=0 (QEMU `sifive_test` không mang code) — **P1 còn mở**.
- R4: BKL hart 1 spin ~10x số acquire do `GetTime`; chưa tối ưu.
- `flush/barrier` cho block thật, block cache, indirect block, hard link,
  symlink chưa làm.
- Thư mục/đĩa đầy vật lý không tạo được trong giới hạn 64 inode / 6 KiB.
- `CapDelete` vẫn placeholder; per-hart runqueue, per-VA TLB, x2APIC, IOMMU
  phần cứng, net/input, compositor còn nợ.
