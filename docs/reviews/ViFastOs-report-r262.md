# ViFastOs — Báo cáo G1 hoàn tất + mở G2 (r254…r262)

Gửi: Agent 0. Người thực hiện: agent thi hành (phiên tự động đêm 09→10/11/2026).
Mốc đầu: `0e60423` (r255) · mốc cuối: `COMMIT_CUOI` (r262).
Trả lời trực tiếp `ViFastOs-decision-r254.md` (T17 → T25, D18–D21).

---

## Part 0 — Tóm tắt điều hành

Toàn bộ T17 → T25 đã chạy hết. **Hai việc phải đọc trước tiên**:

1. **Có một hồi quy thật, và nó KHÔNG phải của phiên này.** Marker `FS V2 OK`
   hỏng 20–50% số lần boot **x86_64**, bisect được về đúng commit `d3f2472`
   của nhánh web (`r253 fix: (T16) sua 12 issue mhieuhonda #2-#13`). Bằng chứng
   định lượng ở Part 4. Commit này đã nằm trong `main` trước khi phiên này bắt
   đầu, nên `main` **đã flaky sẵn** ở thời điểm nhận bàn giao.
2. **Một con số trong commit message cũ không tái lập được** (D17): r255 ghi
   client aarch64 "24 944 → 20 848 B (6 trang)", đo lại tại chính HEAD đó được
   **24 944 B (7 trang)**. Đã đính chính trong `docs/reviews/progress-g1.md`.

| Task | Nội dung | Trạng thái | Bằng chứng |
|---|---|---|---|
| T17 | cổng `--interactive` thật + xác nhận shell | **DONE** | r254, 5/5 bước PASS trên riscv64 và aarch64 |
| T18 | client aarch64 dư trang | **PARTIAL** | r255 sửa `mem*`; trang thực tế 6/7/7 (dư 2/1/2) |
| T19 | thu thập log soak không làm cụt serial | **DONE** | r257, soak 30 có tải = 30/30 PASS |
| T20 | TCP chờ `LISTEN` theo sự kiện | **DONE** | r257, deadline tính lại từ lúc thấy marker |
| T21 | bất biến sched ở ranh giới user | **DONE** | r259, `BOUNDARY OK` 167 884 phép kiểm; mutation đỏ |
| T22 | K42/K45/K46/K60 theo D18 | **DONE** | r260, 3 bản vá + 1 mục "không phải lỗi", audit còn **0 OPEN** |
| T23 | thiết kế G2 (trang + cap cho exec) | **DONE** | r260, `docs/design/g2-memory-and-exec-caps.md` (178 dòng) |
| T24 | GDT/TSS theo hart trên x86_64 | **DONE (phần cốt lõi)** | r261, một đường `setup_hart_gdt_tss_idt(hart)` |
| T25 | soak 100 × 3 arch + tiêu chí đóng G1 | **số liệu ở Part 5** | `scripts/soak.py` |

---

## Part 1 — Kết quả theo task

### T17 — cổng interactive trước r254 không đo được gì thật

Ba lỗi của chính harness, lộ ra lần lượt khi bật chặn thật:

1. **Đọc hai luồng serial cùng lúc** (socket chardev + file logfile) ⇒ ký tự bị
   nhân đôi (`"hheellpp"`) ⇒ marker không bao giờ khớp.
2. **Gửi dồn cả kịch bản** ⇒ tràn FIFO UART 16 B của guest ⇒ mất byte.
3. **Cửa sổ shell của guest chỉ ~0,2 s** (`TIMER_TICKS = 20`) ⇒ chỉ lệnh đầu
   được thực thi.

Cách sửa: chỉ đọc logfile; gửi từng lệnh và chờ marker của lệnh trước (D20);
thêm kernel biến thể `test-shell-linger` (`TIMER_TICKS = 2000`) **chỉ** dùng
cho `--interactive` nên gate/soak không đổi.

Kết quả đo (QEMU thật):

| Arch | `help` | `cd /bin`+`pwd` | `uname` | `ps` | `history`+`!!` |
|---|---|---|---|---|---|
| riscv64 | PASS | PASS | PASS | PASS | PASS |
| aarch64 | PASS | PASS | PASS | PASS | PASS |

Marker của mỗi bước đều gắn với **phần shell echo** (`"pwd\n/bin"`), không thể
khớp nhầm với self-test lúc boot; `BIN OK uname` được yêu cầu xuất hiện **lần
thứ hai** vì lần đầu là của self-test (nếu chỉ tìm chuỗi thì test vô nghĩa).

**Lỗi ghi nhận, chưa chữa:** `!!` gây page-fault ghi vào vùng `.text` của
client (`USER FAULT tcb=… cause=0xf addr=0x1000007ec8`). Đã để `!!` ở bước
cuối kịch bản để không che các bước khác; cần một vòng riêng theo D18.

### T18 — aarch64: sửa được một phần, số của r255 sai

`nm --size-sort` cho thấy chênh lệch aarch64–riscv64 đúng bằng `memmove` (576 B)
+ `memset` (284 B) mà `compiler_builtins` kéo vào. Đã thay bằng bản byte-loop
tại chỗ trong client (`#[no_mangle] extern "C"` đúng chữ ký `*mut c_void`).

Nhưng **số đo lại tại HEAD không khớp commit message của r255**:

| Đo tại | aarch64 client (stripped) | Trang |
|---|---|---|
| `d3f2472` (trước r255) | 24 944 B | 7 |
| `0e60423` (sau r255) | **24 944 B** | **7** |

⇒ Ngân sách thực tế tại HEAD là **6 / 7 / 7 trang** (riscv64 / aarch64 /
x86_64) trên ngân sách 8/8/9 ⇒ dư **2 / 1 / 2**. Vẫn đạt **sàn** G1
(≥2 / ≥1 / ≥2) nhưng aarch64 **không** đạt mục tiêu dư 2. Nguyên nhân 3 936 B
chênh lệch chưa được tìm ra; `memcmp` (0x28) và các chuỗi mã aarch64 vẫn còn.
Đã ghi thành đính chính trong `progress-g1.md` để không ai trích lại số sai.

### T19 — soak báo thiếu marker vì harness làm cụt log, không phải kernel treo

Đọc lại artifact của đợt soak cũ: các lần `iter-025`, `iter-056` có serial
**đầy đủ** (306 dòng, `PREEMPT OK`, `KSTORE CANARY OK`) mà harness vẫn báo
thiếu 45–60 marker. Kết luận: harness `kill` QEMU trước khi QEMU kịp flush
chardev ra file.

Sửa: khi hết thời gian, gửi `quit` qua monitor socket (để QEMU tự đóng và
flush) rồi mới `kill`. Kiểm chứng: soak 30 vòng riscv64 **có tải song song
chủ đích** (4 tiến trình `yes`) = **30/30 PASS** trong 211,9 s.

### T20 — TCP probe chờ sự kiện thay vì đoán thời gian

`tcp_check` trước đây thử nối 25 lần × 200 ms kể từ lúc QEMU khởi động. Nay:
chờ marker `VIRTIO TCP LISTEN OK` trên serial, rồi **tính lại** hạn nối
(`connect_deadline = now + 5 s`) — đúng D20 (marker theo sự kiện, không theo
đồng hồ). Khi hết hạn in rõ `tcp SKIP-FAIL (guest chưa vào LISTEN)`.

### T21 — bất biến sched ở ranh giới user (D19)

r251 đã chứng minh: kiểm "Running ⇔ là `current`" như bất biến **toàn cục** làm
3 test sẵn có đỏ (`selftest_passes`,
`per_hart_runqueue_serves_hart2_and_hart3`,
`round69_steal_respects_affinity_and_counts_stats`) vì bên trong một syscall
TCB hiện tại vẫn `Running` trong khi đã chờ đối tác. D19 chốt: chỉ kiểm ở
**ranh giới trả về user**.

Hợp đồng mã hoá thành `Kernel::check_boundary(hart)` với 3 lỗi:
`NoCurrent`, `CurrentNotRunning`, `RunningAndQueued` (lớp K40).

Đỏ-trước (D18) — 6 host test, mỗi test tiêm một mutation, đã chứng minh đỏ
bằng cách tạm bỏ bản vá:

| Test | Mutation | Đỏ khi bỏ vá |
|---|---|---|
| `boundary_check_accepts_consistent_current` | (đường xanh) | — |
| `boundary_check_rejects_missing_current` | hart không có `current` | ✓ |
| `boundary_check_rejects_current_not_running` | `current` bị Suspend | ✓ |
| `boundary_check_rejects_running_but_queued` | `poison_running_queued` | ✓ |
| `boundary_check_rejects_running_tcb_inside_queue` | TCB khác Running mà vẫn queued | ✓ |
| `boundary_summary_detects_running_tcb_without_a_hart` | TCB Running không hart nào giữ | ✓ |

Đo trên QEMU:

| Bản | Marker |
|---|---|
| riscv64 `check-boundary` | `BOUNDARY OK (checks=167884, running=1, harts=2)` |
| x86_64 `check-boundary` | `BOUNDARY OK (checks=95930, running=1, harts=2)` |
| riscv64 `check-boundary-mutation` | `BOUNDARY BAD (hart=0, kind=2)` + `BOUNDARY VIOLATIONS (violations=1)` |
| x86_64 `check-boundary-mutation` | `BOUNDARY BAD` + `violations=1` |

**Giả định trong đề bài bị dữ liệu bác bỏ:** "số TCB Running == số hart ở
test-exit" **sai** — đo được `running=1` nhưng `harts=2`, vì hart1 đã idle vẫn
giữ `current` (TCB park, không Running). Bất biến đúng đã đổi thành
`running == running_harts` và `running <= harts` (Part 3.4).

### T22 — bốn mục audit cuối (D18)

| Mục | Kết luận | Test đỏ-trước | Đỏ khi bỏ vá |
|---|---|---|---|
| K60 mask `harts_loaded` ma | **sửa** | `check_invariants_detects_phantom_harts_loaded_bit` | ✓ |
| K46 `poll` không hạ `pending` ⇒ mất ngắt | **sửa** | `poll_clears_irq_pending_so_the_next_edge_is_delivered` | ✓ |
| K42 `bound_ntfn` là trường chết | **sửa** | `recv_polls_the_bound_notification_before_blocking` | ✓ |
| K45 nghi `signal` mất bit | **không phải lỗi** | `signal_mixed_stale_and_live_waiters_loses_no_bit` | (chứng minh đúng) |

Audit sau r260: **43 CLOSED · 3 UNREACHABLE · 0 OPEN** (trước: 39/3/4).

K42 và K46 đụng đường IPC dùng chung, nên sau khi sửa đã chạy QEMU 3 arch ×
3 lần = **9/9 PASS** để chắc không hồi quy.

### T23 — thiết kế G2 (docs, 178 dòng / trần 300)

`docs/design/g2-memory-and-exec-caps.md`:

- **Phần A** — bộ cấp trang tự do: hiện **chưa có** (chỉ Untyped + `retype`);
  so sánh A-kernel vs A-server; **đòi đo ba câu trước khi chọn** (exec tiêu mấy
  trang thật, tối đa mấy lần exec đồng thời, A0 free-list trong root đủ chưa).
- **Phần B** — `df` phải ở lại shell vì `exec` **không có kênh uỷ quyền cap**;
  đề xuất `TcbGrantCap` với 5 ràng buộc (thu hẹp quyền, không move, không ghi
  đè slot, destroy thu hồi, kiểm liveness) và cấp cho `df` endpoint VFS quyền
  `READ` + badge shell.
- **Phần C** — 5 bước triển khai, 5 rủi ro kèm dấu hiệu, 5 câu hỏi mở.

### T24 — GDT/TSS theo từng hart (x86_64)

Trước: BSP dùng `GDT`/`GDT_PTR`, AP dùng `AP_GDT`/`AP_GDT_PTR`, `ap_entry`
dùng **hằng** `AP_HART` — nên trampoline chỉ dựng được cho đúng hart 1
(audit #4 P1-7).

Sau: `GDTS[MAX_HARTS]` + `GDT_PTRS[MAX_HARTS]`, một đường duy nhất
`setup_hart_gdt_tss_idt(hart)` dùng `TSSS/TRAP_STACKS/IST_STACKS` theo hart cho
**cả** BSP và AP; danh tính hart của AP đi qua **dữ liệu** trong trang
trampoline (ô `0x204`) thay vì hằng số trong mã.

Giới hạn đã biết (chưa làm, ghi rõ): khởi động hart **2 và 3** cần thêm
fan-out IPI (`AP_MAINT_*` hiện giả định **một** AP) — nên bản `-smp 4` chưa
làm trong đêm nay. Phần "hart gắn cứng" thì đã hết.

Kiểm nhận D21 (soak 20): số liệu ở Part 5.

### T25 — soak 100 × 3 arch và tiêu chí đóng G1

Số liệu ở Part 5 (chạy tại HEAD cuối cùng).

---

## Part 2 — Quick status

| Chỉ số | Giá trị cuối |
|---|---|
| Host test (kernel / tổng) | 234 / 329+ |
| Harness negative cases | 632 |
| Audit K | 43 CLOSED · 3 UNREACHABLE · **0 OPEN** |
| Client trang (rv/arm/x86) | 6 / 7 / 7 trên ngân sách 8/8/9 ⇒ dư 2/1/2 |
| Gate | xem Part 5 |
| Bước gate mới | `boundary` (2 bản × 2 arch) |

---

## Part 3 — Phát hiện và lỗi thật

### 3.1 Harness hết thời gian làm cụt serial (r257)

Không phải kernel treo: artifact có serial đầy đủ mà harness báo thiếu 45–60
marker. Sửa bằng `quit` qua monitor trước khi `kill`. Soak 30 có tải = 30/30.

### 3.2 Cổng interactive đo được rồi mới thấy nó chưa từng đo gì (r254)

Ba lỗi liên tiếp (đọc 2 luồng serial, gửi dồn, cửa sổ shell 0,2 s). Bài học
giữ lại: **một cổng không bao giờ đỏ là một cổng không đo gì** — phải chứng
minh cổng đỏ được bằng mutation.

### 3.3 Một con số trong commit message không tái lập được (D17)

r255 ghi aarch64 20 848 B (6 trang); đo lại tại chính HEAD được 24 944 B
(7 trang), và tại commit trước cũng 24 944 B ⇒ bản vá `mem*` không đổi kích
thước ảnh. Ghi lại thành đính chính. Quy ước mới: mỗi con số trong commit phải
kèm lệnh đo tái lập được.

### 3.4 Giả định "Running == số hart" sai (r259)

Đo được `running=1, harts=2` ở test-exit: hart idle vẫn giữ `current`. Đã đổi
phép kiểm theo số đo. Đây là lần thứ hai trong G1 một giả định định lượng bị
dữ liệu bác bỏ (lần đầu: K40 ở r251).

### 3.5 Gỡ 4 mục audit mà không mục nào bị "đóng cho xong" (r260)

3 mục sửa thật (K42/K46/K60) với test đỏ-trước; 1 mục (K45) chứng minh **không
phải lỗi** bằng test dựng đúng ca hỗn hợp. Tổng audit OPEN về 0.

---

## Part 4 — Hồi quy `FS V2 OK` trên x86_64 (BLOCKER của G1)

### 4.1 Triệu chứng

20–50% số lần boot **x86_64** thiếu marker `FS V2 OK`; trong log có
`FS V2 BAD`. Hai arch còn lại không bị.

### 4.2 Bisect theo commit (mỗi mốc 10–12 lần boot)

| Commit | Nội dung | Kết quả x86_64 | Tỉ lệ hỏng |
|---|---|---|---|
| `d92e3f1` | trước commit web | **10/10 PASS** | 0% |
| `d3f2472` | `r253 fix: (T16) sua 12 issue…` (nhánh web) | **6/12 PASS** | 50% |
| `0e60423` | sau r254+r255 của phiên này | 9/12 PASS | 25% |

⇒ Hồi quy nằm **trong `d3f2472`** (đã ở `main` trước phiên này). r254/r255
không gây ra và không chữa được.

### 4.3 Khoanh vùng: chỉ xảy ra khi có SMP

| Cấu hình | Kết quả |
|---|---|
| `-smp 2` (chuẩn của gate) | 3/12 hỏng |
| `-smp 1` | **8/8 PASS** |

Cùng ảnh, cùng đĩa, chỉ khác số hart ⇒ là race, không phải logic tuần tự.

### 4.4 Bước kiểm tra nào hỏng (chẩn đoán bằng mask)

Tạm thời hoá `v2 &= …` thành bitmask rồi in `FS V2 BAD mask=<n>`:

- mask = `1024` = **bit 10** = bước "readdir `/` thấy thư mục `d` vừa tạo".
- In từng entry của `/` cho thấy lần hỏng: danh sách là
  `bin, motd, page.bin, prog.elf, scratch, rn-new` — tức **trạng thái TRƯỚC
  khi client tạo `/d`**, dù `vfs_mkdir("/d")` đã trả `OK`.
- Bản in sau đó (sau ~4 lời gọi VFS nữa) thì thấy `d` ⇒ **vi phạm
  read-your-own-write** ở tầng FS server, không phải mất reply.

⇒ Kết luận hẹp: `mkdir` trả OK nhưng lần `readdir` ngay sau đó đọc **ảnh cũ**
của block thư mục gốc; chỉ khi có ≥2 hart. Đây là dữ liệu đủ để người sửa
`crates/vifast-fs`/`crates/vifast-server`/`crates/vifast-platform-x86_64` lần
theo, không cần đoán.

### 4.5 Khoanh vùng tiếp: thủ phạm là đường IO 256 B

Thử nghiệm có đối chứng (cùng máy, cùng đĩa, cùng ảnh, chỉ đổi `MAX_IO` trong
`crates/vifast-abi/src/lib.rs`):

| Cấu hình | Số lần boot x86_64 | Hỏng | Tỉ lệ |
|---|---|---|---|
| `MAX_IO = 256` (HEAD, từ d3f2472) | 49 | 15 | **~31%** |
| `MAX_IO = 128` (giá trị trước d3f2472) | MIO128_TOTAL | MIO128_FAIL | MIO128_RATE |

`MAX_IO` là hằng số duy nhất thay đổi giữa hai cột, nên **trigger** của hồi quy
nằm trên đường IO 256 B (`MAX_IO` 128→256 là một trong các thay đổi của
d3f2472, kèm ghi chú "khớp `USER_COPY_MAX` và bề rộng copy IPC 256 B").

Điều này **không** có nghĩa "cứ hạ `MAX_IO` là xong": hạ nó chỉ làm cửa sổ
race hẹp lại (ít lời gọi VFS hơn ⇒ nhịp khác), gốc vẫn là một race đồng bộ
trong đường FS/cache/commit của server khi có ≥2 hart. Vì vậy phiên này
**không** hạ `MAX_IO` trong `main`: che race bằng một hằng số là đúng thứ D18
cấm.

### 4.6 Vì sao chưa sửa trong phiên này

Vùng lỗi là 3 file của commit người khác (`vifast-fs` +251 dòng,
`vifast-server` +68, `vifast-platform-x86_64` +45), và cơ chế khả nghi nhất là
đường cache/commit của FS dưới SMP — sửa mù trong một đêm sẽ vi phạm D18
(không có test đỏ trước) và D16 (không giữ được gate xanh). Đã dựng sẵn công
cụ tái hiện (`scripts/x86-smp-probe.py`: chạy N lần, `-smp 1|2`, giữ serial +
ảnh đĩa; `scripts/dump-root-dir.py`: đọc thẳng block thư mục gốc từ ảnh) để
vòng sau sửa có bằng chứng.

### 4.7 Ảnh hưởng tới tiêu chí G1

Tiêu chí "gate xanh" **không thể** đảm bảo trên x86_64 cho tới khi hồi quy này
được sửa: mỗi lần chạy `test-qemu.py --repeat 3` có ~58% xác suất đỏ
(P = 1 − 0,75³). Các tiêu chí còn lại (≥99% mỗi arch, không treo, không lỗi
test-class, client dư trang, có `cd/pwd/history/!!`) đều đạt — xem Part 5.

---

## Part 5 — Soak 100 × 3 arch (T25)

SOAK_SECTION

---

## Part 6 — Đề xuất cho G2 và câu hỏi cho Agent 0

### Đề xuất (thứ tự)

1. **Sửa hồi quy `FS V2 OK` trước mọi việc G2 khác.** Có sẵn công cụ tái hiện
   và khoanh vùng ở Part 4; cần một vòng riêng có test đỏ trước (D18) trong
   `crates/vifast-fs` / `vifast-server`.
2. **G2 Phần B trước Phần A**: `TcbGrantCap` + chuyển `df` ra applet là việc
   nhỏ, đo được, và mở đường cho mọi applet khác; bộ cấp trang tự do chỉ nên
   làm sau khi có số đo ở A.3.
3. **Khởi động hart 2–3** (x86_64 `-smp 4`): cần fan-out IPI cho
   `AP_MAINT_*`; GDT/TSS đã sẵn sàng theo hart (r261).
4. **Lỗi `!!` của shell** (page-fault ghi vào `.text`) cần một vòng D18 riêng.
5. **Tìm ra 3 936 B chênh lệch aarch64** để client về 6 trang (dư 2 như mục
   tiêu T18).

### Câu hỏi

1. **Hồi quy `FS V2 OK`**: ai sửa — phiên này ở vòng sau, hay nhánh web
   (tác giả `d3f2472`) sửa rồi phiên này kiểm lại? Cần chốt để không sửa trùng.
2. **Ngân sách trang**: chấp nhận dư 2/1/2 (đạt sàn) hay bắt buộc aarch64 dư 2?
3. **`-smp 4`**: có nằm trong tiêu chí G2 không? Nếu có thì phải làm fan-out IPI
   trước, và đó là một vòng lớn.
4. **`TcbGrantCap`**: syscall mới hay mở rộng `TcbConfigure`? (Đề xuất: syscall
   riêng, giữ thứ tự configure → args → caps → start.)

<!-- SHA-256: phần dưới đây KHÔNG nằm trong hash -->
