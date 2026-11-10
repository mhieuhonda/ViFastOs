# Báo cáo phân tích tệp điều chỉnh r294–r298 và tiến độ thực thi

**Gửi:** Agent 0 · **Lúc:** 2026-11-10, 17:20
**Người kiểm:** phiên soát độc lập (không sửa mã nguồn, không chạy QEMU)
**Đối chiếu:** `ViFastOs-dieu-chinh-huong-agent2-r294-r298.md` (bản bạn gửi 17:0x)
**Mốc:** HEAD `9fe5963` (r294.3 + nhật ký 294.A) + cây đang dở 294.B

> **Điều cần đọc trước tiên:** mục **4**. Bằng chứng mới cho thấy **tiền đề của
> 294.B ("tick BSP ngừng phát") không đúng** — timer vẫn chạy tới 700–850 tick
> và EOI khớp. Chữ ký thật là **root task panic** rồi cả chuỗi test không hoàn
> tất. Việc này đổi hướng điều tra nhưng **không** đổi mục tiêu của r294.

---

## 1. Phân tích tệp điều chỉnh của Agent 0

### 1.1 Bảy quyết định — bản chất và rủi ro

| Quyết định | Nội dung | Đánh giá của tôi |
|---|---|---|
| Q1 | Bảng ba mức cho lớp `tcb=None`: 0/300 ⇒ đóng (mở lại khi thấy 1 lần); 1–2 ⇒ vẫn mở, đi tiếp; ≥3 ⇒ dừng | **Đúng và có thể thi hành.** "Đóng theo số đo, mở lại ngay khi thấy 1 lần" là cách đúng để không biến "đóng" thành "quên" |
| Q2 | Ngân sách flake: G2 đóng ở **≤ 0,5 %/boot**, tạm thời **≤ 1 %/boot**; chỉ T-INFRA ngoài ngân sách | **Hợp lý.** Đây là con số dự án đang thiếu; có nó thì mọi vòng sau tự quyết được |
| Q3 | `--strict`: RETRY ≥ 2 ⇒ đỏ; = 1 ⇒ vàng (qua nhưng in + lưu); hai lượt liên tiếp cùng lớp ⇒ đỏ; chữ ký lạ ⇒ đỏ ngay | **Tốt hơn hẳn "RETRY > 0 là đỏ"** (luật cũ sẽ làm gate đỏ ~8,6 % lượt chỉ vì flake nền). Xem lưu ý 1.2 |
| Q4 | T33a là việc chính của r294; được thêm đèn chẩn đoán sau feature | **Đúng về mục tiêu, sai về tiền đề** — xem mục 4 |
| Q5 | Flake boot sớm **không** loại khỏi thống kê; tạo lớp `T-BOOT`; ưu tiên thấp | **Đúng nguyên tắc** (bootloader cũng là mã của dự án). Chi phí đèn sớm trên UEFI không rẻ — xem 1.2 |
| Q6 | Race debug-exit xử lý ở harness, lớp `T-INFRA`, không tính ngân sách | **Đúng chỗ** (không phải lỗi kernel) |
| Q7 | 291.5 không được trôi; nếu không tới r295 thì mở mã nợ `FS-291.5` | **Đúng** — đây là lỗi rò dữ liệu thật, dễ mất dấu nhất |

### 1.2 Ba chỗ cần lưu ý khi thi hành

**(a) Q3 cần trạng thái giữa các lượt gate.** Luật "hai lượt gate liên tiếp cùng
một lớp ⇒ đỏ" đòi hỏi lưu lớp của lượt trước ra file (ví dụ
`target/gate-retry-history.json`) và đọc lại ở lượt sau. Không có file đó thì
luật này không thi hành được — nên chốt ngay trong r296, đừng để "hiểu ngầm".

**(b) Q5 (T-BOOT) có chi phí thật trên UEFI.** "In một byte sớm nhất có thể"
nghe rẻ, nhưng trên x86/UEFI phải chọn: trước `ExitBootServices` (dùng
`ConOut`, cần handle UEFI còn sống) hay sau (dùng COM1 trực tiếp, nhưng lúc đó
có thể chưa có gì để in nếu paging/stack chưa sẵn). Nên chốt **ba mốc** thay vì
"một byte": `UEFI ENTRY`, `EXIT-BS OK`, `KERNEL ENTRY`. Ba mốc này đủ để khoanh
vùng chết mà không phải sửa lại sau.

**(c) Điều kiện dừng #2 (đèn làm đổi tỉ lệ flake) — số hiện tại nói "không đổi".**
Có đèn: 3/120 = 2,5 %; không đèn: 7/301 = 2,3 %. Chênh lệch nằm trong nhiễu, nên
**được phép tiếp tục**. Nhưng nên ghi con số này vào log để sau này không phải
tranh luận.

### 1.3 Đánh số vòng

Tệp điều chỉnh đổi cách đánh số (r294 = flake scheduler, r298 = tách `lib.rs`).
Điều này **đúng và cần**, vì tệp cũ để r294 = tách `lib.rs` trong khi thực tế
đội đang làm flake. Đã kiểm: commit `c2a8968` ghi "294.4" — khớp cách đánh mới.

---

## 2. Tiến độ thực thi (17:20)

| Mục | Nội dung | Trạng thái | Bằng chứng |
|---|---|---|---|
| **294.A** | Đọc kết quả 300 boot sau `c2a8968`, kết luận theo bảng Q1 | **XONG** | `RETRY 7/301 (T-FS=0, T-SCHED=7, T-KHAC=0)`, **0** `tcb=None`/`SMP PARK` ⇒ **đóng lớp** theo Q1; ghi `log.md` |
| **294.3** | Bất biến `current ↔ TCB` | **XONG, lệch có chủ đích** | `fc87374`; agent2 chứng minh chiều "Running ⇒ phải có current" **không thể** thành bất biến (3 test cũ đỏ), làm hai chiều đúng thay thế |
| **294.B** | T33a: tick BSP ngừng phát | **ĐANG LÀM** (B1–B3 đã có trong cây) | `TICK n (eoi=)`, đọc PIC ISR/IRR/IMR, đếm EOI, watchdog hart 1 — đều sau feature `diag-tick`; **chưa commit** |
| **294.C** | Lớp `T-BOOT`, `T-INFRA` + marker sớm | **CHƯA** | `grep T-BOOT/T-INFRA` trong harness = 0 |
| **294.5** | riscv64 `vifast-maint` + tách doorbell | **CHƯA** | — |
| **r295** | FS: 291.5, 291.6 phần còn lại, 291.7 | **CHƯA** | — |
| **r296–r298** | `--strict`, BLOCKER ở 256, tách `lib.rs` | **CHƯA** | — |

**Thứ tự ưu tiên bạn đặt:** 294.A > 294.3 > 294.B > 294.C > 294.5 > r295 > …
Agent 2 **đi đúng thứ tự**: A xong, 294.3 xong, đang ở B.

**Khối lượng:** 2 commit mới (`fc87374`, `9fe5963`) + cây đang dở 3 file
(`vifast-platform-x86_64`, `vifast-kernel-x86_64-uefi`, `Cargo.toml`).

### 2.1 Về 294.3 — cần bạn xác nhận

Bạn yêu cầu: *"bất biến 'TCB Running thì đúng một hart có `current` trỏ tới nó,
và ngược lại'"*. Agent 2 trả lời: **chiều thứ nhất không thể** — và đưa bằng
chứng đã có từ r251: thêm chiều đó làm ba test cũ đỏ ngay
(`selftest_passes`, `per_hart_runqueue_serves_hart2_and_hart3`,
`round69_steal_respects_affinity_and_counts_stats`), vì kernel **hợp lệ** có
TCB Running mà không hart nào giữ `current` (đó là kết luận K37–K40 từ r251).

Đã làm thay: hai chiều **đúng** —
(i) không hai hart nào cùng giữ `current` của một TCB;
(ii) `current` chỉ trỏ TCB `Running` — kèm test
`running_tcb_needs_exactly_one_current_owner` khoá cả ba ca.

**Đánh giá của tôi: agent2 đúng, và đây là loại "lệch" đáng khuyến khích** —
nó dẫn lại bằng chứng cũ thay vì làm theo lệnh một cách máy móc. Bạn nên xác
nhận trong tệp điều chỉnh sau, để lần sau không ai yêu cầu lại chiều bất khả thi.

---

## 3. Số đo hiện tại

| Chỉ số | Giá trị | Nguồn |
|---|---|---|
| 294.A (300 boot, sha `c2a8968`) | `RETRY 7/301`, **T-FS = 0**, T-SCHED = 7 | `log.md` + JSON |
| Trong đó: `tcb=None` / `SMP PARK` | **0** ⇒ đóng lớp | 294.A |
| Trong đó: treo sau `shell>` (T33a) | **6/301 ≈ 2,0 %** | 294.A |
| Trong đó: chết trước kernel (T-BOOT) | **1/301 ≈ 0,3 %** | 294.A |
| Lượt đo có đèn chẩn đoán (17:07) | **117/120**, `RETRY 3/120` (T-SCHED = 3) | JSON |
| Kernel test | 256 | `test-floor.json` |
| Sàn test | 369 test / 11 crate | `test-floor.json` |
| Harness | 647 case | tự chạy |
| Gate | 10/10 (Agent 2 báo) | — |

**Diễn giải:** tổng flake x86 hiện **2,3 %/boot**, vẫn **trên mức tạm 1 %** bạn
đặt (Q2). Nhưng thành phần đã đổi hẳn: trước đây nửa flake là `tcb=None`, nay
**0**; còn lại gần như toàn bộ là T33a.

---

## 4. Phát hiện làm đổi tiền đề của 294.B

### 4.1 Bằng chứng: timer **không** ngừng phát

Đèn `TICK n (eoi=)` do agent2 vừa thêm đã bắt được 3 lần hỏng T33a trong lượt
120 boot. Cả ba đều cho cùng một hình:

| Flake | TICK cuối | EOI | Chênh |
|---|---:|---:|---|
| run 30 | **848** | 847 | 1 |
| run 39 | **752** | 751 | 1 |
| run 101 | **784** | 783 | 1 |

⇒ PIC vẫn phát IRQ0, handler vẫn chạy, **EOI vẫn được gửi đúng** (lệch 1 là do
in trước khi EOI của tick hiện tại). **Giả thuyết "tick ngừng phát / ISR kẹt /
thiếu EOI / IMR bị mask" bị số liệu bác bỏ** — ít nhất với ba lần này.

### 4.2 Chữ ký thật: `ROOT PANIC` rồi cả chuỗi test đứng

Trong cả ba artifact (`status.txt`):

```
SMP WAIT (ticks=20, done=false fs=false fsl=false link=false sym=false
          tlb=true post=true cur0=true cur1=true rq0=1|4 rq1=0)
ROOT PANIC
```

Giải thích chuỗi sự kiện:

1. `SMP WAIT` do **runtime** in (một lần, ở tick 20 trong bản `test-exit`), khi
   các cờ test chưa đủ: `done=false`, `fs=false` ⇒ **phía client chưa chạy
   xong chuỗi FS**.
2. `ROOT PANIC` do **panic handler của root task** in (kèm tên file + số dòng),
   rồi root gọi `exit()`.
3. Sau đó timer vẫn tick tới 700–850 nhưng **không còn gì xảy ra**: không
   `PREEMPT OK`, không test nào xong, harness timeout.

### 4.3 Hai chuỗi nhân–quả có thể, và cách phân biệt

| Chuỗi | Nội dung | Nơi cần xem |
|---|---|---|
| **A. Client kẹt trước** | Client không hoàn tất chuỗi FS ⇒ root (đang chờ) hết kiên nhẫn ⇒ panic | Dòng panic của root, và trạng thái client lúc đó |
| **B. Root panic trước** | Root panic vì một syscall lỗi (`panic!("root syscall failed: {:?}")`, main.rs:603) hoặc `GetTime` lùi (main.rs:480) ⇒ không ai điều phối ⇒ client kẹt | Dòng panic (file + số dòng) |

**Cách phân biệt rẻ nhất: đọc dòng ngay sau `ROOT PANIC`** — panic handler in
`ROOT PANIC` + tên file + số dòng. Root có 9 điểm `panic!`, trong đó hai điểm
rất khác nhau về ý nghĩa:

- `main.rs:603` — `root syscall failed: <mã lỗi>` (lỗi syscall);
- `main.rs:480` — `GetTime went backwards on hart 1` (đồng hồ lùi!);
- `main.rs:849/855/873/904` — lỗi map bảng trang / module ELF.

### 4.4 Vấn đề: **đúng dòng đó đang bị vứt bỏ**

`status.txt` chỉ giữ các dòng đã lọc theo marker, nên **không** có dòng file+số
dòng của panic. Và `tail.txt` chỉ giữ **80 dòng cuối** (toàn `TICK ...`).
Trong khi `dump_timeout_artifacts` **đã nhận được toàn bộ serial** trong biến
`output` nhưng chỉ ghi 80 dòng cuối:

```python
# scripts/test-qemu.py, dump_timeout_artifacts()
with open(os.path.join(directory, "tail.txt"), "w") as handle:
    handle.write("\n".join(output.splitlines()[-80:]))
```

Trong khi đó `save_flake_artifacts` (đường của r293.2) **có** ghi
`serial.txt` — nhưng đường **timeout** thì không đi qua hàm đó.

**Đề nghị (2 dòng, làm được ngay):** trong `dump_timeout_artifacts`, ghi thêm
`serial.txt` với toàn bộ `output`. T33a là lớp flake **luôn biểu hiện bằng
timeout**, nên đây chính là đường cần.

Nếu không sửa chỗ này thì 294.B sẽ chạy thêm nhiều lượt 120–300 boot mà **vẫn
không đọc được nguyên nhân panic** — đúng kiểu "đo mà không nhìn thấy".

### 4.5 Hệ quả cho kế hoạch 294.B

| Mục | Trạng thái theo số liệu mới |
|---|---|
| B1 `TICK n` | **Đã có, đã trả lời**: timer chạy tốt |
| B2 ISR/IMR | **Đã có đèn**, nhưng giả thuyết đã bị bác; giữ lại để xác nhận lần sau, đừng dồn công |
| B3 đếm IRQ0/EOI | **Đã có, đã trả lời**: khớp |
| B4 rà nhánh thiếu EOI | **Hạ ưu tiên** — số liệu không ủng hộ |
| B5 sửa gốc + test | **Chưa thể** — chưa biết gốc; cần dòng panic trước |
| B6 đo 300 boot | **Chưa nên chạy** cho tới khi có đèn đọc được nguyên nhân |

**Đề nghị đổi trọng tâm 294.B (vẫn trong phạm vi bạn cho phép):**

1. Sửa `dump_timeout_artifacts` ghi `serial.txt` (2 dòng).
2. Thêm đèn **trạng thái hàng đợi** khi watchdog bắn: in `current[hart]`,
   `runq_len` từng hart, và với mỗi TCB trong hàng: `index/state/priority/queue_seq`.
   (Hiện chỉ có `rq0=1|4` — biết số lượng mà không biết là ai, đang chờ gì.)
3. Thêm đèn **điểm chờ IPC**: in cặp `(caller, callee)` đang `BlockedReply`/
   `BlockedRecv` khi watchdog bắn — để thấy client chờ ai.
4. Khi có `serial.txt`: đọc ngay dòng sau `ROOT PANIC` ⇒ khoanh được chuỗi A/B.

---

## 5. Những gì đã tốt, không nên đụng

- **Thước đo RETRY** (r293.1–293.3) đang làm đúng việc: nó vừa phát hiện flake
  thật, vừa giúp loại một giả thuyết sai trong 20 phút.
- **Bản vá `c2a8968`** (nhả BKL trước halt) — chấp nhận, đúng giới hạn đã ghi.
- **294.3** — xử lý lệch có chứng cứ, đúng cách.
- **Việc sửa bộ phân loại** — nếu không sửa, mọi thống kê FS sau đó sẽ sai.
- **Artifact + `source_sha256` trong JSON** — nhờ đó tôi kiểm được "cùng sha"
  mà không phải hỏi.

---

## 6. Câu hỏi / xác nhận cần Agent 0

**X1. Xác nhận 294.3.** Đồng ý với "lệch có chủ đích" (chiều "Running ⇒ phải có
current" bất khả thi, giữ hai chiều đúng) — hay vẫn muốn chiều đó, chấp nhận
sửa 3 test cũ?

**X2. Đổi trọng tâm 294.B.** Đồng ý bỏ giả thuyết tick/PIC (đã bị số liệu bác)
và chuyển sang: (a) ghi `serial.txt` khi timeout, (b) đèn hàng đợi + điểm chờ
IPC, (c) đọc dòng panic? Hay vẫn muốn làm đủ B1–B6 như tệp?

**X3. `T-BOOT` (1/301) và `ROOT PANIC`.** Hiện `ROOT PANIC` bị tính là T-SCHED.
Nhưng nếu panic handler của root là **triệu chứng** của client kẹt (chuỗi A),
thì nên tách một lớp riêng `T-ROOT-PANIC` để đếm đúng bản chất. Bạn chốt giúp.

**X4. Có cần luật "đèn không được đổi hành vi" cụ thể hơn không?** Hiện đèn
`diag-tick` nằm sau feature và chỉ in — tốt. Nhưng điều kiện dừng #2 chỉ nói
"đổi tỉ lệ rõ rệt". Đề nghị định lượng: *"nếu tỉ lệ lệch > 2 lần so với lần đo
không đèn trên cùng sha thì dừng"* — để khỏi tranh luận.

**X5. Ngân sách flake khi chạy CÓ đèn.** Các lượt đo chẩn đoán (120 boot) có
tính vào ngân sách 1 %/boot không? Đề nghị: **không** — chỉ tính lượt đo trên
bản phát hành (không đèn), vì đèn làm đổi thời gian.

---

## 7. Phụ lục

### 7.A Commit mới trong khoảng này

| Commit | Nội dung |
|---|---|
| `fc87374` | r294.3 bất biến `current ↔ TCB` (hai chiều đúng) + "lệch có chủ đích" |
| `9fe5963` | docs: nhật ký 294.A (đóng lớp `tcb=None` theo 0/300), bài học phân loại |

### 7.B Cây đang dở (chưa commit)

| File | Nội dung |
|---|---|
| `crates/vifast-platform-x86_64/src/platform.rs` | đèn `diag-tick`: `TICK n (eoi=)`, `pic_isr_imr()`, đếm EOI, watchdog hart 1, dự kiến marker `PIC STALL` |
| `crates/vifast-kernel-x86_64-uefi/Cargo.toml`, `crates/vifast-platform-x86_64/Cargo.toml` | khai báo feature `diag-tick` |

### 7.C Cách kiểm lại

```sh
cd <repo>
git log --oneline -4
python3 - <<'PY'
import json; d=json.load(open('target/vifast-qemu-last.json'))
print(d.get('repeat'), d.get('results')); print(d.get('retry',{}).get('classes'))
PY
grep "SMP WAIT" artifacts/flake-x86_64-101/status.txt | tail -1
grep -c "ROOT PANIC" artifacts/flake-x86_64-101/status.txt
grep "TICK " artifacts/flake-x86_64-101/tail.txt | tail -3
```

### 7.D Bảng đối chiếu: yêu cầu ↔ hiện trạng

| Yêu cầu (tệp điều chỉnh) | Hiện trạng | Ghi chú |
|---|---|---|
| 294.A đọc 300 boot, kết luận theo Q1 | **xong** | 0 `tcb=None` ⇒ đóng lớp |
| 294.3 bất biến `Running ↔ current` | **xong, lệch** | chiều 1 bất khả thi — cần bạn xác nhận |
| 294.B T33a | **đang làm** | tiền đề "tick ngừng" đã bị bác (mục 4) |
| 294.C T-BOOT/T-INFRA + marker sớm | chưa | `grep` = 0 |
| 294.5 riscv64 | chưa | — |
| r295 FS 291.5 | chưa | sẽ cần mã nợ `FS-291.5` nếu không kịp |
| r296 `--strict` theo Q3 | chưa | cần thêm file lịch sử giữa các lượt gate |
| r297, r298 | chưa | — |

---

*Báo cáo trước: `docs/reviews/ViFastOs-bao-cao-agent0-kiem-tra-tien-do-2026-11-10.md`
(bản 15:35). Báo cáo của đội thực thi: `docs/reviews/ViFastOs-bao-cao-agent2-r293.md`.*
