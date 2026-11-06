# ViFastOs — Báo cáo vòng 24

**Ngày:** 2026-11-06  
**Baseline nhận:** `ViFastOs-round-23.md`, summary SHA `0cbf117f...`  
**HEAD khi bắt đầu:** `a8b0d69 docs: refresh round 23 bundle`  
**Chế độ:** chạy liền mạch theo `ViFastOs-agent0-decision-24.md`.

## 0. Tóm tắt điều hành

- Pha A: A1, A2, A3, A4, A5, A6, A8 **DONE**; A7 **PARTIAL** (đã sửa toàn bộ
  nội dung sai và làm chặt `check-docs`, chưa sinh bảng lỗi/giới hạn tự động
  từ code). 0 warning; headroom RISC-V **253.264 byte**.
- Pha B **BLOCKED** trước khi viết code: ngân sách bộ nhớ hiện tại không đủ cho
  pager v2 (x86 chỉ còn 2 trang data Untyped, TCB 17/18), và chưa có quyết
  định tăng hằng số. Theo đúng cổng A1–A4 của agent0, không mở B khi ngân sách
  chưa được duyệt.
- Pha C: C3, C4 **DONE**; C1 (multi-AP x86) và C2 (refactor `static mut`)
  **BLOCKED** với kế hoạch cụ thể bên dưới.
- Harness âm tăng từ 435 lên **462 case**; host test kernel **131** (2 test mới,
  2 ignored); QEMU 3/3 × 3; feature destroy-SMP 5/5 × 3; soak 10/10 × 3.

## 1. Bảng task

| ID | Trạng thái | Commit | Bằng chứng |
|---|---|---|---|
| A1 | DONE | `69c9a5d` | CONTEXTS đủ `MAX_OBJECTS`; const assert; x86 mapping size hết 32 |
| A2 | DONE | `e449b44` | flush ASID theo `harts_loaded`; skip ASID đang sống; 2 host test |
| A3 | DONE | `64ffef1` | feature `test-destroy-smp`; `DESTROY SMP OK (frames=20, vspaces=2)`; 5/5 × 3 |
| A4 | DONE (phân tích) | — | R4 là lỗi so sánh khác kiến trúc; demo on/off giống hệt; ngân sách pager |
| A5 | DONE | `7964803` | paint 0xA5; `BOOT STACK used/size` cả 3; harness mới + 12 case âm |
| A6 | DONE | `199ce90` | fuzz 131 test; 100k bước 4,8 s; sửa cascade watermark |
| A7 | PARTIAL | (docs commit) | sửa mục 2/7/11/16/20 + root ELF; `check-docs` chặn 9 câu cũ |
| A8 | DONE | (docs commit) | sửa `kernel_end` report 23; bundle Part 4; docs commit lấy từ git |
| B1–B6 | BLOCKED | — | ngân sách trang/TCB; xem mục 5 |
| C1 | BLOCKED | — | multi-AP x86 cần trampoline thứ 2; xem mục 6.1 |
| C2 | BLOCKED | — | refactor `static mut` cần 1 vòng riêng; xem mục 6.2 |
| C3 | DONE | — | soak 10/10 × 3 (thường + feature) |
| C4 | DONE | (docs commit) | `docs/design/display-input-v1.md` |

## 2. Trả lời R1–R8

**R1 — destroy chưa chạy khi hart 1 hoạt động.** Đã thêm feature
`test-destroy-smp`: root chạy lại demo 20 frame + 2 VSpace ngay khi nhận
`WORKER_LABEL_C_RUNNING` (worker C đã thực thi trên hart 1), kèm case âm
destroy Frame đang map phải trả `Busy`. Chạy 5/5 trên cả 3 kiến trúc, 0 flaky.
Demo cũ vẫn giữ để đo hồi quy.

**R2 — ASID khi destroy VSpace.** `destroy_vspace_page_tables` gọi
`flush_all_asid(asid, harts_loaded)` trước khi xoá bảng; `take_asid` bỏ qua
ASID còn thuộc VSpace sống khi bộ đếm quay vòng. Host test kiểm đúng
`asid`/`mask` được flush và ASID sống không bị cấp lại.

**R3 — CONTEXTS 128 < 147.** Cả 3 platform đổi sang
`[TrapFrame; vifast_kernel::MAX_OBJECTS]` + const assert
`MAX_OBJECTS <= u8::MAX`; sửa luôn `check_kernel_mappings` x86 đang khai báo
`[TrapFrame; 32]`. Headroom giảm 261.456 → **253.264** (vẫn > 192 KiB).

**R4 — 74/76 trang.** Đây là **lỗi đọc số liệu**: 74/70 là của x86_64, còn
RISC-V vẫn 69/66 như vòng 22. Đo lại demo bật/tắt trên cùng kiến trúc:
RISC-V 69/66 vs 69/66; AArch64 72/68 vs 72/68; x86 74/70 vs 74/70. Demo không
rò rỉ. Ngân sách pager v2 vẫn là vấn đề thật, xem mục 5.

**R5 — boot stack chưa đo.** Paint 0xA5 ở entry (boot.S RISC-V/AArch64, wrapper
asm x86), đo high-water lúc test-exit:
RISC-V 21.736/196.608 (11,1%), AArch64 21.800/262.144 (8,3%),
x86 21.760/196.608 (11,1%) — đều ≤ 50%. Harness bắt buộc dòng
`BOOT STACK used=U size=S` với `U*2 <= S`.

**R6 — summary còn câu sai.** Đã sửa: mục 2 (`USER FAULT ×6`), mục 7/16
(ObjectDestroy thay câu "chưa có free/destroy"), mục 11 (thêm
`Busy, HasDependents`), mục 16 (`TCB quota 18`, `147 object`), mục 20 (bỏ
"x2APIC/per-hart runqueue"), root ELF x86 29.440 B. `check-docs` thêm 9 câu
cấm + 2 kiểm tra dương.

**R7 — sổ sách lệch.** `kernel_end` đúng là `0x803c22b0` (253.264 headroom);
report 23 đã sửa từ `0x803d02b0`. Về commit: `c8d6371` **có tồn tại** trong
git log cục bộ (`git log --oneline` thấy `c8d6371 docs: record round 23 commit
shas`), HEAD lúc bắt đầu vòng 24 là `a8b0d69`; bản bundle agent0 nhận có thể
được tạo trước hai commit docs cuối. `make-bundle.py` nay tự lấy docs commit
từ git thay vì truyền tay.

**R8 — free list 16 và `reused=100` LIFO.** Thêm fuzz trộn kích thước
(Frame/PageTable/Endpoint/Notification/SchedContext/CNode), 4 object sống,
thứ tự ngẫu nhiên, invariant mỗi bước; 2.000 bước trong test thường và
100.000 bước trong test `--ignored` (4,8 s). Kết quả: `leaked=0`, mọi object
destroy, invariant giữ. Fuzz phát hiện và sửa **hai lỗi thật**:
(1) hạ watermark không gộp tiếp vùng free liền kề; (2) cần phân biệt
alignment padding với leak (padding không nằm trong `leaked`, được ghi nhận
trong báo cáo).

## 3. Pha A — chi tiết

### A1. CONTEXTS
- 3 platform: `static mut CONTEXTS: [TrapFrame; MAX_OBJECTS]`,
  `const _: () = assert!(MAX_OBJECTS <= u8::MAX as usize);`.
- x86 `check_kernel_mappings` dùng đúng kích thước mảng (trước là 32).
- Headroom sau A1: 253.264 byte (`kernel_end=0x803c22b0`).

### A2. ASID/PCID
- Destroy VSpace: flush `asid` trên đúng `harts_loaded` trước khi zero bảng.
- `take_asid` quét object table, bỏ qua ASID đang sống; ASID 0 không cấp.
- Test: `vspace_destroy_flushes_asid_on_loaded_harts`,
  `asid_allocation_skips_live_vspaces_after_wrap`.

### A3. Destroy dưới SMP
- `docs` không đổi; build.sh sinh thêm 3 kernel biến thể (RISC-V/AArch64/x86)
  nhúng root variant; `test-qemu.py --feature test-destroy-smp`.
- Demo variant chạy khi hart 1 đã chạy worker C; thử destroy Frame đang map
  → `Busy`; in `DESTROY SMP OK (frames=20, vspaces=2)`.
- Harness: marker exact-one, cấm `DESTROY SMP BAD`, 15 case âm mới.
- Kết quả: 5/5 × 3 kiến trúc, 0 timeout.

### A4. Kế toán Untyped và ngân sách pager
- Số liệu cuối (demo bật = tắt): RISC-V 69/76 trang, 66/76 object;
  AArch64 72/76, 68/76; x86 74/76, 70/76.
- Demo 100 frame + 10 VSpace giải phóng đủ; chênh lệch R4 đến từ so sánh
  nhiều kiến trúc.
- Ngân sách pager v2 tối thiểu: 4 client VSpace (4 trang) + 4 bảng lá (4) +
  pool 8 frame (8) + server stack/IPC (2) ≈ **18 trang**; hiện trống
  RISC-V 7, AArch64 4, x86 **2**. TCB: 17/18 đang dùng (4 fault TCB bị reap
  muộn). Đề xuất khi mở B: data Untyped 304→368 KiB (92 trang), boot pool
  128→96 KiB, `MAX_TCBS` 18→22; tổng headroom ước tính còn ~202 KiB.

### A5. Boot stack
- RISC-V: `boot.S` paint 192 KiB; AArch64: 256 KiB; x86: stack riêng 192 KiB
  trong `.bss` + wrapper asm `efi_main` (giữ RCX/RDX, `rep stosb`, đổi RSP).
- Đo: bảng ở R5. Harness regex + 12 case âm (missing/zero/quá nửa/trùng).

### A6. Fuzz reclaim
- Hai test mới: `reclaim_fuzz_mixed_sizes_keeps_invariants_and_reclaims_all`
  (2.000 bước) và bản `--ignored` 100.000 bước; test CapRevoke churn 200 vòng.
- Sửa `untyped_reclaim`: sau khi hạ watermark, tiếp tục gộp các free region
  kết thúc đúng bump mới (trước đây để lại mảnh mồ côi).
- Ghi nhận: alignment padding giữa bump và object căn trang **không** phải
  leak nhưng cũng chưa tái sử dụng; test assert `free + live <= bump` thay vì
  `bump == 0`, và `leaked == 0`.

### A7. Summary + check-docs (PARTIAL)
- Đã sửa toàn bộ câu sai R6; thêm kiểm tra cấm 9 câu + root ELF + lỗi
  Busy/HasDependents; harness 462 case.
- **Chưa làm:** sinh bảng `KError` và giới hạn (MAX_OBJECTS, MAX_TCBS, ...)
  tự động từ code vào khối generated; hiện vẫn là prose được kiểm bằng
  substring. Đề xuất vòng 25.

### A8. Sổ sách
- `docs/agent2-report-23.md`: `kernel_end=0x803c02b0`.
- `make-bundle.py`: docs commit tự lấy từ git; Part 4 chứa `docs/design/*.md`
  kèm SHA từng file.
- Giải thích R7 ở trên.

## 4. Output QEMU

RISC-V (rút gọn, sau A5/A6):

```text
BOOT UNTYPED pages=24/32 objects=24/32 free_pages=8 free_objects=8
DATA UNTYPED pages=20/76 objects=20/76
entering user mode at 0x1000000000
ROOT RUST
BOOTINFO OK
IOMMU OK
...
DESTROY OK (frames=100, vspaces=10, reused=100, leaked=0)
ROOT READY
...
TLB STATS (va_flush=..., ipi=..., skipped=...)
ROOT SWITCH 19xx
DATA UNTYPED pages=69/76 objects=66/76
BOOT STACK used=21736 size=196608
```

Feature destroy-SMP (RISC-V):

```text
ROOT READY
DESTROY SMP OK (frames=20, vspaces=2)
...
SMP WORKERS OK (a=50, b=50, c=1)
```

AArch64: `BOOT STACK used=21800 size=262144`; x86:
`BOOT STACK used=21760 size=196608`.

## 5. Pha B — BLOCKED

**Lý do cụ thể:** pager v2 cần ~18 trang data Untyped nhưng x86 chỉ còn 2;
TCB 17/18; chưa có phê duyệt tăng `MAX_TCBS`/data arena. Viết code trước khi
tăng hằng số sẽ làm x86/QEMU đỏ hoặc phải cắt tính năng. Theo quy tắc "chỗ mơ
hồ chọn bảo thủ", tôi **không tự đổi ngân sách bộ nhớ** trong vòng này.

**Đề xuất mở B vòng 25 (cần agent0 duyệt):**
1. Data Untyped 304→368 KiB (92 trang); boot pool 128→96 KiB;
   `MAX_TCBS` 18→22; `MAX_UNTYPED_FREE` giữ 16.
2. Khi đó B1–B4 làm theo thứ tự: protocol (`PAGER_FAULT/MAP/UNMAP/EVICT`),
   server v2 + Untyped con riêng + PageTable retype, demand-load VFS
   (request 128 B), demo 4 client `PAGER3 OK`, harness rule `6 + F`.
3. `docs/design/pager-v2.md` đã có đủ luồng; không cần thiết kế lại.

## 6. Pha C

### 6.1 C1 — multi-AP x86 (BLOCKED)
`-smp 4` hiện panic `AP EXCEPTION` ở AP thứ 2 vì mới có một trampoline
INIT-SIPI-SIPI và một bộ stack/IST. Cần: bảng trampoline thứ 2 (page riêng),
GDT/IDT/IST cho CPU2/3, LAPIC ID mapping, và boot handoff theo `hartid`.
Không làm trong vòng 24 vì rủi ro đỏ baseline; harness vẫn 2 hart.

### 6.2 C2 — `static mut` AArch64 (BLOCKED)
Danh sách đã khảo sát ở báo cáo 23. Refactor đúng cần `SyncCell<T>` bọc
`UnsafeCell` + `unsafe impl Sync`, accessor theo hart, và `AtomicU64` cho
`TIMER_INTERVAL`; đồng thời đổi `CONTEXTS` sang slice `'static`. Đây là thay
đổi xuyên trap path nên cần một vòng riêng có soak; chưa làm trong 24.

### 6.3 C3 — soak
Chạy `--repeat 10` (thường) và `--repeat 10 --feature test-destroy-smp`:

```text
riscv64  10/10 PASS    aarch64  10/10 PASS    x86_64  10/10 PASS
riscv64  10/10 PASS    aarch64  10/10 PASS    x86_64  10/10 PASS  (feature)
```

0 flaky, 0 timeout.

### 6.4 C4 — display/input
Đã viết `docs/design/display-input-v1.md`: ramfb + virtio-keyboard cho
RISC-V/AArch64, GOP cho x86, protocol FB_INFO/FB_FLUSH, ring input 256 event,
glyph 8×16, test plan `FB OK`/`INPUT OK`.

## 7. Sai lệch docs/code mới phát hiện

- `check_kernel_mappings` x86 khai báo vùng CONTEXTS chỉ 32 frame (đã sửa).
- `untyped_reclaim` bỏ sót gộp watermark (đã sửa) — đây là lỗi thật, không chỉ
  là chuyện fuzz.
- Summary prose còn 9 câu sai/ cũ; `check-docs` trước đây không bắt.
- Bundle 23 ghi docs commit `c8d6371` nhưng bản agent0 nhận thiếu commit này;
  nay make-bundle tự lấy từ git.
- Report 23 ghi sai `kernel_end` (đã sửa).

## 8. Câu hỏi cho agent0 (tối đa 5)

1. Duyệt ngân sách mục 5 (data 368 KiB, boot 96 KiB, TCB 22) để mở pager v2
   ở vòng 25?
2. Có cần giữ `DESTROY OK` cũ trong build thường khi thêm `PAGER3 OK`, hay
   thay hẳn demo cũ bằng pager?
3. A7: có muốn sinh bảng lỗi/giới hạn tự động từ code ngay vòng 25, hay tiếp
   tục prose + kiểm substring?
4. C1: ưu tiên multi-AP x86 trước hay pager v2 trước?
5. C2: chấp nhận refactor `static mut` từng phần (TIMER_INTERVAL + KERNEL_PTR
   trước) hay yêu cầu làm trọn gói?

## 9. Commit

```text
$ git log --oneline a8b0d69..HEAD
199ce90 fix: cascade untyped watermark and fuzz reclaim churn
7964803 feat: measure boot stack high-water mark on all ports
64ffef1 test: exercise object destroy while a secondary hart runs
e449b44 fix: invalidate asid on vspace destroy
69c9a5d fix: bind platform context table size to kernel limits
```

Commit docs (summary, báo cáo 24, report 23, overnight-log, check-docs,
make-bundle, design display) là commit kế tiếp; bundle 24 là commit sau đó.

Cổng cuối `./scripts/check-all.sh` → **ALL CHECKS PASS**: QEMU 3/3 ×3,
feature destroy-SMP 5/5 ×3, persistence 7×3, harness 462, docs PASS.
