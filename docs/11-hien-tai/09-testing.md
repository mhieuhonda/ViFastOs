# 11.9. Kiểm thử và tiêu chí PASS

Cập nhật: 2026-11-06, sau vòng 22.2 (HEAD `c86de49`, code `13c26f8`).
Số liệu dưới đây là số liệu thật của baseline; khi code đổi, chạy lại các lệnh
ở mục 1 rồi cập nhật tài liệu này.

## 1. Cổng kiểm tra

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./scripts/check-all.sh        # cổng đầy đủ: fmt + build + QEMU + persist + harness + docs
```

`check-all.sh` lần lượt chạy:

```text
cargo fmt --all -- --check
./scripts/build.sh
python3 scripts/test-qemu.py --repeat 3
python3 scripts/test-persist.py
python3 scripts/test-harness-spec.py
python3 scripts/check-docs.py
```

Cổng nhỏ khi chỉ sửa tài liệu:

```bash
python3 scripts/gen-docs.py
python3 scripts/check-docs.py
cargo fmt --all -- --check
```

`check-docs.py` phụ thuộc `target/vifast-qemu-last.json`. Nếu chạy QEMU với
`--repeat` khác lần ghi trong summary, phải chạy `gen-docs.py` trước
`check-docs.py`, nếu không sẽ báo "summary generated sections are stale".

## 2. Build — `scripts/build.sh`

Thứ tự:

1. `cargo test` cho các crate host: kernel, arch, pte, elf, runtime, user,
   acpi, fs, fsutil (runtime/user/fsutil hiện không có unit test).
2. `cargo test -p vifast-kernel layout_sizes_for_status -- --nocapture`
   (in kích thước `Kernel`/`Object`/`Tcb`/`Cap`).
3. `cargo build -p vifast-fsutil` (công cụ host cho persistence).
4. `python3 scripts/test-harness-spec.py` (462 case âm).
5. Build root/server/client ELF cho 3 target, `llvm-objcopy --strip-all`,
   kiểm tra bằng `scripts/check-root-elf.sh` (ET_EXEC, segment/window,
   soft-float).
6. Build 3 kernel bare-metal (`riscv64gc`, `aarch64`, `x86_64-uefi`).

Kết quả baseline 2026-11-06:

```text
root ELF OK: riscv64imac   size=22960 bytes memsz=17064
root ELF OK: aarch64       size=26784 bytes memsz=20268
root ELF OK: x86_64        size=29440 bytes memsz=19431
build complete (0 warning)
```

## 3. Host test

| Crate | Số test | Ghi chú |
|---|---|---|
| `vifast-abi` | **8/8** | protocol pager v2: label, VA window, encode/decode |
| `vifast-kernel` | **201/201** | + 2 test fuzz dài `#[ignore]`, chạy `--ignored` |
| `vifast-arch` | **7/7** |  |
| `vifast-pte` | **11/11** |  |
| `vifast-elf` | **19/19** |  |
| `vifast-acpi` | **8/8** |  |
| `vifast-fs` | **31/31** | định dạng đĩa, biên block, fsck; nằm trong cổng từ vòng 166 |
| `vifast-virtio` | **7/7** | checksum TCP/IP; nằm trong cổng từ vòng 167 |
| `vifast-runtime`, `vifast-user`, `vifast-fsutil` | 0 | kiểm qua QEMU/build |
| **Tổng duy nhất** | **292** | |

Phạm vi chính của `vifast-kernel`:

- layout `Kernel`/`Object`/`Tcb`/`Cap`, boot Untyped, capacity;
- object/capability: copy/mint/derive/revoke, badge, quyền monotonic, cây cap,
  `CapDelete`/`CapRevoke`, CNode phẳng;
- IPC: Send/Call/Recv/Reply/ReplyRecv, queue, stale epoch, IPC buffer;
- notification, fault endpoint, `TcbBindNotification`;
- scheduler: priority, round-robin, per-hart queue, work stealing, affinity,
  `Sleep`, `BlockedBudget` (SchedContext budget/period);
- TCB: configure/resume/suspend/restart/destroy/reap, quota;
- VSpace/Frame/PageTable: generic walk, W^X, PA uniqueness, W^X theo Frame,
  user range validation, page table rollback, revoke theo thứ tự;
- Untyped: accounting, quota, con, reparent, `OutOfMemory`;
- IRQ object: `IrqGet`/`IrqSetNotification`/`IrqAck`, raise coalesce;
- invariant checker sau các thao tác.

`vifast-fs` kiểm format VF04/VF03/VF02, journal replay/torn commit, hard link,
symlink, double indirect, `O_APPEND`, `O_EXCL`/`O_DIRECTORY`, `STAT_PATH`,
fsck, cache write-back, `fsync`.

`vifast-pte` kiểm encode/decode, `table_span`, `KERNEL_SHARED_SLOTS`, dải user
thật 3 kiến trúc, leaf L3 AArch64. `vifast-elf` kiểm header/phdr, range,
congruence, W^X, overlap, entry, segment không hỗ trợ.

Fuzz syscall: test dài 100.000 bước × 3 PTE format nằm sau `#[ignore]`;
bản ngắn hơn chạy mặc định.

## 4. QEMU — `scripts/test-qemu.py`

### Lệnh

```bash
python3 scripts/test-qemu.py                 # 1 lần mỗi kiến trúc
python3 scripts/test-qemu.py --repeat 3      # 3 lần mỗi kiến trúc
python3 scripts/test-qemu.py --arch riscv64  # một kiến trúc
python3 scripts/test-qemu.py --feature test-panic-hart1   # mutation test
```

### Máy ảo

| Kiến trúc | QEMU | Ghi chú |
|---|---|---|
| RISC-V 64 | `-machine virt -smp 2 -m 128M -bios default` | OpenSBI; VirtIO-MMIO block |
| AArch64 | `-machine virt,gic-version=2 -cpu max -smp 2 -m 128M` | kernel ELF trực tiếp |
| x86_64 | `-machine q35 -cpu max -smp 2 -m 256M` + OVMF + FAT ESP | `isa-ide` + `ide-hd` ở `ide.6`; `isa-debug-exit` 0x501 |

Timeout: 20 s (RISC-V/AArch64), 40 s (x86).

### Điều kiện PASS

Nguồn sự thật là `scripts/harness_spec.py`; `test-qemu.py` gọi `accept()`.
Tóm tắt:

- **Chung**: `SELFTEST PASS`, `booting on`, `BOOT UNTYPED`, `PREEMPT OK`,
  `Isolation`, `Server`, `Client`, `SLEEP OK`, `Mapped`, `Cap OK`,
  `ViFastOs ramfs`, `ViFastOs write`, `FS V2 OK`, `FS FSCK OK`,
  `FS PERM OK`, `BADGE SPOOF DENIED OK`, `IPC BUF OK`, `BLOCK DENIED OK`,
  `Reply OK`, `ROOT RUST`, `BOOTINFO OK`, `SCHED CTX OK`,
  `RUST MODULES OK`, `CLIENT VSPACE OK`, `FAULT EP OK`, `ROOT READY`,
  `Zero OK`, `BLOCK OK`, `FS DISK OK`, `PAGER RECLAIM OK`, `RUN FS OK`,
  `HIARG`, `WX OK`, `MEM OOM OK`, `FS USAGE OK`, `KILL OK`,
  `SLOT REUSE OK`, `IPC DEAD OK`, `IPC JUNK OK`, `SEC PROBE OK`,
  `SYS REJECT OK`, `MEM FREE OK`, `FS MOUNT CLEAN`, `FS OPS FSCK OK`,
  `FS RENAME OK`, `FS LINK OK`, `FS TRUNC OK`, `FS RM OK`, `PROG MID OK`,
  `PROG TAIL OK`, `PROG SIZE OK`, `PAGER SRV 0/0/0`
- **Đúng một lần**: `VFS SERVER OK`, `VFS OK`, `VFS STAT OK`, `IOMMU OK`,
  `PERCPU CLOBBER OK`, `TIME MONOTONIC OK`, `TLB REMAP OK`,
  `CAP DELETE OK`, `TCB SUSPEND OK`, `FS LARGE OK`, `FS APPEND OK`,
  `FS OPEN FLAGS OK`, `FS STAT PATH OK`, `HARD LINK OK`, `SYMLINK OK`,
  `IRQ OK`, `CNODE OK`, `TCB REAP OK`, `WAIT REAP OK`, `PAGER OK`,
  `PAGER2 OK`, `PAGER EVICT OK`, `PAGER3 OK`, `SHELL READY`, `DESTROY OK`
- **Fault**: đúng **6** dòng `USER FAULT`; phải có các địa chỉ `0x80000000`,
  `0x1000002000`, `0x1000301000`, `0x1001000000`, `0x1001001000`
  (record còn lại hiện là `0x0`).
- **Không được xuất hiện**: các marker `... BAD` tương ứng (gồm `DESTROY BAD`), `ROOT PANIC`,
  `STALL`, `BOOT HANDOFF FAIL`, `KERNEL IRQ OUTSIDE IDLE`, `AP PIC IRQ`,
  `FS CORRUPT`.
- **RISC-V/AArch64**: `SMP OK (... ipi=1)`, `SMP TIMER RATE`, `SMP TIMER OK`,
  `SMP IDLE OK`, `SMP PREEMPT OK`, `SMP SCHED OK`, `SMP WORKERS OK`,
  `SMP POST IDLE OK`, `TLB STATS`; AArch64 yêu cầu TLB `ipi == 0`
  (broadcast), RISC-V `ipi >= 1`, `skipped >= 1`.
- **x86_64**: thêm `KERNEL DATA NX OK`, `ACPI MADT OK (cpus=2)`, `LAPIC OK`,
  `AP TRAMPOLINE OK (page < 0xA0000)`, `AP REAL MODE OK` → `AP LONG MODE OK`
  → `SMP OK` đúng thứ tự, `SMP IPI COUNT (1 <= seen <= sent)`.
- **Exit code**: RISC-V 0, AArch64 0, x86_64 **3** (kernel ghi 1 vào
  `isa-debug-exit`).

### Mutation test

`--feature test-panic-hart1` build kernel panic trên hart 1; harness PASS khi
exit code khác 0 và chỉ có đúng một `PANIC (hart=1)`, không có log sau đó.
`--feature test-skip-tlb-ipi` và `--feature test-boot-handoff` tương tự cho
TLB/boot handoff.

## 5. Persistence — `scripts/test-persist.py`

Dùng `target/debug/vifast-fsutil` và QEMU với disk tạm mới mỗi phase. 7 phase
× 3 kiến trúc:

1. blank format;
2. two-way persistence;
3. migration VF03/VF02 → VF04;
4. valid journal replay;
5. torn journal;
6. corrupt superblock (negative);
7. file lớn double-indirect.

Kỳ vọng: mỗi kiến trúc in `PASS`, exit 0; tổng 21 dòng phase PASS.
Script không có `--help`; gọi không tham số là chạy toàn bộ.

## 6. Harness âm — `scripts/test-harness-spec.py`

462 case: mỗi điều kiện trong `harness_spec.py` bị xoá khỏi output tổng hợp và
kiểm tra rằng `accept()` trả `False`. Case đặc biệt: fault count/address,
SMP counter, thứ tự marker, exit code, feature mode.

## 7. Debug

- Serial file: `/tmp/vifast-riscv64.serial`, `/tmp/vifast-aarch64.serial`,
  và `serial.txt` trong thư mục tạm x86.
- Khi timeout, `test-qemu.py` ghi `artifacts/flake-<arch>-<n>/` gồm
  `registers.txt` (qua QEMU monitor), `tail.txt`, `status.txt`.
- Nếu x86 treo trước banner: kiểm tra OVMF code/vars và ESP.
- Nếu RISC-V/AArch64 thiếu `SMP OK`: kiểm tra `-smp 2` và HSM/PSCI.
- `VIFAST_KEEP_SERIAL=1` giữ serial từng lần chạy (`vifast-keep-<arch>-<n>.serial).

## 8. Runbook sự cố thật (mốc 1-9, vòng 9-16)

1. **Client treo sau `shell>` (không PREEMPT): stack tràn.** Client stack chỉ
   4 KiB; buffer lớn để trên stack (vd. lịch sử shell 2 KiB) làm tràn xuống
   dưới `STACK_BASE` → fault không có fault_ep → treo. Dấu hiệu: serial dừng
   ở `shell>`, fault addr trong vùng stack. Chữa: chuyển buffer ra `static`
   hoặc thu nhỏ; đã ghi TODO cấp từ Untyped riêng (mốc 2).
2. **Static `.bss` client to ra treo timer phase (bisect xác nhận).** Thêm
   2 KiB static vào client làm ELF to ra và treo sau boot — nguyên nhân sâu
   chưa rõ, workaround là giữ `.bss` client nhỏ. Nghi ngờ va chạm layout
   với Untyped/data region trên QEMU.
3. **x86 OOM (`ROOT ERR 5`) khi spawn task mới cuối boot.** x86 cạn data
   pages (76/76) vì fault task reap chỉ destroy TCB, stack frame/mapping còn
   đó. Chữa: spawn tối giản (chỉ retype TCB, tái dùng stack mapping cũ của
   task đã reap), hoặc spawn sớm ngay sau reap. Đừng cấp frame/page-table mới
   cuối boot trên x86.
4. **`WAIT REAP BAD` do race đúng.** `TcbWait` trả `Ok` khi target đã exit
   trước (xong ngay), `ObjectDestroyed` khi exit sau (wake lỗi) — cả hai đều
   là rendezvous đúng. Test phải chấp nhận cả hai, không chỉ một.
5. **Interactive test: một lệnh chắc chắn mỗi lần chạy.** Guest tự thoát sau
   PREEMPT nên cửa sổ gõ lệnh hẹp; UART FIFO cứng chỉ 16 byte (gửi dồn mất
   byte). Cách làm: gửi lệnh TRƯỚC (khi shell rỗi, UART đã đọc hết), kiểm tra
   output bắt buộc; lệnh sau là bonus best-effort.
6. **Phân biệt flake hạ tầng và bug thật.** Bisect bằng `git stash`/`checkout`
   + rebuild + chạy đơn lẻ từng arch; flake hạ tầng (vd. x86 2/3 rồi rerun
   3/3) không liên quan thay đổi mã. Tải máy cao (load ~2) làm QEMU chậm và
   timeout giả — `pkill qemu` dọn tiến trình sót trước khi kết luận.

## 9. Chưa có

- Stress SMP > 2 hart trong harness (`-smp 4` mới chỉ chạy thủ công).
- Kani/Verus; soak dài nhiều giờ tự động.
- Test trên phần cứng thật; test network/GPU/driver ngoài QEMU.
- Hosted CI (đã bỏ theo yêu cầu; dùng `scripts/check-all.sh` cục bộ).
