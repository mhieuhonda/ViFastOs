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
