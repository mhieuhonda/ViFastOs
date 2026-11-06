# ViFastOs — Báo cáo vòng 28 (M1.2 + thử M1.3)

**Ngày:** 2026-11-06  
**HEAD đầu vòng:** `3ec9ccb`  
**Mốc:** M1.2 DONE (a/b/c), M1.1 BLOCKED, M1.3 BLOCKED lần 1/3.

## 1. Bảng task

| ID | Trạng thái | Commit | Bằng chứng |
|---|---|---|---|
| M1.2 host test | DONE | `d3f6d77` | tái hiện chuỗi reset → root → 2 TCB/VSpace → configure/resume |
| M1.2a | DONE | `809a1da` | thêm `Tcb.context_slot`, chưa dùng; QEMU 3/3×3 |
| M1.2b | DONE | `91f7496` | cấp slot dense mọi đường tạo TCB + test unique/reuse |
| M1.2c | DONE | `d1b998f` | runtime + platform dùng slot; **ROOT ERR 16 đã giải** |
| M1.1 HWM | BLOCKED | — | 4 biến thể, AArch/x86 đỏ; 2 phiên liên tiếp → inbox |
| M1.3 TCB pool | BLOCKED (1/3) | — | host 134/134 + Object 4.152 nhưng AArch 0/3; đã revert |
| Docs/process | DONE | `42abf6b`, `dd90468` | inbox + decision log |

## 2. Checklist 6.2

| Mục | Đạt? | Ghi chú |
|---|---|---|
| HISTORICAL | Có | không sửa ở vòng này |
| DONE docs có grep | Có | self-test 11 câu cấm |
| Danh sách tay/generated | Có | limits generated |
| Status repeat 3 | Có | `qemu-status-standard.json` |
| pass render đủ | Có | boot stack, 6+F, feature |
| 1 task/commit | Có | a/b/c riêng |
| git log đủ | Có | mục 7 |
| BLOCKED có thử | Có | HWM 4 biến thể; pool 1 lần |
| Không so khác arch | Có | bảng tách arch |

## 3. Footprint trước/sau (3 kiến trúc)

| Hạng mục | Trước (M1.1) | Sau M1.2c | Thử M1.3 (revert) |
|---|---:|---:|---:|
| `size_of::<Object>()` | 4.752 | 4.752 | **4.152** |
| `size_of::<Kernel>()` | 710.248 | 710.248 | **706.576** |
| headroom RISC-V | 253.264 | **290.128** | **286.032** |
| `kernel_end` | 0x803c22b0 | 0x803b92b0 | 0x803ba2b0 |
| `TrapFrame` | 280/288/160 | 280/288/160 | 280/288/160 |

BOOT UNTYPED: RISC-V 24/32, AArch64 27/32, x86_64 28/32. DATA cuối:
69/76 (66 obj), 72/76 (68), 74/76 (70).

## 4. Số liệu test

- Host: kernel **134**, abi 4, arch 7, pte 11, elf 17, acpi 8, fs 22 → **203**.
- Harness âm 462; QEMU chuẩn **3/3 ×3**; feature 5/5 ×3; persistence 7×3
  (cổng M1.2c ALL CHECKS PASS); warning **0**.

## 5. Nguyên nhân ROOT ERR 16

Runtime vẫn index `CONTEXTS` bằng `tcb.index` sau khi mảng đổi sang
`MAX_TCBS`; `ConfigureTcb/RestartTcb` dùng `get_mut(index)` OOB → trả
`IllegalOperation (16)` ngay sau BOOTINFO. Sửa 6 điểm runtime sang
`kernel.context_slot(tcb)` (root context, configure, restart, switch_blocked,
resume_blocked, on_timer).

## 6. Output QEMU RISC-V (M1.2c)

```text
DESTROY OK (frames=100, vspaces=10, reused=100, leaked=0)
ROOT READY
...
ROOT SWITCH 19xx
CTX SIZE 280
DATA UNTYPED pages=69/76 objects=66/76
BOOT STACK used=21752 size=196608
```

## 7. Commit

```text
$ git log --oneline 3ec9ccb..HEAD
dd90468 docs: record tcb pool attempt and aarch blocker
42abf6b docs: record pool hwm blocker and route to tcb pool
```
(code M1.2 nằm trước `3ec9ccb`, đã ghi ở báo cáo 27)

## 8. Task kẹt

**M1.1 HWM:** 4 biến thể (field+scan, field+incremental, static atomic,
runtime read-only sampling) đều làm AArch64 lỗi sau BOOTINFO; RISC-V xanh.
BLOCKED 2 phiên → inbox, chuyển M1.3.

**M1.3 TCB pool (1/3):** triển khai đầy đủ `Kind::Tcb(TcbId)` +
`Kernel.tcbs[22]`, host test 134/134, Object 4.152, Kernel 706.576, headroom
286.032; QEMU RISC-V 3/3 nhưng **AArch64 0/3**
(`KERNEL EXCEPTION vector=4, ELR=0x40092fa0, FAR=0xab5b8`) và x86 2/3
(1 timeout). Đã revert. Giả thuyết: lỗi nền AArch64 với thay đổi layout
object/.bss; cần debug bằng thay đổi tối thiểu (in SCTLR/TTBR/VBAR tại
exception) trước khi thử lần 2.

## 9. Câu hỏi cho agent0

1. Ưu tiên debug AArch64 layout (in system registers) hay thử nhánh thay thế
   tối thiểu (chỉ tách `cspace`/`fds` khỏi Tcb, giữ state trong Object)?
2. Có chấp nhận tạm bỏ HWM và dùng quota cố định (MAX_TCBS=22,
   MAX_CNODES=8, MAX_VSPACES=8) cho M1.3–M1.5 không?
