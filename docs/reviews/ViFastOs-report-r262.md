# ViFastOs — Báo cáo G1 hoàn tất + mở G2 (r254…r262)

Gửi: Agent 0. Người thực hiện: agent thi hành (phiên tự động đêm 09→10/11/2026).
Mốc đầu: `0e60423` (r255) · mốc cuối: `6c65091` (r262; commit báo cáo theo sau).
Trả lời trực tiếp `ViFastOs-decision-r254.md` (T17 → T25, D18–D21).

---

## Part 0 — Tóm tắt điều hành

Toàn bộ T17 → T25 đã chạy hết. **Hai việc phải đọc trước tiên**:

1. **Có một hồi quy thật, KHÔNG phải của phiên này — đã khoanh vùng và xử lý
   tạm.** Marker `FS V2 OK` hỏng **25,5%** số lần boot **x86_64**; bisect được
   về commit `d3f2472` của nhánh web (`r253 fix: (T16) sua 12 issue #2-#13`),
   và trigger là thay đổi `MAX_IO` 128→256 trong chính commit đó. Đã hạ
   `MAX_IO` về 128 (đo: 47 lần boot 256 B hỏng 12; 32 lần boot 128 B hỏng 0)
   ⇒ **gate xanh 10/10** lần đầu sau nhiều vòng. Gốc (race đồng bộ trong đường
   FS server khi có ≥2 hart) **vẫn mở**, có công cụ tái hiện và điều kiện nâng
   lại 256 — xem Part 4.
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
| T25 | soak 100 × 3 arch + tiêu chí đóng G1 | **DONE** — 298/300 (99,3%), 2 lỗi hiếm đã phân loại | Part 5 |

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

| Chỉ số | Giá trị cuối (r262) |
|---|---|
| Host test kernel | **236** (2 `#[ignore]` là fuzz) |
| Harness negative cases | 632 |
| Audit K | 43 CLOSED · 3 UNREACHABLE · **0 OPEN** |
| Audit P (mục kiến trúc) | P1-7 và P2 `AP_GDT` đóng ở r261 |
| Client trang (rv/arm/x86) | 6 / 7 / 7 trên ngân sách 8/8/9 ⇒ dư **2 / 1 / 2** |
| `MAX_IO` | **128** (hạ từ 256 ở r262 — xem Part 4.5) |
| Gate full | **PASS 10/10** (log `/tmp/gate-r262.log`) — lần đầu xanh hoàn toàn |
| Bước gate mới | `boundary` (2 bản × 2 arch, 4 lần boot) |
| Công cụ mới | `scripts/x86-smp-probe.py`, `scripts/dump-root-dir.py` |

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

Lưu ý đọc bảng: mẫu mỗi dòng chỉ 10–12 lần nên tỉ lệ là **xấp xỉ**; dòng
`d3f2472` (50%) cao hơn dòng HEAD `0e60423` (25%) không có nghĩa "HEAD tốt
hơn d3f2472" — r254/r255 đổi client (nhỏ và nhanh hơn) nên chỉ **dịch cửa sổ
race**, đúng như kết luận ở 4.5.

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
| `MAX_IO = 256` (từ d3f2472) | 47 | 12 | **25,5%** |
| `MAX_IO = 128` (giá trị trước d3f2472) | 32 | **0** | **0%** |

Nếu tỉ lệ thật ở 256 B là 25,5% thì xác suất thấy 32/32 lần xanh chỉ vì may mắn
là `0,745^32 ≈ 0,0001` (0,01%) — không thể là nhiễu.

`MAX_IO` là hằng số duy nhất thay đổi giữa hai cột, nên **trigger** của hồi quy
nằm trên đường IO 256 B (`MAX_IO` 128→256 là một trong các thay đổi của
d3f2472, kèm ghi chú "khớp `USER_COPY_MAX` và bề rộng copy IPC 256 B").

Điều này **không** có nghĩa "cứ hạ `MAX_IO` là xong": hạ nó chỉ làm cửa sổ
race hẹp lại (ít lời gọi VFS hơn ⇒ nhịp khác), gốc vẫn là một race đồng bộ
trong đường FS/cache/commit của server khi có ≥2 hart.

Quyết định của phiên này (r262): **hạ `MAX_IO` về 128 trong `main`**, kèm chú
thích nêu rõ đây là thu hẹp cửa sổ chứ không phải sửa gốc, **và** để lại công
cụ tái hiện + điều kiện để nâng lại 256 (test tái hiện được + sửa gốc trước).
Lý do: giữ `main` đỏ 25% là trạng thái không thể giao; một hằng số ABI hạ về
giá trị đã chạy ổn định nhiều vòng là thay đổi nhỏ, đo được, hoàn nguyên được —
khác với sửa mù trong đường FS của người khác.

### 4.6 Vì sao chưa sửa trong phiên này

Vùng lỗi là 3 file của commit người khác (`vifast-fs` +251 dòng,
`vifast-server` +68, `vifast-platform-x86_64` +45), và cơ chế khả nghi nhất là
đường cache/commit của FS dưới SMP — sửa mù trong một đêm sẽ vi phạm D18
(không có test đỏ trước) và D16 (không giữ được gate xanh). Đã dựng sẵn công
cụ tái hiện (`scripts/x86-smp-probe.py`: chạy N lần, `-smp 1|2`, giữ serial +
ảnh đĩa; `scripts/dump-root-dir.py`: đọc thẳng block thư mục gốc từ ảnh) để
vòng sau sửa có bằng chứng.

### 4.7 Cách tái hiện (cho vòng sửa gốc)

```sh
# 1) tái hiện: 12 lần boot x86_64, -smp 2, giữ serial + ảnh đĩa
VIFAST_KEEP_SERIAL=1 VIFAST_KEEP_DISK=1 \
    python3 scripts/x86-smp-probe.py 2 12

# 2) đối chứng SMP: cùng ảnh, chỉ 1 hart — kỳ vọng 8/8 xanh
python3 scripts/x86-smp-probe.py 1 8

# 3) soi block thư mục gốc của một lần hỏng (lba 30)
python3 scripts/dump-root-dir.py "$TMPDIR/vifast-smp2-<n>.img"

# 4) tái hiện bằng harness chuẩn (đắt hơn): 3 arch × 3 lần
python3 scripts/test-qemu.py --repeat 3
```

Mẹo: muốn thấy **bước nào** hỏng chứ không chỉ "thiếu marker", tạm thời hoá
chuỗi `v2 &= …` trong `crates/vifast-client/src/main.rs` thành bitmask và in
`FS V2 BAD mask=<n>` (đúng cách phiên này tìm ra bit 10); nhớ gỡ trước khi commit.

### 4.8 Ảnh hưởng tới tiêu chí G1

Tiêu chí "gate xanh" **không thể** đảm bảo trên x86_64 cho tới khi hồi quy này
được sửa: mỗi lần chạy `test-qemu.py --repeat 3` có ~58% xác suất đỏ
(P = 1 − 0,75³). Các tiêu chí còn lại (≥99% mỗi arch, không treo, không lỗi
test-class, client dư trang, có `cd/pwd/history/!!`) đều đạt — xem Part 5.

---

## Part 5 — Soak 100 × 3 arch (T25)

Lệnh: `python3 scripts/soak.py --iterations 100 --settle 3` tại HEAD cuối
(`r262`), sau khi gate xanh 10/10. Mỗi vòng boot cả 3 kiến trúc một lần.

| Arch | PASS | Tỉ lệ | Lỗi |
|---|---:|---:|---|
| riscv64 | 99/100 | 99,0% | 1 (vòng 62: `DESTROY DIAG stuck=4` + `BIN EXEC FAIL`) |
| aarch64 | 100/100 | 100,0% | — |
| x86_64 | 99/100 | 99,0% | 1 (vòng 53: `PAGER EVICT OK` trùng) |
| **Tổng** | **298/300** | **99,3%** | 2 lỗi, đều đã phân loại ở dưới |

Thời gian: 1 923,6 s cho 100 vòng × 3 kiến trúc (~19 s/vòng).

### Phân loại TỪNG lỗi (yêu cầu T25)

| Vòng | Arch | Marker | Nguyên nhân (đã soi serial/run.log) | Loại |
|---|---|---|---|---|
| 053 | x86_64 | `PAGER EVICT OK` xuất hiện 2 lần | root in marker này khi nhận `PAGER2_LABEL` **lần thứ hai**; lần thứ ba ⇒ in lại ⇒ có **message bị giao/ xử lý hai lần**. Cùng arch, cùng lớp "message lặp" với hồi quy `FS V2 OK` ở Part 4 | **lỗi thật (IPC), hiếm** |
| 062 | riscv64 | `DESTROY DIAG stuck=4` + `BIN EXEC FAIL` | `destroy_exec` gọi `object_destroy` cho 4 TCB, cả 2 pass đều trả `Busy` (err=20). `object_in_use` coi `current[hart]` là đang dùng (đúng như K37), mà TCB đã exit vẫn có thể còn là `current` của hart đang idle ⇒ không huỷ được ⇒ vòng exec sau hết slot | **lỗi thật (vòng đời TCB vs. destroy), hiếm** |

Hai lỗi này **không phải** hồi quy `FS V2 OK` và **không phải** lỗi do harness:
đều là hành vi thật của khách, tần suất ~1/60 vòng, chỉ lộ ra khi soak 100
(soak 20 trước đó không gặp). Đây đúng là loại lỗi mà soak dài tồn tại để tìm —
ghi thành T26/T27 cho vòng sau, chưa sửa trong đêm nay vì cả hai cần quyết định
thiết kế (IPC giao 2 lần; ai được destroy một TCB đã exit).

### Tiêu chí đóng G1 (đối chiếu)

| Tiêu chí (r254 §T25) | Kết quả | Đạt |
|---|---|---|
| Mỗi arch ≥ 99% PASS | 99,0% / **100%** / 99,0% | **ĐẠT** (sát sàn ở riscv64 và x86_64) |
| Không lỗi treo (timeout) | 0 timeout trong 300 lần boot | **ĐẠT** |
| Không lỗi test-class (do test sai) | 2 lỗi đều là hành vi thật của khách (không phải test sai) | **ĐẠT** (có điều kiện: xem T31/T32) |
| Client dư trang ≥ 2 / ≥ 1 / ≥ 2 | 2 / 1 / 2 | **ĐẠT** |
| Client có `cd`, `pwd`, `history`, `!!` | 4/4 (T17, r254) | **ĐẠT** |
| Gate xanh | 10/10 (`/tmp/gate-r262.log`) | **ĐẠT** |

---

## Part 6 — Đề xuất cho G2 và câu hỏi cho Agent 0

### Đề xuất (thứ tự)

1. **Sửa GỐC của hồi quy `FS V2 OK` trước mọi việc G2 khác.** `main` đã xanh
   trở lại nhờ hạ `MAX_IO`, nhưng đó là thu hẹp cửa sổ race; gốc là race đồng
   bộ trong đường FS server khi có ≥2 hart. Có sẵn công cụ tái hiện
   (`scripts/x86-smp-probe.py`) + cách soi (`scripts/dump-root-dir.py`) ở
   Part 4.7; cần một vòng riêng có test đỏ trước (D18).
2. **G2 Phần B trước Phần A**: `TcbGrantCap` + chuyển `df` ra applet là việc
   nhỏ, đo được, và mở đường cho mọi applet khác; bộ cấp trang tự do chỉ nên
   làm sau khi có số đo ở A.3.
3. **Khởi động hart 2–3** (x86_64 `-smp 4`): cần fan-out IPI cho
   `AP_MAINT_*`; GDT/TSS đã sẵn sàng theo hart (r261).
4. **Lỗi `!!` của shell** (page-fault ghi vào `.text`) cần một vòng D18 riêng.
5. **Tìm ra 3 936 B chênh lệch aarch64** để client về 6 trang (dư 2 như mục
   tiêu T18).

### Câu hỏi

1. **Hồi quy `FS V2 OK`**: ai sửa gốc — phiên này ở vòng sau, hay nhánh web
   (tác giả `d3f2472`) sửa rồi phiên này kiểm lại? Cần chốt để không sửa trùng.
   Phiên này **đã** hạ `MAX_IO` về 128 như một biện pháp tạm (đo được, hoàn
   nguyên được); nhánh web cần biết để không nâng lại 256 mà không có bằng
   chứng.
2. **Ngân sách trang**: chấp nhận dư 2/1/2 (đạt sàn) hay bắt buộc aarch64 dư 2?
3. **`-smp 4`**: có nằm trong tiêu chí G2 không? Nếu có thì phải làm fan-out IPI
   trước, và đó là một vòng lớn.
4. **`TcbGrantCap`**: syscall mới hay mở rộng `TcbConfigure`? (Đề xuất: syscall
   riêng, giữ thứ tự configure → args → caps → start.)


---

## Phụ lục A — sau khi ký báo cáo: r263 và r264

Báo cáo trên chốt ở r262. Hai vòng sau đó hoàn tất việc bàn giao và mở đường G2;
ghi lại ở đây để Agent 0 có trạng thái cuối, không phải đọc thêm tài liệu khác.

### r263 — chốt báo cáo

`docs/reviews/ViFastOs-report-r262.md` (chính tài liệu này) + `scripts/sign-report.py`
(tính/ghi khối SHA-256 theo quy ước r253) + sửa số case harness 462 → 632 trong
`docs/11-hien-tai/09-testing.md`.

### Đính chính kỹ thuật (r267) — quan trọng cho G2

Bản đầu của báo cáo này (và của `docs/design/g2-memory-and-exec-caps.md`) nói
`exec` **không có kênh uỷ quyền cap**. Sau khi đọc lại mã root: **sai**.

`CapMint`/`CapCopy` đã là kênh đó, và root **đang dùng thật** để cấp cho client
slot 1 (badge 1 — danh tính shell) và slot 6 (badge 2) ngay lúc boot
(`crates/vifast-root/src/main.rs`, khối `cap_mint` cho `client_tcb`). Vậy thứ
thiếu để đưa `df` ra applet chỉ là:

1. `spawn_elf_from_fs` chưa `cap_mint` cho chương trình con (khoảng 3 dòng), và
2. **quyết định chính sách badge** — con thừa hưởng badge 1 (danh tính shell)
   hay badge riêng; badge là `uid` mà FS server dùng để kiểm quyền file.

`TcbGrantCap` (r264) vẫn có giá trị riêng (bản không đặc quyền: uỷ quyền cap đã
có badge mà không cần mint badge mới), nhưng **không phải** điều kiện tiên
quyết của G2 Phần B — và nó không đáng bị coi là "việc lớn" như bản đầu mô tả.

### r264 — T27 (một phần): `TcbGrantCap`

Đây là **đường mà `exec` đang thiếu** (G2 Phần B). Đã có:

| Thành phần | Nội dung |
|---|---|
| ABI | `SyscallId::TcbGrantCap = 57` (`a0`=slot TCB đích, `a1`=slot nguồn, `a2`=slot đích, `a3`=mask quyền; `0` = giữ nguyên trừ `GRANT`) |
| Kernel | `grant_cap_to_tcb` + arm syscall: kiểm WRITE trên TCB đích, cap nguồn phải có thật, slot đích trống, quyền **thu hẹp**, badge **giữ nguyên**, cấm chuyển Untyped gốc, derive (caller không mất cap) |
| User | `vifast_user::tcb_grant_cap` |
| Test | 6 host test, mỗi test khoá một ràng buộc; riêng clamp `GRANT` đã chứng minh **đỏ** khi tạm bỏ |

Kernel host test: 236 → **242** (240 chạy + 2 fuzz `#[ignore]`).

**Chưa làm** (và vì sao): nối `TcbGrantCap` vào đường `exec` rồi chuyển `df` ra
applet cần chốt **chính sách badge** — badge là `uid` mà FS server dùng để kiểm
quyền file, nên phải quyết định chương trình con thừa hưởng badge của shell hay
của root trước khi viết mã. Đây đúng là câu hỏi 4 ở Part 6.

### Một thí nghiệm thất bại, đã hoàn nguyên (ghi để khỏi lặp)

Định sửa lỗi soak 100 vòng 62 (`DESTROY DIAG stuck=4` → `BIN EXEC FAIL`) bằng
cách nhả `current[hart]` khi hart park. **Boot chết cả 3 kiến trúc** (timeout,
thiếu 37 marker) ⇒ đã hoàn nguyên toàn bộ, build lại, boot 3/3 PASS. Suy đoán:
`current` còn là chỗ duy nhất giữ danh tính task để lưu/khôi phục context và để
`Exit` dọn dẹp. Hướng đúng nhiều khả năng là cho `destroy_object` chấp nhận TCB
`Exited` và tự gỡ `current` của chính nó, không phải dọn ở đường park.

### r268 — T27 hoàn tất: `df` thành applet, con nhận cap VFS

Đây là **applet đầu tiên cần capability**, nên nó chứng minh end-to-end đường
uỷ quyền cap cho tiến trình con — đúng mục tiêu G2 Phần B.

| Việc | Chi tiết |
|---|---|
| Root | `spawn_elf_from_fs` mint endpoint VFS từ cap GỐC (badge 0) vào slot 1 của con với **badge 1** = danh tính shell (không phải badge root) |
| `vifast-bin` | applet `df` gọi `STATFS` qua slot 1 (không cần IPC buffer: mọi tham số nằm trong message words) |
| Client | bỏ builtin `df` (960 B mã); shell fallback exec `/bin/df` |
| Ảnh đĩa | hard link `/bin/df` cùng inode `/bin/vf` |
| Cổng | `--interactive` thêm bước `df` với marker `blocks`; nếu cap thiếu thì applet in `df: error` ⇒ bước ĐỎ |

Quyết định **chính sách badge** (câu hỏi 4 ở Part 6): con thừa hưởng badge 1
(danh tính shell). Cấp badge root sẽ cho applet quyền đọc/ghi toàn bộ FS — vượt
xa nhu cầu của `df`.

Đo được trên QEMU thật:

```text
[riscv64] df -> 'blocks': OK      -- riscv64 interactive PASS
[aarch64] df -> 'blocks': OK      -- aarch64 interactive PASS
```

QEMU chuẩn 3 arch × 3 lần: 9/9 PASS. Gate full: xem dòng trạng thái cuối.

Bài học lặp lại lần thứ ba trong phiên này: **giả định kỹ thuật phải kiểm bằng
mã**. Bản đầu của tài liệu G2 khẳng định "exec không có kênh uỷ quyền cap" và
đề xuất viết syscall mới; đọc lại mã root thì thấy `CapMint` đã làm đúng việc
đó và đang được dùng thật cho client. Việc thật cần làm chỉ là **3 dòng nối
dây + một quyết định chính sách**.

### r271 → r273 — T32: đã sửa, rồi phải HOÀN NGUYÊN (ghi trung thực)

r271 sửa lỗi soak 100 vòng 62: `destroy_object` chấp nhận TCB `Exited` dù nó
còn là `current` của hart idle (dọn luôn tàn dư `current`). Hai host test
đỏ-trước, gate 10/10, soak 30 tại r271 = **90/90**.

**Nhưng soak 100 trên HEAD ngay sau đó hỏng ở vòng 2**: x86_64 **treo** ngay
sau `shell> `, thiếu đúng một marker `PREEMPT OK`. Trước bản vá, hai đợt soak
30 liên tiếp là 90/90 và không có lần treo nào; một lần treo ở 2 vòng đầu là
tín hiệu hồi quy, và treo thì nặng hơn hẳn triệu chứng cũ (`DESTROY DIAG`,
vẫn boot xong). ⇒ **r273 hoàn nguyên r271** (`git checkout` phần kernel), đưa
`current`/`destroy` về hành vi cũ.

Trạng thái T32: **vẫn mở**, nhưng nay có thêm dữ kiện quan trọng — hướng "dọn
`current` khi destroy" **chạm vào đường scheduler của hart đang chạy**, không
phải thay đổi cục bộ. Vòng sau cần: (a) tái hiện được cảnh treo này (chạy
soak 100 nhiều lần trên bản vá), (b) hiểu ai còn đọc `current` sau khi task
exit (`exit_current_on`, `note_current_tcb`, đường `Sleep` → `wait_for_work`),
rồi mới sửa lại.

Bài học giữ nguyên: **một bản vá "xanh gate + xanh soak 30" vẫn có thể là hồi
quy** — soak 100 mới đủ để bắt.

### r274 — áp lại T32; r275/r276 — T33: chẩn đoán, một lần đính chính SAI, rồi kết luận đúng

r273 hoàn nguyên T32 vì soak 100 hỏng ngay vòng 2 (x86_64, thiếu `PREEMPT OK`).
Soak 40 trên bản đã hoàn nguyên lại hỏng đúng triệu chứng đó ⇒ treo không do
T32 ⇒ r274 áp lại.

Rồi tới một chuỗi sai–sửa đáng ghi lại nguyên văn:

1. **r275 — kết luận SAI**: thấy serial của lần hỏng có đủ marker (kể cả
   `PREEMPT OK`) nên kết luận "harness đọc sớm, không phải lỗi kernel". Sai vì
   file serial đó đã bị **lần chạy thử lại thứ hai ghi đè** — harness có cơ chế
   retry, và serial dùng chung một đường dẫn.
2. **r276 — thêm `read_settled_serial()`** (đọc lại sau khi QEMU chết, chờ file
   ổn định). Cải thiện này vẫn đúng và giữ lại, nhưng nó không phải nguyên nhân.
3. **Đọc lại `run.log` của đúng lần hỏng** (không phải serial) mới ra sự thật:

   | Vòng | Arch | Dòng trong run.log | Nghĩa |
   |---|---|---|---|
   | 004 | x86_64 | `rc=-9, 80.1s` + thiếu `PREEMPT OK` | boot **treo thật** tới hết 80 s |
   | 096 | riscv64 | lần 1: `rc=0, 2.6s` thiếu 48 marker; lần 2 PASS | guest **thoát sớm** rồi lần chạy lại xanh |

⇒ T33 là **lỗi thật, hai triệu chứng**:
- **T33a (x86_64, ~1%)**: treo trước `PREEMPT OK`; chẩn đoán `SMP IDLE STALL`
  cho thấy bộ đếm post-idle của hart phụ kẹt ở 3–4 (cần ≥5) ⇒ hart phụ ngừng
  thức. Đã thêm `SMP AP STATE (online, window, gen, wake_gen, ipi, resched)`
  (r275) để vòng sau đọc được trạng thái AP lúc treo.
  **Bằng chứng mới (r284)**: lỗi này đã bắt được cả **gate** — lần chạy gate đầu
  ở r284 đỏ đúng `qemu` với `x86_64 2/3` và `timeout (thiếu 1 marker: PREEMPT
  OK)`; chạy lại thì xanh 10/10. Tức với tỉ lệ ~1% mỗi lần boot thì mỗi lượt
  gate (9 lần boot) có ~3% khả năng đỏ oan vì lỗi này — cần sửa trước khi coi
  "gate đỏ" là tín hiệu tuyệt đối.
- **T33b (riscv64, ~1%)**: guest thoát mã 0 sau 2,6 s với 48 marker thiếu; lần
  chạy lại xanh ⇒ thoát sớm, không phải treo.

Cơ chế retry có sẵn của harness che hai lỗi này ở đường `--repeat`, nhưng soak
đếm lần đầu là hỏng — đúng, vì lần đầu hỏng thật.

**Bài học phương pháp (lần thứ tư)**: khi có retry, phải xác định **serial thuộc
lần chạy nào**; `run.log` mới là nơi ghi rõ `rc` và thời lượng của từng lần.

### r283 — bài test hồi quy D18 cho BLOCKER `FS V2 OK` (T26)

Lỗi gốc chỉ lộ ra qua nhiều lần boot QEMU nên không viết được unit test trên
host. Đã thêm `scripts/fs-smp-regression.py` — dạng "test đỏ trước" khả thi cho
lỗi này, với hai chế độ:

| Chế độ | Ý nghĩa | Kết quả đo |
|---|---|---|
| `--expect green` (mặc định) | điều kiện ĐÓNG lỗi: N lần boot x86_64 phải xanh hết | **8/8 xanh** ở `MAX_IO = 128` (đo trong phiên) |
| `--expect red` | chứng minh bài test CÓ RĂNG: phải thấy ít nhất 1 lần đỏ | đỏ đã đo được ở `MAX_IO = 256`: **12 hỏng / 47 lần boot** (Part 4) |

Script tự đọc `MAX_IO` từ crate ABI và in ra để log biết đang đo ở giá trị nào;
nó không tự rebuild (bài học r280). Từ nay, **nâng `MAX_IO` phải kèm chạy
`--expect green` với N ≥ 40**; nếu đỏ thì đó đúng là điều kiện cấm nâng.

Đồng thời dọn kho: xoá nhánh `r254` + worktree `/private/tmp/vf-r254` (nội dung
duy nhất còn giá trị — phân tích T18 vì sao aarch64 dư 1 trang — đã chép vào
`progress-g1.md`) và xoá bốn nhánh đã merge/hết giá trị (`g1`, `r261`, `r268`,
`t13x`). Còn đúng một worktree: `main`.

### r289 – r293 — tầng đáy, capability, FS, và hai chẩn đoán T33

**r289** (commit `1e5c051`): payload (va, pages, asid) vào TRONG hộp thư
maintenance và hợp nhất chỉ-nới-rộng; x86 shootdown đối xứng (bỏ `target != 1`,
mỗi hart một hộp thư, đếm theo cặp nguồn→đích); `BKL.is_held()` → `held_by`
ở 8 chỗ 3 port; sàn test đổi từ hằng số `-ge 138` sang số ĐO ĐƯỢC
(`scripts/test-floor.py`), mutation xoá 1 test ⇒ ĐỎ.

**r290** (`fc6aec4`, `f5c28b4`, `75ffb6e`): `CapRef.cnode` + `TableId` +
`Kernel::for_each_cap` (đi qua cả CNode đã tháo khỏi TCB) + `resolve_cap_ref`;
kịch bản 5 bước của **P0-3** có test đỏ-trước
`orphan_cap_in_a_detached_cnode_is_reparented_and_revoked` (ĐỎ đúng chỗ "parent
phải trỏ cap gốc của root" khi dùng vòng duyệt cũ); `check_invariants` kiểm
`cap.parent` ở MỌI nhánh; `reply_epoch` tăng mỗi lần call (guard K49 hết rỗng).

**r291** (`c59bbb8`, `ca78f58`): FS — biên theo superblock (`capacity`), chặn
chỉ số indirect bịa, lỗ file thưa hợp lệ (`read` trả 0, `fsck` bỏ qua), rollback
inode khi `create` hỏng. Bốn test đỏ-trước, đã kiểm chứng ngược từng cái.

**r292**: tái hiện race FS — `MAX_IO=256` ⇒ **3/20** lần đỏ (`FS V2 OK` thiếu);
giữ `MAX_IO=128`, ghi ba nghi phạm điều tra tiếp.

**r293** (`9c6d29f`): T33a thêm `SMP NO READY` + `live_tcb_count()` (đo 80 lần
x86: 80/80, **0** lần in) ⇒ nhánh thoát im lặng không phải gốc; T33b: hart idle
gặp ngoại lệ nay KHÔNG `halt()` cả máy mà ack + quay lại idle (đo riscv 60/60,
0 `KERNEL FAULT`).

Số đo tổng: gate **10/10** ở mỗi mốc; soak 100×3 (sau r290) = **299/300**;
`vifast-kernel` 250 test, `vifast-fs` 44; sàn **365 test / 11 crate**.

### r293 – r294.B — thước đo mới, flake scheduler, và GỐC của T33a

**Thước đo (r293.1–293.3, commit `2a04bd2`)**: harness đếm `RETRY n/N`, phân lớp
T-FS/T-SCHED/T-KHAC, lưu artifact đầy đủ cho MỌI lần hỏng, có `--strict`. Nhờ nó
mà lộ ra flake thật **2,6 %/boot** — trước đó "PASS" che mất.

**r294.4 (`c2a8968`)**: `fault::suspend` gặp "hết TCB chạy được" thì `halt()`
TRONG trap handler đang giữ BKL ⇒ hart kia quay BKL mãi với IF=0 ⇒ chết máy
(4/305). Sửa: trả `false`, caller nhả BKL rồi mới halt. Đo sau vá: **0/301** lần
`tcb=None`/`SMP PARK` ⇒ đóng theo số đo (tỉ lệ < 3/301).

**GỐC T33a (`28aefcd`)**: đèn `NO TICK` (chạy trên hart còn sống) cho
`lvt0=0x10000` — **LINT0 của BSP bị MASK** từ tick 2, nên PIC không đẩy được
IRQ0 ⇒ tick ngừng vĩnh viễn ⇒ `PREEMPT OK` (cần tick ≥ 99) không bao giờ in.
Chỉ có hai chỗ ghi LVT0 trong toàn bộ mã: BSP đặt `0x700` (ExtINT, không mask)
và **AP mask `1<<16`**. Sửa: AP chỉ mask LINT1.

| Lượt (x86_64) | Sha | N | Kết quả |
|---|---|---:|---|
| Nền | `c2a8968` | 305 | 8 hỏng = **2,6 %** (6 T-ROOT `main.rs:480`, 1 T-BOOT, 1 T-INFRA) |
| Sau vá LINT0, không đèn | `3699fe7` | 301 | **2 hỏng = 0,66 %** (1 T-SCHED treo, 1 T-DUP `PREEMPT OK` in hai lần) |

Hai giả thuyết bị BÁC bằng đèn trước khi tìm ra gốc: "PIT ngừng phát"
(`TICK`/`eoi` khớp) và "PIC kẹt in-service" (`isr=0, imr=0xec`).

**Khối D (`6c026a4`)**: D1.a marker `SMP NO CURRENT` qua hàm thuần + 3 test host;
D3 panic in `file:dòng` có dấu hai chấm, harness đọc được cả hai dạng; bảng phân
loại 294.C có `T-BOOT`/`T-ROOT <file:dòng>`/`T-INFRA`. **D2 đã revert theo luật**
(3 test scheduler + `selftest_passes` đỏ — đúng như r251 đã ghi).

### Sau nửa đêm — hai GỐC của lớp flake x86 đã tìm và vá

**Gốc 1 (`28aefcd`)**: AP ghi LINT0 (0x350) của LAPIC ⇒ BSP bị **mask LINT0**
(`lvt0=0x10000` đọc lại bằng đèn `NO TICK` ngay từ tick 2) ⇒ PIC không đẩy được
IRQ0 ⇒ tick ngừng vĩnh viễn ⇒ `PREEMPT OK` (cần tick ≥ 99) không bao giờ in.
Chỉ có một chỗ ghi LVT0 trong toàn mã (BSP đặt `0x700`), nên bản vá là: AP chỉ
mask **LINT1**.

**Gốc 2 (`8cd767f`)**: AP được thả trong `init_platform()` **trước** cả hai lần
`A::init_kernel()` (mỗi lần `reset()` xoá `system_ticks`/runqueue/`current`).
AP kịp `pick_on(1)` trên trạng thái tiền-reset ⇒ worker chạy với `last` từ đồng
hồ cũ, `GetTime` sau reset đọc đồng hồ mới ⇒ `TIME MONOTONIC BAD` ⇒
`ROOT PANIC crates/vifast-root/src/main.rs:480`. Bằng chứng: vòng đệm 96 sự kiện
toàn `hart=1, tick=2`. Bản vá: cờ `KERNEL_READY`, AP chỉ `sti; hlt; cli` tới khi
BSP bật cờ ở `prepare_user()`.

| Lượt (x86_64) | Sha | N | Kết quả |
|---|---|---:|---|
| Nền | `c2a8968` | 305 | 8 hỏng = **2,6 %** |
| Vá LINT0 | `3699fe7` | 301 | 2 hỏng = **0,66 %** |
| Vá LINT0 + `KERNEL_READY` | `8cd767f` | 301 | 3 hỏng = **1,0 %** (1 T-ROOT, 1 T-SCHED, 1 **T31 dup `PAGER EVICT OK`** — lần đầu tái hiện được T31) |

Hai mốc sau chênh nhau 3/301 so với 2/301 ⇒ trong nhiễu; **không** kết luận bản
vá thứ hai làm tệ đi, cũng **không** nói đã hết lớp T-ROOT. Việc tiếp đã ghi ở
`progress-g1.md`.

### r294.C — mốc khởi động và định nghĩa T-BOOT

- Kernel in `KERNEL ENTRY` ngay đầu `boot()` (trước `init_platform`) ⇒ mốc sớm
  nhất chắc chắn có; `classify_failure` dùng đúng định nghĩa của phiếu soát
  §1.5: **T-BOOT ⇔ không có `KERNEL ENTRY`**, xét trước mọi lớp.
- Harness: artifact đọc **lại** serial đã settled từ file. Vá lỗi khiến artifact
  của lần hỏng T31 (`dup=PAGER EVICT OK`) không chứa chính marker gây hỏng.
- Spec harness: **652 case**, gồm ca âm "đã vào kernel mà thiếu `PREEMPT OK` ⇒
  T-SCHED, không phải T-BOOT".

**Số đo vòng này (x86_64, bản không đèn):** 200 boot ⇒ `RETRY 1/200`, và lần
hỏng đó là **T-ROOT `main.rs:480`** — nghĩa là cờ `KERNEL_READY` mới chặn được
một phần lớp này. T31 (`PAGER2_LABEL` xử lý hai lần) **không** tái hiện trong
200 boot; lần duy nhất bắt được vẫn là ở lượt 300 boot trước.

### r294.T31 — nghi vấn gốc: context không được ghi lại sau `apply_resume`

Phân tích độc lập (subagent, chỉ đọc mã) kết luận: hai dòng `PAGER EVICT OK`
nghĩa là thân nhánh PAGER2 chạy **≥3 lần** trong khi client chỉ có hai `send`;
cơ chế khả dĩ nhất là `apply_resume()` vá **context sống** mà **không ghi lại**
`contexts[slot]`, nên lần nạp slot sau chạy lại **snapshot cũ** (mang đúng bộ
thanh ghi message).

Bản vá (`79e0f13`): ghi lại `contexts[slot]` sau `apply_resume` ở **cả 5 điểm
nạp**; tách hàm thuần `slot_choice(Option<u8>)` + test host cho ca "TCB thiếu
slot" (trước đây lặng lẽ `unwrap_or(0)` — ghi nhầm context của root); gỡ feature
chết `diag-pick` (khai báo nhưng không dùng, tài liệu cũ ghi sai).

Phép đo để chốt T31 (chưa chạy vì hết giờ): đếm `USER FAULT addr=0x1001001000`
trong serial của lần hỏng — 2 ⇒ replay phía server, 3 ⇒ replay phía client.

### T33a — chốt cuối ngày 10/11

| Bản vá | N | Tỉ lệ |
|---|---:|---|
| (nền `c2a8968`) | 305 | 2,6 % |
| LINT0 (`28aefcd`) | 301 | **0,66 %** |
| + `KERNEL_READY` (`8cd767f`) | 301 | 1,0 % |
| + ghi lại context (`79e0f13`) | *chưa đo* | — |

Còn mở (đã ghi việc tiếp trong `progress-g1.md`): T-ROOT `main.rs:480` 1/200,
T-SCHED 1/301, T31 (phép đo ở trên), `BIN EXEC FAIL`/`DESTROY DIAG stuck=4`,
T26, T33b, 294.5.

### r294.T33b — riscv64: panic của hart phụ (`assert_current_tcb`)

Handler panic của riscv64/aarch64 nuốt `PanicInfo` nên log chỉ có
`PANIC (hart=1)` — đo 200 boot riscv64 gặp 2 lần mà **không biết chỗ nào**. Đã in
`PANIC AT <file>:<dòng>` (cùng kiểu D3 của root) và cho harness phân loại
`T-ROOT <file:dòng>`; spec 653 case.

Chụp được ngay: `crates/vifast-platform-riscv64/src/platform.rs:671` —
`assert_current_tcb()` "per-CPU current_tcb mismatch". Gốc: `fault_to_endpoint`
xoá `current` của **mọi** hart khi task bị park vì fault, còn cache per-CPU
không được cập nhật ⇒ lần trap sau lệch ⇒ **máy chết**. Đây là bất biến **chẩn
đoán**, nên bản vá là: lệch thì **sửa cache theo kernel** và in một lần
`CURRENT MISMATCH`, không `assert_eq!`.

| Lượt riscv64 | N | Kết quả |
|---|---:|---|
| Trước | 200 | 2 hỏng — **2 × `PANIC (hart=1)`** |
| Sau | 201 | 1 hỏng — **0 panic**; 1 lần thiếu marker `IPC RIGHTS OK` |

Triệu chứng "TCP echo đỏ" hoá ra chỉ là **hệ quả** của panic (guest chết trước
khi in marker). Sau khi vá, TCP echo không còn là nguyên nhân duy nhất; việc
tiếp là đo riêng "thiếu marker `IPC RIGHTS OK`".

### r294.T26 — hai phép đo quyết định về race FS ở `MAX_IO=256`

| Cấu hình | N | Kết quả |
|---|---:|---|
| `MAX_IO=128`, đệm IPC 256 B (sản phẩm) | 40 | **40/40 xanh** |
| `MAX_IO=256`, đệm IPC 256 B | 40 | **12/40 đỏ** (tái hiện đúng 12/49 của r262) |
| `MAX_IO=256`, **đệm IPC 512 B** | 40 | **8/40 đỏ** |

**Kết quả âm tính có giá trị:** nới đệm IPC 256 → 512 B **không** diệt được lỗi
(12/40 → 8/40, trong nhiễu) ⇒ **không** phải "payload 256 B đụng trần đệm IPC";
đây là race thật của đường FS khi ≥2 hart, đúng như ghi chú r262. Đã trả nguyên
trạng `IPC_BUFFER_WORDS=32`, `MAX_IO=128`.

**Đính chính:** artifact `flake-x86_64-1-a0` đọc lúc đầu **có `FS V2 BAD`** (kết
luận "treo không in OK lẫn BAD" là sai); ba artifact thật sự thiếu `FS V2` lại
có `serial.txt` **0 byte** ⇒ muốn biết treo ở đâu phải thêm đèn, không suy từ log
rỗng.

Việc tiếp: đèn một marker mỗi bước trong khối V2 + in mã lỗi thay vì `EIO`; vá
`ipc.rs::reply()` (không bỏ rơi client `BlockedReply` khi `valid==false`) và
`reply_recv()` (không `?` trước `recv`).

### r294.T26 — giả thuyết reply-orphan bị BÁC bằng đo (đã revert)

Đã thử bản vá theo giả thuyết H1 của bản phân tích: `ipc.rs::reply()` đánh thức
client `BlockedReply` khi caller stale, và `reply_recv()` không bỏ lượt `recv`
khi reply lỗi. Kèm test đỏ-trước đã chứng minh ĐỎ khi tắt bản vá.

| Cấu hình | N | Đỏ |
|---|---:|---:|
| `MAX_IO=256` trước vá | 40 | 12 |
| `MAX_IO=256` sau vá | 40 | 9 |
| `MAX_IO=128` sau vá | 40 | **1 (hồi quy)** |
| `MAX_IO=128` sau revert | 40 | **0** |

12 → 9 nằm trong nhiễu ⇒ H1 **không** phải gốc; và bản vá làm **cấu hình sản
phẩm** đỏ 1/40. Theo D18 đã **revert cả bản vá lẫn test**; cây về 40/40 xanh.

Bài học: thay đổi `wake_error` trên đường IPC nóng là thay đổi **hành vi**, phải
đo riêng từng bản vá và luôn kiểm cấu hình đang xanh (128) trước khi tin kết quả
ở cấu hình đỏ (256).

### r294.T26 — khoanh vùng được: lỗi ở `vfs_fsck()`, không phải bước FS nào khác

Thêm ba đèn giữ lại lâu dài: mốc `V2S01..V2S11` sau mỗi bước của khối FS V2,
dòng `V2FAIL v2=<0|1> fsck=<0|1>` ngay trước `FS V2 BAD`, và mã lỗi fsck hai phía
(server `FS ERR <mã>`, client `FSCKERR <mã>`) — trước đây chỉ có bool nên không
phân biệt được "fsck nói hỏng" với "mất phản hồi IPC".

**Kết quả (`MAX_IO=256`, 12/40 đỏ):** 100 % lần đỏ đều đi hết `V2S11` và in
`V2FAIL v2=1 fsck=0` ⇒ mọi kiểm tra V2 khác ĐẠT, chỉ `vfs_fsck()` trả false.

Điều này **bác** cách giải thích cũ ở đây ("readdir đọc ảnh cũ"): nếu readdir sai
thì `v2` phải bằng 0. Ở `MAX_IO=128` sau khi thêm đèn: **40/40 xanh**.

### r294.T26 — máy thứ hai làm lộ hai điều

**Số nền đúng (HEAD, `MAX_IO=128`):** Mac 40/40 xanh **và** `ps` (Ubuntu x86_64)
40/40 xanh.

**Ba bản vá thử từ bản phân tích:** P2 (`ata_wait_bsy` kiểm bit `ERR/DF` thay vì
chỉ chờ `BSY`), P1 (`read_dir_entry_full` đọc bằng `read_staged`), P3 (tách nhóm
đèn 04). Ở `MAX_IO=128` trên `ps`: **2/40 đỏ** trong khi baseline cùng máy 40/40
xanh ⇒ **hồi quy cấu hình sản phẩm**, đã **revert theo D18** (giữ trong
`git stash` để vòng sau lấy lại).

**ĐÍNH CHÍNH (subagent bắt được, quan trọng):** kết luận "P2 làm lộ lỗi ATA" là
**SAI**. `ERR/DF` là bit **dính**, chỉ bị xoá khi host ghi thanh ghi Command; tôi
lại dùng `ata_wait_bsy` (đã kiểm `0x21`) làm **cổng TRƯỚC khi phát lệnh** trong
`ata_prepare` ⇒ sau lỗi đầu tiên mọi lệnh ATA chết ⇒ 3 lượt retry không hề phát
lại lệnh nào. Đỏ 2/40 ở 128 là **cascade tự gây**, không phải bằng chứng về thiết
bị. Đã revert `platform.rs` + `vifast-fs`.

Thiết kế đúng (vòng sau): tách `wait_bsy_only` (chỉ BSY, dùng TRƯỚC khi ghi
command) và `wait_done` (BSY=0 rồi mới xét ERR/DF + đọc Error 0x1F1, dùng SAU khi
ghi command, SAU 256 word và sau FLUSH); đèn có `stage`/`class`/`lba`; ngân sách
retry toàn cục; `ATA_DEAD` khi timeout lặp. Kèm đèn `ATA STAT cmds=` ở **cả 128
và 256**: nếu số lệnh ATA xấp xỉ nhau thì lỗi ATA **không thể** giải thích chênh
lệch 12–18/40 — retry là sửa bug thật nhưng **không phải** sửa T26.

**Số đo sau revert (128, song song 2 máy):** local 40/40; `ps` 39/40 ⇒ chưa đủ
kết luận 128 tuyệt đối xanh trên `ps`; cần ≥120 boot nữa.

### r294.T26 — PHÉP ĐO QUYẾT ĐỊNH: race có ở CẢ `MAX_IO=128` trên máy thứ hai

Revert **toàn bộ** về HEAD (cây sạch, không đèn, không bản vá ATA) rồi chạy
**80 boot** trên `ps`: **3/80 đỏ** (~3,75 %).

⇒ `MAX_IO=128` **không** tuyệt đối xanh trên `ps`; lượt "40/40 xanh" trước đó chỉ
là may (P(0/40) ≈ 22 % khi p ≈ 0,0375). Ba hệ quả:

1. Các bản vá của tôi không phải nguyên nhân của 1–2/40 ở 128 — nằm trong tỉ lệ
   nền có sẵn.
2. **Race read-your-own-write của FS server KHÔNG phụ thuộc `MAX_IO`**: có ở cả
   128 (~3,75 % trên `ps`) lẫn 256 (10–45 % tuỳ máy). `MAX_IO` chỉ **khuếch đại**
   tần suất — khớp giả thuyết "khác nhau ở thời điểm, không ở bộ đếm".
3. Không được dùng câu "128 xanh nên an toàn"; phải nói "128 có ~1–4 % tuỳ máy" và
   định nghĩa mức chấp nhận bằng số.

**Loại trừ dứt khoát tầng ATA bằng số:**

| `MAX_IO` | cmds | writes | flushes | retry | fail |
|---|---:|---:|---:|---:|---:|
| 128 | 2303 | 2304 | 2568 | 0 | 0 |
| 256 | 1535 | 1536 | 1694 | 0 | 0 |

Ở 256 có **ít** lệnh ATA hơn và **0** lỗi/retry, kể cả ở lần boot đỏ ⇒ ATA không
phải gốc T26. (Bản vá `wait_done` + `ATA STAT` vẫn đúng và đang để trong
`git stash` để đo riêng.)

### r294.T26 — ĐỘT PHÁ: không mất dữ liệu, lỗi ở tầng LIỆT KÊ

Đèn phân biệt chạy ngay trong client khi `readdir("/")` thiếu `d`: hỏi thẳng
`open("/d")` rồi lặp readdir 2 lần nữa. Kết quả ở `MAX_IO=256` (2/2 lần đỏ):

```
V2GROUP 06  V2CHK open=1 rd=2
```

`open=1` ⇒ ảnh FS **có** `d`; `rd=2` ⇒ **cả hai** lần readdir lặp lại đều **thấy**
`d`. ⇒ **Không mất dữ liệu**, **không phải race hai hart ghi**; chỉ **lần liệt kê
đầu** bỏ sót.

Hệ quả: loại cơ chế "RMW đọc ảnh cũ ghi đè" và "hai hart cùng thực thi một `Fs`";
còn lại **tầng liệt kê** — gói nhiều record vào khung IPC 256 B (`READDIR_REC=32`
⇒ 8 record = 256 B = **đầy khung, không slack**), client giải mã lười từng record,
và cả hai phía `break` im lặng khi gặp `Err`
(`vifast-client/src/main.rs:575-581`, `vifast-server/src/main.rs:648-652`).

Hướng sửa đổi hẳn: **không** sửa journal/cache mà sửa **đường readdir** (trang hoá
+ không cắt danh sách trong im lặng). Ở `MAX_IO=128`: 40/40 xanh.

### T26 — tổng hợp điều tra (sau 11 giả thuyết bị loại bằng đo)

Triệu chứng: ở `MAX_IO=256`, ~10–45 %/boot (tuỳ máy) `readdir("/")` không thấy
entry `d` vừa `mkdir`, rồi `V2GROUP 06`. Ở `MAX_IO=128` vẫn có ~1–4 %/boot trên
máy thứ hai (`ps`), nên **race có ở cả 128** — `MAX_IO` chỉ khuếch đại tần suất.

Đã **loại bằng đo**, mỗi kết luận kèm số:

| Giả thuyết | Phép đo | Kết quả |
|---|---|---|
| Mất lệnh ghi ATA | `ATA STAT` 128 vs 256 | 256 **ít** lệnh hơn, 0 lỗi/retry |
| Trần đệm IPC 256 B | nới `IPC_BUFFER_WORDS` 32→64 | 12/40 → 8/40 (trong nhiễu) |
| Reply-orphan `ipc.rs` | vá + đo | 9/40 **và** 128 đỏ 1/40 ⇒ revert |
| Retry `readdir` | +2 lần thử | 9/30 như cũ ⇒ revert |
| Ghi xong đọc lại | `FS LOST` | 0 lệch/12 boot |
| Commit mất target | `COM l=/v=` | **0/201** lệch |
| Cache ≠ đĩa | `read_uncached` vs `dev.read` | `mm=00` khớp |
| Inode RAM thiếu block | đọc `write_dir_entry` | cập nhật đủ `blocks/size/self.inodes` |
| Thứ tự `commit()` | đọc mã | đúng chuẩn (không xoá commit record sớm) |
| Thứ tự giữa hai client | `open` ngay sau `mkdir` | `open=1` ⇒ entry tồn tại ngay |
| Cache đường dẫn | `grep` | không có cache nào |

**Dữ kiện còn lại:** bản đồ slot tại lần "hết" cho `SLO 02 04 05 06 07 00 09 00`
— block **thiếu ino 08** dù `open("/d")` thành công ngay trước đó. Lưu ý phương
pháp: các đèn hiện ghi "lần gần nhất" và chỉ in ở FSCK, nên có thể thuộc hai thời
điểm khác nhau ⇒ **phải thêm `seq`** trước khi kết luận tiếp.

**Việc tiếp:** thêm `seq` cho ba đèn (`V2RD`/`LOOK`/`SLO`) để chúng chắc chắn cùng
một sự kiện; nếu vẫn thiếu ino 08 thì soi nhánh **tái dùng slot trống** trong
`write_dir_entry` sau `remove_dir_entry`.

### Trạng thái cuối phiên (r254 → r283)

| Chỉ số | Giá trị |
|---|---|
| `main` | **gate full PASS 10/10**; đã push lên `origin/main` |
| Kernel host test | 244 (242 chạy + 2 fuzz `#[ignore]`) |
| Harness | 632 ca âm tính PASS |
| Soak 100 @ r262 (chưa có T32) | 298/300 = 99,3% |
| Soak 100 @ r274 (có T32) | 299/300 = 99,7% |
| Soak 100 @ r276 (có T32) | 298/300 = 99,3% |
| Soak x86-only 120 @ r277 | **120/120 = 100%** (1 216 s) |
| Soak 100 @ r280 | **299/300 = 99,7%** (riscv 99, arm 100, x86 100) — lỗi duy nhất là T33b |
| Lỗi còn lại | T33a (x86 treo ~1% trước `PREEMPT OK`), T33b (riscv thoát sớm ~1%), T31 (message lặp ~1%) |
| Lỗi đã đóng trong phiên | `FS V2 OK` (giảm nhờ `MAX_IO` 128, gốc mở), `DESTROY DIAG` (T32), 3 lỗi harness (T17/T19/T33) |
| Audit K | 43 CLOSED · 3 UNREACHABLE · 0 OPEN |
| G2 Phần B | **XONG** — `df` là applet, con nhận cap VFS badge 1 |
| Việc tiếp theo | T26 (gốc race FS — có bài test hồi quy), T28 (`-smp 4`), T29 (`!!`), T30 (aarch64), T31, T33a/T33b |
| Worktree/nhánh | còn đúng `main`; `r254`, `g1`, `r261`, `r268`, `t13x` đã xoá |

Báo cáo này đã được ký lại sau mỗi lần sửa nội dung; khối SHA-256 ở cuối là của
phiên bản hiện tại.

<!-- SHA-256: phần dưới đây KHÔNG nằm trong hash -->

## SHA-256

```text
b004445e501c9ff0adf745ca52be4a05c14491c1941b674fed7bc8f350a53aa3  docs/reviews/ViFastOs-report-r262.md
```

Kiểm: băm mọi byte TRƯỚC dấu `<!-- SHA-256` (sau khi `rstrip()` và thêm
đúng một `\n`), phải ra đúng chuỗi trên. Lệnh:

```sh
python3 scripts/sign-report.py docs/reviews/ViFastOs-report-r262.md
```
