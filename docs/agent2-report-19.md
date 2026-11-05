# ViFastOs — Báo cáo Agent 2, vòng 19

**Ngày:** 2026-11-05  
**Baseline:** `111e072` (bundle vòng 18)  
**Code cuối:** `8342553`  
**Tag an toàn:** `overnight-start` tại `111e072`

## 1. Task → trạng thái → commit → test riêng

| Task | Trạng thái | Commit | Test riêng |
|---|---|---|---|
| 1. Headroom RISC-V | xong | `334e5c1` | host layout test; QEMU 10/10 |
| 2. TLB mutation | xong | `b029aac`* | mutation 5/5 RISC-V+x86; QEMU 50/50 |
| 3. Fuzz 100k | xong | `4688353` | 100.000 bước × 3 PTE, `--ignored` |
| 4. CapDelete | xong | `b029aac`* | 3 host test + QEMU `CAP DELETE OK` |
| 5. Priority/Suspend | xong | `b029aac`* | 3 host test + QEMU `TCB SUSPEND OK` |
| 6. FS single-indirect | xong | `d8b762e` | 12 host test; server 20 KiB; persist phase 7 |
| 7. Double-fetch audit | xong (0 lỗi) | — | bảng audit mục 7 |
| 8. GetTime lock-free | xong | `aa10090` | BKL stats 3 trước/3 sau; QEMU 50/50 |
| 9. User IRQ AArch64 | không bắt đầu | — | stretch, hết ưu tiên |
| 10. Per-hart runqueue | không bắt đầu | — | stretch, hết ưu tiên |
| 11. Docs | xong | `docs: round 19` | check-docs + 327 case âm |
| 12. Soak | một chu kỳ | — | soak log, 0 FAIL |

`*` Tasks 2/4/5 cùng sửa `Kernel::user_syscall`, ABI dispatch, `root_main`,
`trap_handler` và harness; gộp một commit tích hợp `b029aac`. Các task không
dùng chung file được tách commit riêng.

**Thời điểm (commit time, có chồng lấn do sửa chung file):**

| Task | Bắt đầu gần đúng | Kết thúc |
|---|---|---|
| 1 | 18:20 | 18:35 |
| 2/4/5 tích hợp | 18:35 | 19:08 |
| 3 | 19:05 | 19:07 |
| 6 | 18:55 | 19:07 |
| 8 | 19:05 | 19:07 |
| 11/12 | 19:08 | 19:20 |

## 2. Task 1 — headroom

Đo tại `85cbdb9`, `111e072`, `8342553`:

| Section | `85cbdb9` | `111e072` | `334e5c1`+ |
|---|---:|---:|---:|
| `.text` | 66.008 | 68.392 | 71.832 |
| `.rodata` | 86.320 | 86.664 | 86.968 |
| `.data` | 606.992 | 869.136 | 613.144 |
| `.bss` | 664.040 | 664.144 | 664.144 |
| `.stack` | 262.144 | 262.144 | 262.144 |
| headroom | 400.912 | 138.672 | **392.624** |

Nguyên nhân chính: `.data` tăng đúng **262.144 byte** khi `MAX_CAPS` 64→96,
vì `Tcb` (Object lớn nhất) có `cspace` 96 × 64 byte × 128 object.

Sửa: tách `MAX_CAPS_ROOT=96` và `MAX_CAPS_TASK=64`; root dùng `root_cspace`
riêng 96, TCB thường 64. `Kernel` còn 613.056 byte. Guard `build.sh` fail nếu
headroom < 196.608 byte (192 KiB); build hiện tại 392.624 byte.

Top symbol lớn nhất (RISC-V, `llvm-nm --size-sort`): `KERNEL_STORAGE`
(0x95ac0/613.056 B), `USER_DATA` 256 KiB, `BOOT_POOL` 128 KiB,
`TRAP_STACKS` 128 KiB, `METADATA` 64 KiB, `CONTEXTS` 0x8c00,
`riscv_trap_handler` ~0x20xx, runtime boot ~0x3144. Không có buffer mới ngoài
`root_cspace`; phần tăng `.text` là các syscall/suspend/TLB mới.

## 3. Task 2 — TLB mutation

- `TLB STATS` regex đã kiểm `V >= 1`; RISC/x86 `I >= 1`, `K >= 1`;
  AArch64 `I == 0`. Case âm V/I/K có trong `test-harness-spec.py`.
- `harts_loaded` chỉ xóa bởi full-ASID flush; unmap/map đều phát per-VA.
- Worker d đọc liên tục VA trong lúc root `FrameRemap` (không block/không đổi
  root); root remap không có cửa sổ unmapped nhờ `replace_leaf`.
- 10 lần đầu cuối: RISC `V=14 I=5 K=9`, AArch `V=14 I=0 K=0`,
  x86 `V=14 I=3 K=11`; latency RISC/x86 0 tick, AArch 10 tick.
- **Mutation `test-skip-tlb-ipi`:** bỏ IPI target + flush target local.
  RISC-V/x86 đều in `TLB REMAP BAD (stale)` và script báo
  `PASS (expected TLB failure detected)` 5/5. Nếu bỏ mutation, test vẫn
  `TLB REMAP OK`.
- Bản mutation bỏ IPI thật sự làm test fail; QEMU không tự flush trong cửa sổ
  này. Điều kiện harness **không bị hạ**.

## 4. Task 3 — fuzz

Seed `0x5eed_1234_abcd_9876`, 100.000 bước mỗi PTE format (RISC-V/x86/AArch),
gồm CapMint, TcbSetAffinity, BlockRead/Write fake cap, FrameMap/Unmap có mask,
Retype, CapRevoke, CapDelete. `check_invariants` chạy mỗi 1.000 bước và cuối.
Bản mặc định 10.000 bước; bản dài `#[ignore]` chạy `--ignored` 1,08 giây.
Không tìm thấy lỗi; invariant mới: badge nguồn, `harts_loaded` hợp lệ,
không TCB Running hai hart (đã có từ trước).

## 5. Task 4 — CapDelete

Syscall 13 đã nối `delete_cap`. Quy tắc:

- Xóa cap lá; slot dùng lại được.
- Refuse `IllegalOperation` nếu còn object/cap con.
- Refuse xóa TCB/VSpace cap của chính caller (dù kernel còn strong ref) để
  caller không tự khóa đường cấu hình.
- Xóa endpoint cap không hủy endpoint; sender đang chờ vẫn được phục vụ qua
  object/endpoint khác. Client xóa cap badge 2 sau demo, gọi lại nhận
  `InvalidCap`, in `CAP DELETE OK`.
- Host test: leaf/reuse, child refusal, self TCB/VSpace, queued sender.

## 6. Task 5 — priority và suspend

- `TcbSetPriority`: chỉ hạ hoặc giữ; reject nếu priority mới > priority hiện
  tại của target hoặc > priority caller (`PrioTooHigh`).
- `TcbSuspend`: không suspend root/chính caller; Running giữ `current` đến
  timer kế tiếp; Ready bị rút khỏi runqueue; BlockedSend/Recv/Notification bị
  rút khỏi endpoint/notification queue, pending bị hủy và resume trả
  `WouldBlock`; BlockedReply/Faulted bị từ chối.
- `TcbResume` đưa Suspended về Ready và enqueue.
- Demo: C bận chờ flag; root spawn keeper E, suspend C 5 tick, resume, cho C
  bận thêm 5 tick rồi mở flag; C báo elapsed ≥10; in `TCB SUSPEND OK`.
- 50/50 cả 3 kiến trúc.

## 7. Task 6 — FS v3 single-indirect

- Superblock `VF03`; mount được `VF02` cũ (layout 12 direct) và migrate
  `VF01` sang `/scratch`.
- Inode giữ 64 byte: 11 direct + 1 indirect 128 entry; file tối đa
  `139 × 512 = 71.168` byte (~69 KiB). Thư mục tối đa 11 block.
- Ghi lớn chia transaction theo từng block; mỗi block nguyên tử, cả file
  không nguyên tử. Truncate từ vùng indirect về direct giải phóng cả indirect.
- Host test: file 60 KiB, đọc/ghi đúng, truncate còn 1000 byte, fsck sạch;
  crash tại mọi write của lần cấp phát indirect đầu tiên, fsck sạch và size
  chỉ 11/12 block; 12/12 host test.
- Server demo ghi/đọc 20 KiB, checksum từng chunk, in `FS LARGE OK`.
- `test-persist.py` thêm phase 7: `put-file` 20 KiB, guest mount/fsck, host
  đọc lại khớp; 7 phase × 3 kiến trúc PASS.

## 8. Task 7 — audit double-fetch

| Syscall/đường | Đọc user | Kết luận |
|---|---:|---|
| DebugWrite | 1 lần vào buffer | không double-fetch |
| FrameWrite | 1 lần | |
| BlockWrite | 1 lần | |
| BlockRead | 0 đọc; 1 ghi | |
| Open path | 1 lần | |
| Read | 0 đọc; 1 ghi | |
| Write | 1 lần | |
| IPC buffer | copy frame→frame, không đọc user | |
| TcbSetIpcBuffer | không đọc buffer | |
| Stat/Seek/Close | không đọc | |

Không tìm thấy chỗ quyết định dựa trên lần đọc đầu rồi dùng lần đọc sau.
Validation PTE và copy nội dung là hai bước nhưng nội dung chỉ dùng một lần;
không sửa code. Ghi nhận vào mục 16 summary.

## 9. Task 8 — GetTime lock-free

Fast path trong trap trước `bkl_lock`: RISC-V a7=33, AArch x8=33, x86 rax=33
đọc `system_ticks` atomic (BSP cập nhật) và trả ngay. `GetTime` dispatch
thường cũng dùng atomic.

BKL STATS 3 mẫu trước → sau (h1 acq/spin):

| Arch | Trước | Sau |
|---|---|---|
| RISC-V | 416754/2553646, 316940/2396661, 416518/3043449 | 101401/648263, 113202/548152, 147651/690514 |
| AArch64 | 328181/4779122, 371919/4021569, 329196/5019539 | 22176/6109905, 13530/5688774, 10471/4756585 |
| x86_64 | 127309/861349, 88660/619797, 93379/663675 | 18339/190935, 39044/348696, 25738/268107 |

Không thêm assertion tuyệt đối; `h1_spin` còn phụ thuộc scheduling và các
syscall khác, không đủ 10/10 để đặt cận tương đối.

## 10. Task 9, 10

Không bắt đầu: Task 9/10 là stretch, ưu tiên Task 1–8 và docs/soak. Không có
code dở; working tree sạch.

## 11. Task 12 — soak

Xem `docs/soak-log.md`. Chu kỳ cuối: 50/50 mỗi kiến trúc, mutation 5/5 mỗi
kiến trúc, panic 5/5 mỗi kiến trúc, handoff 10/10, persist 7 phase × 3.
Không có FAIL; không có artifact.

## 12. Nợ mới / headroom

- RISC-V headroom **392.624 byte**; guard 192 KiB.
- FS v3 chưa có double indirect/block cache/barrier phần cứng; file tối đa
  69 KiB; ghi file lớn không nguyên tử toàn bộ.
- Task 9/10 stretch chưa làm: user IRQ AArch64, per-hart runqueue.
- `h1_spin` vẫn cao; fast path giảm acquisition nhưng chưa chứng minh cận
  tương đối ổn định.
- CapDelete không hủy object; chỉ bỏ cap.
