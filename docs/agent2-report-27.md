# ViFastOs — Báo cáo vòng 27 (agent2 tự quyết định)

**Ngày:** 2026-11-06  
**HEAD đầu vòng:** `91296ee` (sau C1) / baseline `446a5c3`  
**Mốc:** M0.2 xong, M1.1 PARTIAL, M1.2 chưa làm.

## 1. Bảng task

| ID | Trạng thái | Commit | Ghi chú |
|---|---|---|---|
| M0.1 | DONE | — | baseline `--repeat 3`: 3/3 × 3 kiến trúc |
| M0.2-C1 | DONE | `91296ee` | HISTORICAL "tổng 4", nguồn gốc 2 fault pager, boot Untyped 24/27/28 trang |
| M0.2-C2 | DONE | `e69230e` | khối limits về mục 7, ghi crate, `MAX_FDS`/`MAX_FD`/`MAX_FILES` giải thích |
| M0.2-C3 | DONE | `d793786` | status đọc `target/qemu-status-standard.json`, chặn file cũ hơn build |
| M1.1 | PARTIAL | `216035d` | đo variant + `CTX SIZE`; **POOL HWM BLOCKED** (AArch/x86 exception) |
| M1.2 | SKIPPED | — | chưa bắt đầu; ưu tiên debug HWM theo inbox |
| Quy trình | DONE | `da873a0` | inbox + decisions log + decision-27 |

## 2. Checklist tự review (6.2)

| Mục | Đạt? | Ghi chú |
|---|---|---|
| HISTORICAL không sửa nội dung | Có | C1 giữ câu gốc + `[đính chính vòng 26]` |
| DONE docs có grep | Có | self-test 11 câu cấm; grep sạch |
| Danh sách tay trùng generated | Không còn | C2 chuyển hết sang generated |
| Status từ repeat 3 | Có | C3 file riêng + mtime check |
| pass render đủ điều kiện | Có | boot stack, 6+F, feature |
| 1 task = 1 commit | Có | C1/C2/C3/M1.1 riêng |
| git log đủ commit | Có | mục 8 |
| BLOCKED có thử | Có | HWM đã thử 3 biến thể |
| Không so số khác kiến trúc | Có | bảng footprint tách arch |
| Không né SMP vô lý | Có | — |
| Design doc không lộ PA/index | Có | C4 vòng 26 |
| Bảng footprint 3 arch | Có | mục 3 |

## 3. Footprint

| Hạng mục | RISC-V | AArch64 | x86_64 |
|---|---:|---:|---:|
| `size_of::<Object>()` | 4.752 | 4.752 | 4.752 |
| `size_of::<Kernel>()` | 710.248 | 710.248 | 710.248 |
| `size_of::<TrapFrame>()` | 280 | 288 | 160 |
| headroom | 253.264 (`kernel_end=0x803c22b0`) | — | — |
| BOOT UNTYPED | 24/32 (free 8) | 27/32 (free 5) | 28/32 (free 4) |
| DATA UNTYPED cuối | 69/76, 66 object | 72/76, 68 object | 74/76, 70 object |

Variant: Tcb 4.696, CNode 4.096, VSpace 3.136, Endpoint 200, Untyped 256,
Iommu 1.536, Notification/Frame 24, Irq 20, Cap 64.

**HWM:** BLOCKED — xem mục 7.

## 4. Số liệu test

- Host: kernel **132**, abi 4, arch 7, pte 11, elf 17, acpi 8, fs 22 → **201**.
- Harness âm: **462**; warning **0**.
- QEMU chuẩn: 3/3 ×3; feature destroy-SMP: 5/5 ×3.
- Persistence: RISC-V/AArch64 PASS; x86_64 phase 7 timeout một lần, chạy lại
  **PASS 3/3** (flake chưa tái hiện, log `/tmp/persist27b.out`).

## 5. Output QEMU (RISC-V)

```text
DESTROY OK (frames=100, vspaces=10, reused=100, leaked=0)
ROOT READY
...
ROOT SWITCH 19xx
CTX SIZE 280
DATA UNTYPED pages=69/76 objects=66/76
BOOT STACK used=21736 size=196608
```

Chưa có `POOL HWM` vì BLOCKED.

## 6. Quyết định tự đưa ra

1. C1 làm trước C2/C3 vì rủi ro thấp (đã ghi commit riêng).
2. M1.1 chỉ commit phần đo an toàn (host test + `CTX SIZE`), tách HWM sang inbox;
   không đụng `alloc_owned` sau 3 lần AArch/x86 crash.
3. C3 dùng file `qemu-status-standard.json` thay vì đổi thứ tự check-all.

## 7. Task kẹt

**M1.1 POOL HWM — BLOCKED (3 lần thử).**
- Lần 1: field HWM trong `Kernel` + scan sau alloc → AArch/x86
  `KERNEL EXCEPTION vector=4`, ELR ~0x40091a54, FAR=0xac412 sau `BOOTINFO OK`;
  RISC-V PASS.
- Lần 2: field + counter tăng/giảm → cùng triệu chứng.
- Lần 3: static `AtomicU16` (không đổi `Kernel`) → cùng triệu chứng.
- Đã revert mỗi lần; baseline xanh. Giả thuyết: lỗi nền AArch64 khi kernel crate
  thêm/ghi static trong đường boot (chưa khoanh được trong 90 phút).
- Đề xuất: debug riêng AArch64 với static tối thiểu (inbox mục vòng 26).

## 8. Commit

```text
$ git log --oneline 446a5c3..HEAD
216035d test(M1.1): measure object variants and context size
d793786 test: pin docs status to standard run artifact
da873a0 docs: round 27 self-decision and agent0 inbox
e69230e docs: qualify generated limits by crate
91296ee docs: correct fault provenance and stale boot untyped text
```

(chưa gồm commit docs cuối chứa báo cáo/bundle)

## 9. Cổng cuối

- build: 0 warning.
- QEMU chuẩn 3/3 ×3; feature 5/5 ×3; harness 462.
- `check-all.sh`: lần 1 đỏ ở persistence x86 phase 7 (timeout, log
  `/tmp/final27.out`); chạy lại riêng persistence **PASS 3/3**
  (`/tmp/persist27b.out`). Chạy lại toàn bộ `check-all.sh` lần 2:
  **ALL CHECKS PASS** (`/tmp/final27b.out`) — flake không tái hiện.

## 10. Câu hỏi cho agent0

1. Có ưu tiên debug AArch64/HWM trước M1.2, hay bỏ HWM và đi thẳng tách TCB pool?
2. `Kernel` 710 KiB vẫn chiếm chủ yếu object table; có muốn tách luôn VSpace
   pool ngay cả khi Object đạt ≤256 B không?
