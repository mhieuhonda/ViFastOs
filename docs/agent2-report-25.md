# ViFastOs — Báo cáo vòng 25

**Ngày:** 2026-11-06  
**Baseline:** `ViFastOs-round-24.md` (summary `aa8e7f55...`, code `199ce90`, docs
`54ee3dd`); HEAD đầu vòng: `f1491ff` + hai commit phát sinh `14ad2fe`, `d88627a`.  
**Chế độ:** chạy liền mạch theo `ViFastOs-agent0-decision-25.md`.

## 1. Bảng task

| ID | Trạng thái | Commit | Bằng chứng / phần thiếu |
|---|---|---|---|
| A1 | DONE | `e9f73a7` (gộp A1+A2) | sửa mục 12/15/16, dời 74 vào mục 18; grep sạch |
| A2 | DONE | `d88627a` + `e9f73a7` | self-test 8 câu cấm đều FAIL; bảng errors/limits sinh từ code |
| A3 | DONE | `a4c2a84` | pass render BOOT STACK, fault 6+F, feature; status chặn repeat≠3 |
| A4 | BLOCKED | — | ngân sách duyệt không khả thi với headroom/quota; số liệu mục 3 |
| B1 | DONE | `14ad2fe` + `86aea65` | protocol/layout + 4 host test; pager-v2.md sửa theo S7 |
| B2–B4 | BLOCKED | — | A4 chưa mở được; object quota 76 không đủ ~16 object pager |
| B5 | SKIPPED | — | chỉ làm sau B3/B4 |
| B6 | DONE | (commit docs cuối) | báo cáo + bundle 25 Part 4 |
| C1 | PARTIAL | `fa89e89` | TIMER_INTERVAL AtomicU64 + KERNEL_PTR wrapper 3 platform; còn các `static mut` khác |
| C2 | DONE | — | soak 10/10 ×3 thường + 10/10 ×3 feature; sau đó repeat 3 chuẩn |
| C3 | DONE | `99d0223` + `86aea65` | sửa display/input + pager-v2 theo 5.1 |

## 2. Trả lời S1–S7

**S1 — summary còn câu sai / sửa lịch sử.** Đã sửa:
`grep -n "Chưa có đường user destroy\|quota 16\|cả bốn log USER FAULT"` → không
còn. Mục 15 HISTORICAL khôi phục "từ 7.5 ... đúng 4 USER FAULT" và thêm dòng
riêng "Vòng 22.2 nâng lên 6 USER FAULT". Mục 74 đã nằm trong danh sách mục 18
(trước "19. FILE"). `check-docs --self-test` chèn từng câu cấm và xác nhận
FAIL (8/8), gồm câu "Chưa có đường user destroy..." từng lọt.

**S2 — harness/BOOT STACK.** Cả hai **có thật** trong `harness_spec.accept`:
`BOOT STACK` regex + điều kiện `U*2 <= S`, và `DESTROY SMP OK`/`DESTROY SMP
BAD` cho `feature_mode`. Lỗi là `gen-docs.pass_conditions()` không render các
luật này. Đã thêm mục "Boot stack", "Fault rule", "Feature test-destroy-smp"
vào khối GENERATED pass; `check-docs` đếm đúng 148 dòng.

**S3 — status lấy từ feature.** `gen-docs.qemu_status()` nay từ chối nếu
`repeat != 3` hoặc thiếu kiến trúc; sau soak đã chạy lại `--repeat 3` chuẩn
trước khi gen-docs.

**S4 — ngân sách boot.** Đúng: "boot pool 128→96" là **BOOT_POOL handoff**,
không phải Boot Untyped. Boot Untyped giữ 128 KiB (32 trang); không giảm.
Tuy nhiên A4 vẫn blocked vì lý do khác (mục 3).

**S5 — BLOCKED khi chưa thử.** Đã đổi: C1 PARTIAL (đã thử), B5 SKIPPED (chưa
thử), B2–B4 BLOCKED kèm số liệu. C1 không ghi BLOCKED nữa.

**S6 — mục 74 sai chỗ.** Đã chuyển vào mục 18 trước "19. FILE".

**S7 — pager-v2.md lệch.** Đã sửa: client theo **badge 1..4**, trang file
**READ-only**, pager nhận **Untyped con riêng**, evict = ObjectDestroy rồi
Retype lại, VFS đọc bằng badge riêng (không `0xFFFF`), bỏ TCB index khỏi
protocol.

## 3. Ngân sách (A4) — vì sao BLOCKED

Hiện trạng: headroom **253.264** (`kernel_end=0x803c22b0`), `Kernel=710.248`,
`Object=4.752`, `MAX_OBJECTS=147`, quota hiện 5+34+76+32=147.

| Phương án | Headroom | Object quota | Kết luận |
|---|---:|---|---|
| Data 304→368 KiB (+65.536) | 187.728 | 147 (giữ 76 object) | **dưới 196.608** |
| + giảm BOOT_POOL handoff 128→96 KiB (+32.768) | 220.496 | 147 | đủ headroom nhưng data vẫn 76 object |
| Tăng MAX_OBJECTS 147→163 (+76.032 cho 16 object) | 144.464 | 163 | **dưới guard rất xa** |
| Giữ MAX_OBJECTS 147, data max_objects 76 | 253.264 | 147 | không thêm object cho pager |

Pager cần ~16 object (1 client: VSpace+PT+8 frame+stack; 4 client nhiều hơn),
trong khi data chỉ còn 6–10 slot. Reap sớm 4 fault TCB chỉ giải phóng ~4 frame
data, không đủ. Vì vậy **không áp dụng thay đổi nào** để tránh vỡ hoặc vi phạm
hợp đồng; cần agent0 chọn: (a) nới cửa sổ kernel 4 MiB, (b) tách data arena ra
khỏi kernel image, hoặc (c) giảm tính năng pager (1 client, pool 2 frame).

`MAX_TCBS` 18→22 **không** làm phình `Kernel`/`Object` (Tcb nằm trong enum
Object; Object=4.752 vẫn là variant lớn nhất), nên nếu sau này mở quota chỉ cần
đổi hằng, không tốn headroom.

## 4. Số liệu test

- Host: kernel **131/131**, abi **4/4**, arch 7, pte 11, elf 17, acpi 8, fs 22,
  runtime 0, user 0 → **200** test.
- Harness âm: **462 case** (gồm boot stack + feature + 6+F rule).
- QEMU chuẩn: 3/3 ×3; feature destroy-SMP: 10/10 ×3 (soak) và 5/5 ×3 ở cổng.
- Persistence: 7×3.
- **Cổng đầy đủ:** `check-all.sh` lần đầu đỏ ở bước docs vì chạy feature SAU
  repeat 3 (status lấy repeat 5). Đã đảo thứ tự trong `check-all.sh`
  (feature trước, standard repeat 3 sau) và chạy lại; kết quả ghi ở cuối.
- Warning: **0**; headroom **253.264**.

## 5. Output QEMU (RISC-V)

```text
DESTROY OK (frames=100, vspaces=10, reused=100, leaked=0)
ROOT READY
...
DATA UNTYPED pages=69/76 objects=66/76
BOOT STACK used=21736 size=196608
```

Feature: `DESTROY SMP OK (frames=20, vspaces=2)` sau `ROOT READY`.
Hai dòng DATA UNTYPED quanh pager: **chưa có** vì B2–B4 blocked (đã ghi rõ).

## 6. Ghi đè pager-v2.md

Đã sửa theo 5.1 (badge, READ-only, Untyped con, ObjectDestroy+Retype, VFS badge).
Quyết định tự đưa ra: giữ `PAGER_MAP/UNMAP` là future (root cài backing qua IPC
buffer lúc spawn), vì vòng 25 không có B2.

## 7. Task kẹt

- A4/B2–B4: kẹt ngân sách object/headroom như bảng mục 3. Đã thử tính 3 phương
  án; không áp dụng để giữ baseline xanh.
- C1: còn `CONTEXTS`/`CPU_STATES`/`KERNEL_STORAGE` là `static mut`; đã làm
  TIMER_INTERVAL + KERNEL_PTR, phần còn lại cần vòng riêng.
- Multi-AP x86: không làm theo quyết định.

## 8. Câu hỏi cho agent0

1. Chọn hướng mở ngân sách: nới cửa sổ kernel 4 MiB, tách data arena ra khỏi
   image, hay giảm pager MVP còn 1 client/2 frame?
2. Có chấp nhận `MAX_OBJECTS` tăng và headroom tạm thời dưới 212.992 (vẫn
   ≥196.608) nếu chỉ tăng 4 object thay vì 16?
3. B1 đã có protocol; vòng 26 có cần làm B2 với 1 client trước để đo
   `reused/leaked` không?
4. `check-docs --self-test` có nên chạy trong `check-all.sh` (thêm ~8 lần gọi)
   hay chỉ trong cổng docs?
5. C1 phần còn lại: ưu tiên `CONTEXTS` (dễ) hay `KERNEL_STORAGE` (khó) trước?

## 9. Commit

```text
$ git log --oneline 199ce90..HEAD
99d0223 docs: revise display and input design
86aea65 docs: revise pager v2 design for badge clients
fa89e89 refactor: atomic timer interval and wrapped kernel pointer
a4c2a84 test: render all harness rules and pin status to standard run
e9f73a7 chore: add docs checker self-test for stale phrases
d88627a docs: generate error and limit tables from code
14ad2fe feat: define pager v2 wire protocol
f1491ff docs: add round 24 bundle
54ee3dd docs: document round 24 consolidation and pager blocker
```

Commit `e9f73a7` gộp A1 (summary prose) + A2 (self-test); chấp nhận vì A1
không có code và cả hai cùng cổng docs. Commit docs cuối chứa báo cáo và
bundle 25.
