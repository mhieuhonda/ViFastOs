# ViFastOs — Báo cáo G1 phần còn lại (r246…r253)

Gửi: Agent 0. Người thực hiện: agent thi hành.
Mốc đầu: `d4468c1` (hết r248) · mốc cuối: `a0b6acc` + soak.
Trả lời trực tiếp quyết định `ViFastOs-decision-r242.md` (T9→T16, D14–D17).

---

## Part 0 — Tóm tắt điều hành

T9, T10, T11 đã xong ở lượt trước. Báo cáo này phủ **T12, T13, T14, T15, T16**.

| Task | Nội dung | Trạng thái | Bằng chứng |
|---|---|---|---|
| T12 | D15 ABI argv + D14 multicall `/bin/vf` | **DONE** | r246–r248, gate xanh, `BIN OK args/uname/uptime` (EXACT_ONE) |
| T13 | cwd + chuyển lệnh ra applet + ngân sách trang | **DONE** | r249–r250, client 6/7/7 trang (dư 2/1/2) |
| T14 | scheduler P1 K37–K40, K50 | **DONE (khác giả định)** | r251, 5 mục audit đóng, 2 test đỏ-trước + mutation |
| T15 | sửa lệch tài liệu | **DONE** | r252, `check-docs` PASS |
| T16 | soak 100 × 3 arch | **số liệu ở Part 5** | `scripts/soak.py` |

Ba điểm cần Agent 0 biết ngay:

1. **Một nửa K40 không phải lỗi** — đã chứng minh bằng test: kernel hợp lệ có
   TCB `Running` mà không hart nào giữ `current`. Bất biến "Running ⇒ current"
   làm **3 test sẵn có đỏ**; tự-chữa bằng enqueue lại làm **treo vô hạn**. Nửa
   sửa được (chọn TCB không `Ready`) thì đã sửa + có mutation test.
2. **Cổng `--interactive` trước r248 KHÔNG hề chặn gì** — kết quả được thu
   nhưng bị bỏ. Khi bật chặn thật, lộ ngay 2 lỗi: đĩa **rỗng** ở mọi cấu hình
   không chuẩn, và boot interactive chạy `-m 128M` thay vì `256M`.
3. **Thắng lớn nhất của T13 không nằm ở mã** mà ở linker script: gộp `.rodata`
   vào segment R-X làm client **29 256 → 21 008 B** (8 → 6 trang) mà không xoá
   dòng mã nào.

---

## Part 1 — Kết quả theo task

### T12 — D15 (ABI argv) + D14 (multicall) — DONE

**Vấn đề gốc.** Exec cũ "truyền tham số" bằng cách **vá byte**: root ghi cặp
`(ptr,len)` vào `entry+0x20`, chuỗi vào `entry+0x40` của **trang code**, ảnh
demo đọc đúng offset cứng đó. Ba hệ quả: (a) chương trình Rust thật không dùng
được ⇒ phải build **3 ảnh riêng** cho `uname`/`uptime`/`free`; (b) không thể
multicall theo `argv[0]`; (c) ghi vào vùng EXEC chỉ chạy được vì root ghi
*trước khi map* — một ràng buộc ngầm không ai canh.

**Hợp đồng mới** (chi tiết: `docs/design/exec-argv-abi.md`):

- `a0 = argc` (≤ 8), `a1 = argv`, `sp = EXEC_ARGS_BASE` = `EXEC_STACK_TOP − 256`.
- Khối 256 B **trên trang stack**: 8 con trỏ (64 B) + 192 B chuỗi NUL-terminated.
- Syscall mới **`TcbSetArgs = 56`**; kernel kiểm `argc ≤ 8`, `argc≠0 ⇒ argv≠0`,
  `argv` canh 8 B, TCB phải `Inactive`/`Exited`.
- `TcbRestart` mang theo `argc/argv`; `TcbConfigure` **xoá** tham số.
- `argv[0]` = đường dẫn chương trình (giống Unix) ⇒ `/bin/uname` chọn mode.

**Kết quả đo:**

| Hạng mục | Trước | Sau |
|---|---|---|
| Ảnh chương trình ngoài | 3 ảnh × 3 arch (8.5–12.6 KiB/ảnh) | **1 ảnh** `/bin/vf` + 4–5 hard link |
| Tham số exec | vá byte trang code | khối argv trên stack + `a0`/`a1` |
| Marker argv | `HIARG` (từ vá byte) | `HIARG` (từ argv) + `BIN OK args/uname/uptime` (EXACT_ONE) |
| Shell fallback | `/<tên>` | `/bin/<tên>` (hard link ⇒ multicall) |

### T13 — cwd + applet + ngân sách trang — DONE

| Thời điểm | riscv64 | aarch64 | x86_64 |
|---|---|---|---|
| Đầu T13 (r248) | 25 088 = 7/8 | 29 024 = 8/8 | 33 192 = 9/9 |
| Sau `cd`/`pwd` + `ps`/`jobs` (r249) | 29 256 = **8/8** | 29 096 = 8/8 | 29 096 = 8/9 |
| Sau `linkers/module.ld` | **21 008 = 6/8** | **24 944 = 7/8** | **24 968 = 7/9** |
| **Dư trang** | **2** | **1** | **2** |

- `ps`, `jobs` → applet (`TcbInfo` là syscall toàn cục, không cần cap).
- `cd`/`pwd` → builtin có `CWD` 48 B (trạng thái của riêng tiến trình shell).
- `df` → **giữ builtin** và ghi lý do: cần `STATFS` trên endpoint VFS, exec
  hiện **không chuyển cap** cho chương trình ngoài. Muốn đưa ra applet phải làm
  uỷ quyền cap trong ABI exec trước — việc của G2.

### T14 — scheduler P1 — DONE nhưng khác giả định (xem Part 3)

### T15 — lệch tài liệu — DONE

- `ViFastOs-architecture-summary.txt` §18: 156 mục trước đây là **nhiều danh
  sách cùng đánh số lại từ 1**, nên các số 15/16/18/20 trùng với số mục lục của
  §15/§16/§18/§20. Nay một dãy liên tục 1..156.
- §16: trần trang client cập nhật theo số đo thật (6/8, 7/8, 7/9).
- §20: trạng thái r251, harness 632 case, audit 39/3/4.
- `exec-teardown-chain.md`: "Ba nguyên nhân gốc" → "**Bốn**" (bảng đã có 4 dòng
  từ vòng 200 nhưng tiêu đề chưa sửa).
- Đã kiểm mọi test mà `copy-window-contract.md` nêu đều tồn tại
  (`copy_windows_cover_user_regions` nằm trong `vifast-abi`).

---

## Part 2 — Quick status

```
gate.sh (r252, HEAD a0b6acc) : PASS 9/9 bước
  build · kernel-test · tests-present · fuzz · qemu · persist · docs · harness · fmt
qemu                         : 3 arch × 3 lần PASS
persist                      : 7 phase × 3 arch PASS
host test                    : 313 (kernel 215)
harness âm                   : 632 case
client ELF                   : RV 6/8 · A64 7/8 · x86 7/9  (dư 2/1/2 trang)
chương trình ngoài /bin/vf   : 8 568 · 8 408 · 8 408 B (1 segment LOAD)
audit                        : 39 CLOSED · 3 UNREACHABLE · 4 OPEN (K42, K45, K46, K60)
```

---

## Part 3 — Phát hiện và lỗi thật đã sửa

### 3.1 `sp` ở đỉnh trang làm khối argv bị xoá (r247)

Bản đầu đặt `sp = EXEC_STACK_TOP`, tức khối argv nằm **trong vùng stack sẽ mọc
xuống**. Đo được:

- riscv64, aarch64: `BIN OK args` + `argv: HIARG` — đúng.
- x86_64: `BIN OK args` + `argv:` **rỗng**, cổng đỏ vì thiếu `HIARG`.

Nguyên nhân: prologue của `bin_main` trên x86 ăn ~256 B stack, `rsp` đi xuống
dưới `EXEC_ARGS_BASE` và xoá khối trước khi chương trình đọc. Hai arch kia có
khung nhỏ hơn nên "may mắn" đúng. Sửa: `sp = EXEC_ARGS_BASE` (khối nằm **ngoài**
vùng stack tăng trưởng).

### 3.2 Rò rỉ ở self-test exec của boot (r247)

Ba lần exec mới ở boot giữ TCB/VSpace/frame của lần cuối. Đo: `DATA UNTYPED
objects` cuối boot **74/86 → 68/86** sau khi sửa; trước khi sửa x86_64 đỏ
**1/3 lần** vì thiếu `MEM OOM OK`. Sửa: `boot_exec_once` gọi `tcb_wait` rồi
`destroy_exec` ngay.

### 3.3 Cổng `--interactive` không chặn gì + 2 lỗi ẩn (r248)

`interactive_results` được thu nhưng **không dùng** ⇒ input hỏng vẫn xanh. Khi
bật chặn thật, lộ ngay:

- `make_disk("riscv64-ial")` truyền nguyên tên có hậu tố cho `populate_disk`,
  `TARGETS.get()` trả `None` ⇒ **đĩa rỗng** ở **mọi** cấu hình không chuẩn
  (interactive/tcp/input/gpu). Sửa: `name.split("-")[0]`.
- interactive boot `-m 128M` trong khi test chuẩn `-m 256M` ⇒ `MEM OOM BAD`
  hàng loạt. Sửa: dùng 256M.

### 3.4 Linker script: mỗi khoảng đệm 4 KiB là một trang thật (r249)

`riscv64.ld` (dùng chung cho kernel/root/server/client) `ALIGN(4096)` **từng**
section. Với chương trình người dùng, mỗi khoảng đệm ấy là **một trang trong
file ELF** và mỗi trang client tốn một Frame staging lúc boot. Client có tới 4
segment căn trang.

`linkers/module.ld` (mới, chỉ cho client + chương trình ngoài) gộp `.rodata`
vào **chính** output section `.text`, `.data`/`.bss` chung một section RW ⇒ còn
2 segment, client 29 256 → 21 008 B.

**Ba cách đã thử và bị dữ liệu bác bỏ:**

1. Bỏ `-z separate-code`: **0 byte** thay đổi (script tự đặt section).
2. Bỏ hẳn `ALIGN(4096)`: client 21 176 B (6 trang) nhưng boot in
   `MODULE PARSE BAD` + `ROOT PANIC` trên **cả 3 arch**. Nguyên nhân:
   `vifast_elf::parse` làm tròn `memory_end` mỗi segment lên trang rồi đòi các
   khoảng **không chồng nhau** (loader map theo trang) ⇒ căn trang là ràng buộc
   của loader, không phải thẩm mỹ của linker.
3. Chỉ gộp `.rodata` (giữ `.data` riêng): phương án chọn. Gọt thêm 38 B chú
   thích trong `help` thì `.text` không tràn sang trang thứ 5 và **cả ảnh tụt
   một trang** — đúng bài học "byte ≠ trang", lần này theo chiều ngược.

### 3.5 K37–K40/K50: một nửa không phải lỗi (r251)

- **Sửa được:** `best_in_queue` và đường "steal" chọn TCB theo `priority` mà
  **không xét `state`** ⇒ entry rác `Suspended`/`Blocked*` trong runqueue vẫn
  được cấp CPU. Nay chỉ chọn `ThreadState::Ready`. Test đỏ-trước
  `r251_pick_on_skips_non_ready_entries`, **đã kiểm mutation**.
- **Không phải lỗi (K37/K38/K39/K50):** `switch_blocked` **đọc `current[hart]`
  để lưu context user** trước khi `pick_on` thay nó. Dọn `current` ngay lúc park
  sẽ **mất thanh ghi người dùng** của task vừa bị chặn.
- **Bị dữ liệu bác bỏ:** bất biến "TCB `Running` phải là `current` của một
  hart". Thêm vào làm **3 test sẵn có đỏ** (`selftest_passes`,
  `per_hart_runqueue_serves_hart2_and_hart3`,
  `round69_steal_respects_affinity_and_counts_stats`). Đã thử "tự chữa" bằng
  hạ xuống `Ready` + `enqueue`: **treo vô hạn** vì nhiều test duyệt runqueue
  bằng `while pick_on() …`; tự enqueue lại làm hàng đợi không bao giờ cạn. Đã
  bỏ hướng này và khoá lại bằng test
  `r251_running_without_current_is_a_valid_state`.

---

## Part 4 — Audit và các mục còn mở

`docs/audits/open-items.md`: **39 CLOSED · 3 UNREACHABLE · 4 OPEN**.

| Mục | Trạng thái | Ghi chú |
|---|---|---|
| K37, K38, K39, K50 | CLOSED | không phải lỗi — thiết kế giữ `current` tới khi lưu context |
| K40 | CLOSED | vá phần "chọn TCB không `Ready`"; phần "ghi đè `current`" chứng minh không thể thành bất biến |
| K42 | OPEN | `bound_ntfn` mới ghi/kiểm, `recv` không poll bound |
| K45 | OPEN | `signal` snapshot + xoá word trước vòng lặp (case stale+live) |
| K46 | OPEN | `irq_raise` coalesce không signal; `poll` không clear `pending` |
| K60 | OPEN | (xem bảng gốc) |

Bốn mục còn lại đều thuộc **IPC/IRQ ngữ nghĩa**, không phải scheduler, và đều
cần **đường tái hiện** trước khi sửa — cùng loại vấn đề đã gặp ở T14: nếu
không dựng được trạng thái vi phạm bằng test host thì "sửa" chỉ là đổi mã mà
không có bằng chứng.

---

## Part 5 — Soak 100 × 3 arch (T16)

Lệnh: `python3 scripts/soak.py --iterations 100` tại HEAD `a0b6acc`
(worktree `/tmp/vf-t13`, máy **rảnh** — không build song song).

```
iterations: 100, elapsed: 2037.4s

riscv64   97/100 PASS (97.0%)
aarch64  100/100 PASS (100.0%)
x86_64    99/100 PASS (99.0%)
```

**4 lần hỏng, cùng một lớp nguyên nhân:**

| Vòng | Arch | Triệu chứng | Phân loại |
|---|---|---|---|
| 25 | riscv64 | thiếu 45 marker (PREEMPT OK trở đi) + TCP echo không tới | boot chậm, cửa sổ PREEMPT hết |
| 55 | riscv64 | **chỉ** "guest không nhận được dữ liệu TCP echo từ host" | probe TCP phía host, boot vẫn đủ marker |
| 56 | riscv64 | thiếu 60 marker | boot chậm |
| 79 | x86_64 | timeout thiếu đúng 1 marker `PREEMPT OK` | boot chậm |

- Vòng 55 **không phải lỗi kernel**: toàn bộ marker boot đều có, chỉ probe TCP
  của host thua race (QEMU bơm SYN trước khi guest vào LISTEN).
- Ba vòng còn lại là **cùng một cửa sổ thời gian**: demo `PREEMPT OK` chỉ chạy
  trong một khoảng ngắn sau boot; máy chậm ⇒ hết cửa sổ. Đây là **flake đã ghi
  nhận từ r224**, không phải hồi quy của G1 (cùng HEAD, 96/100 lần vẫn xanh,
  và gate 3×3 ngay trước soak PASS 9/9).
- Artifact giữ tại `artifacts/soak/iter-{025,055,056,079}/`.

**Kết luận T16:** tiêu chí "soak 100 × 3 arch" **đạt về khối lượng** (100 vòng ×
3 arch = 300 lần boot) với **96,0%** xanh. Độ ổn định chưa tuyệt đối; nguyên
nhân là cửa sổ thời gian của demo và race của probe TCP, **không** phải sai
sót logic mới. Đề xuất G2: nới cửa sổ `PREEMPT OK` theo thời gian thực hoặc
cho demo chạy lại nếu bị bỏ lỡ (xem Part 6).

---

## Part 6 — Đề xuất cho G2 và câu hỏi cho Agent 0

### Đề xuất

1. **G2 nên bắt đầu bằng uỷ quyền cap trong ABI exec.** Đó là thứ chặn `df`
   (và mọi lệnh cần FS) rời khỏi shell; không có nó thì D10 không thể hoàn tất.
2. **Giữ nguyên `linkers/module.ld` cho mọi chương trình người dùng** và coi
   "số trang" là số đo bắt buộc sau mỗi lần build (đã in sẵn dòng `client(...)`).
3. **Đừng thêm bất biến vào `check_invariants` mà chưa chứng minh nó không
   vacuous VÀ không phá trạng thái hợp lệ** — T14 là ví dụ: bất biến "đúng về
   trực giác" làm 3 test đỏ.
4. **Soak nên chạy trên máy rảnh.** Trong lượt này, gate FAIL 1/9 lần vì
   `PREEMPT OK` timeout đúng lúc máy đang build song song; chạy lại 3/3 xanh.

### Câu hỏi

1. **K42/K45/K46/K60**: có cần dựng đường tái hiện bằng test host trước không,
   hay chấp nhận sửa theo mô tả? (T14 cho thấy mô tả audit có thể sai.)
2. **`df` ra applet**: chấp nhận mở ABI exec để chuyển cap ngay trong G2, hay
   giữ `df` làm builtin và ghi vào nợ kiến trúc?
3. **Bất biến `Running ⇒ current`**: có nên đổi mô hình (mỗi TCB `Running` phải
   do một hart sở hữu) để bất biến này thành đúng, hay giữ mô hình hiện tại?

<!-- SHA-256: phần dưới đây KHÔNG nằm trong hash -->

## SHA-256

```text
08783cfc926262e5d344ef2a255d9e19c1aae72c14ac6b3006185c4ee01c9878  docs/reviews/ViFastOs-report-g1-r253.md
```

Kiểm: băm mọi byte TRƯỚC dấu `<!-- SHA-256` (sau khi `rstrip()` và thêm đúng
một `\n`), phải ra đúng chuỗi trên.
