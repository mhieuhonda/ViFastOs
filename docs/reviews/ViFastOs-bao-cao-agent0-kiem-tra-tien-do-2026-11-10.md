# Báo cáo kiểm tra tiến độ thực thi r289–r294

**Gửi:** Agent 0 · **Cập nhật:** 2026-11-10, 15:35 (thay bản 13:00 cùng tên)
**Người kiểm:** phiên soát độc lập (không sửa mã nguồn, không chạy QEMU)
**Đáp ứng:** `ViFastOs-quyet-dinh-agent2-r289-r294.md` + `...-r293-r298.md`
**Mốc kiểm:** HEAD `24c1ef3`, **cây sạch** (chỉ có chính tệp báo cáo này đang sửa)

> **Phần mới so với bản 13:00** — đọc trước nếu chỉ có 2 phút:
> mục **1** (trạng thái), **4.1** (số đo flake thật 2,6 % và việc `T-FS = 0`),
> **4.2** (đã tìm ra gốc lớp "chết sớm" + bản vá), **4.3** (hướng còn lại: tick
> PIT), **6** (bảy câu cần chốt để ra tệp điều chỉnh hướng).

---

## 0. Phương pháp và giới hạn

| Việc | Cách kiểm | Kết quả |
|---|---|---|
| 25 commit sau r288 | `git log`, `git show` | đúng thứ tự, message có mã mục |
| Cây làm việc | `git status` | **sạch** — mọi thứ đã commit (15:20); chỉ tệp báo cáo này đang sửa |
| Host test 5 crate lõi | tự chạy | xanh |
| Sàn test | `test-floor.py --check` | PASS, **368 test / 11 crate** |
| Harness spec | tự chạy | **647 case** PASS |
| Số đo flake 305 boot | `target/vifast-qemu-last.json` + đọc 8 artifact | **8 RETRY / 305** (2,6 %) |
| Gốc lớp "chết sớm" | đọc `registers.txt` + diff bản vá `c2a8968` | xác nhận: halt khi đang giữ BKL |
| Phép đo sau vá | theo dõi tiến trình | **đang chạy** 300 boot (bắt đầu 15:22; lúc 15:33 ở run **80/300**, dự kiến xong ~16:00) |

**Không kiểm:** gate QEMU 3×3 và các lượt `--repeat` do đội thực thi chạy — tôi
không chạy chồng để khỏi nhiễu phép đo đang chạy. Mọi số QEMU dưới đây đọc từ
artifact/JSON, có ghi nguồn.

**Ngưỡng:** mọi con số là đo được, trừ khi ghi "Agent 2 báo".

---

## 1. Trạng thái sáu vòng (cập nhật 15:35)

| Vòng | Nội dung | Trạng thái | Ghi chú |
|---|---|---|---|
| **r289** | P0-1, P0-2, P1-6, BKL, sàn test | **5/6** | riscv64 chưa nối hộp thư — **đã chuyển thành 293.4** |
| **r290** | Bộ duyệt chìa, P0-3, invariant, reply_epoch | **6/6** | 290.2 khép lại bằng 293.5 (`other_caps_to` qua `for_each_cap`) |
| **r291** | FS P1-1…P1-5 | **3,5/7** | 291.5 (fd đang mở), 291.7 (persist riêng) còn trống |
| **r292** | Gốc race `FS V2 OK` | **chưa có gốc** | nhưng **đã đo được**: `T-FS = 0/305` ở `MAX_IO=128` (4.1) |
| **r293** | Thước đo + P2 còn lại | **6/8** | 293.4, 293.8 còn; 293.8 **đang chạy lại** |
| **r294** | Tách `lib.rs` | **chỉ mới 294.4** | phần tách module chưa bắt đầu |

**Tổng:** 25 commit, cây sạch, 368 test host, 647 case harness, gate 10/10
(Agent 2 báo).

**Ba việc đang mở theo đúng nghĩa đen:** 293.4 (riscv64), 293.8 (đo nền sau vá
— đang chạy), r294 (tách module). Cộng hai nợ cũ: 291.5 và BLOCKER FS.

---

## 2. Chi tiết các vòng đã xong

### r289 — khép tầng đáy (5/6)

P0-1 (`nG` cho AArch64 + `flush_va` gắn ASID), P0-2 (hộp thư dùng chung
`vifast-maint`, payload trong hộp thư, `Scope` + quy tắc "hợp nhất chỉ nới"),
P1-6 (x86 shootdown đối xứng hai chiều), 289.4 (`held_by(hart)` thay `is_held`,
8 chỗ / 3 port), 289.5 (sàn test theo số đo, giảm test ⇒ đỏ).

Điểm mạnh nhất: 289.2 làm **mạnh hơn** đề bài — thay vì "gộp yêu cầu chồng
nhau thành flush-all", họ chứng minh bất biến "không bao giờ thu hẹp phạm vi
flush" (`scope_merge_never_narrows`).

### r290 — capability (6/6)

`for_each_cap` + `TableId` + `CapRef.cnode` + `resolve_cap_ref`;
`check_invariants` kiểm `cap.parent` ở mọi nhánh; `reply_epoch` tăng mỗi Call.

Hai điểm đáng ghi nhận:
- 290.2 **lệch cách làm** so với chỉ đạo (sửa cách biểu diễn thay vì hợp nhất
  4 vòng lặp). Đánh giá của tôi: cách này đúng gốc hơn. Đến 293.5 thì
  `other_caps_to` cũng đã đi qua `for_each_cap` ⇒ **lệch đã được khép**, hiện
  chỉ còn 3 chỗ `iter().flatten()` và cả 3 là "view của một đối tượng".
- 290.4 làm mạnh hơn đề: tăng epoch **mỗi lần Call** thay vì chỉ khi restart.

### r291 — filesystem (3,5/7)

Xong: P1-1 (capacity theo superblock — tôi đã tự tái hiện lỗi panic trước đó),
P1-2 (biên chỉ số gián tiếp ⇒ `Corrupt` thay vì panic), P1-3 (lỗ file thưa hợp
lệ), P1-4 (rollback inode khi `create` lỗi), 291.6 một nửa (`fsck` theo capacity).

Còn: **291.5** (unlink inode đang mở → fd cũ đọc ra file khác — rò dữ liệu thật),
**291.7** (chạy riêng persistence 7 pha).

### r292 — BLOCKER `FS V2 OK`: **khung kết luận đã đổi**

r292 không tìm được gốc, nhưng phép đo nền 305 boot (r293.8) đã trả lời được
câu hỏi quan trọng hơn: **ở `MAX_IO=128`, `T-FS = 0/305`** — BLOCKER **không**
tái phát trong mẫu này. Nghĩa là:

- "hạ `MAX_IO` để xanh" hoá ra **không che một lỗi FS đang hoạt động** ở mức
  1 %; nó chỉ làm hẹp cửa sổ race như đã ghi;
- flake còn lại (2,6 %) **không phải FS** — xem 4.1.

**Nhưng:** điều này *không* chứng minh race FS đã hết; nó chỉ chứng minh tỉ lệ
tái phát ở 128 **< 1 %** (3/305, quy tắc three). Muốn nói "đã hết" thì phải
nâng lại 256 và đo — đúng điều kiện 292.4 đặt ra.

---

## 3. Chi tiết r293 — thước đo và các mục P2

| Mục | Trạng thái | Bằng chứng |
|---|---|---|
| 293.1 đếm RETRY + phân loại | **xong** | `RETRY n/N (T-FS, T-SCHED, T-KHAC)`; JSON có khối `retry` + `source_sha256` |
| 293.2 lưu artifact có serial | **xong** | mỗi lần hỏng có `serial.txt`/`status.txt`/`tail.txt` — **nhờ nó mới tìm ra gốc 4.2** |
| 293.3 `--strict` | **xong, chưa bật** | có trong spec; gate bật từ r296 |
| 293.4 riscv64 + tách doorbell | **chưa** | — |
| 293.5 `other_caps_to` qua `for_each_cap` | **xong** | test `other_caps_to_sees_caps_inside_a_detached_cnode` |
| 293.6 `reply_epoch` u16 + canary TCB | **xong** | TCB = 1648 B; test 260 lần Call không trùng |
| 293.7 nhật ký + open-items | **một phần** | K49 "mở lại rồi đóng"; mục 300 boot chờ 293.8 |
| 293.8 đo nền 300 boot | **xong lần 1, đang đo lại** | 8/305; bản đo lại sau vá đang chạy |

### 3.1 Lỗi trong chính bộ phân loại (bài học đo lường)

Lượt đầu in `T-FS=4` — **sai**. Nguyên nhân: mọi lần guest **chết sớm** đều
thiếu marker `FS V2 OK` (vì chưa tới đó), nên bị dán nhãn "FS". Đã sửa: xét
chữ ký scheduler **trước**, và chỉ coi là T-FS khi guest **đã tới `SHELL READY`**.

Đây là bài học đáng giữ: **một bộ phân loại sai làm lệch cả hướng điều tra**
— nếu không phát hiện, cả đội sẽ đi tìm race FS trong khi lỗi thật là scheduler.

---

## 4. Số đo mới và phát hiện mới (phần quan trọng nhất)

### 4.1 [Số đo] Flake thật ở `MAX_IO=128`: **8/305 = 2,6 %**, và **không phải FS**

Điều kiện: guard `KERNEL FAULT` TẮT, `MAX_IO=128`, x86_64, một sha binary.

```
RETRY 8/305 (T-FS=0, T-SCHED=7, T-KHAC=1)
x86_64   297/300 PASS
```

| Nhóm | Số lần | Run | Chữ ký |
|---|---:|---|---|
| T-SCHED "T33a" — treo sau `shell>` | 3 | 36, 192, 287 | serial dừng ngay sau `shell>`, thiếu `PREEMPT OK` |
| T-SCHED "chết sớm" — `tcb=None` | 4 | 102, 108, 212, 217 | `USER FAULT tcb=None` + `no runnable thread after fault` |
| T-KHAC — race debug-exit | 1 | 122 | guest in đủ marker, QEMU thoát mã lạ |

**Ba kết luận cho agent0:**

1. **`T-FS = 0/305`** ⇒ BLOCKER FS **không tái phát** ở 128. Khung "BLOCKER đang
   hoạt động" phải sửa lại: nó đang **ngủ**, không phải đang cắn.
2. **Nhưng flake thật vẫn 2,6 %**, chỉ là **khác nguồn**. Trước đây con số
   "299/300" và "80/80" bị **retry che** — nay đã lộ ra nhờ thước đo mới
   (r293.1–293.3). Đây là giá trị lớn nhất của r293, không phải mấy dòng code.
3. **Hai nhóm còn lại đều thuộc scheduler/tick**, không liên quan bộ nhớ hay FS.

### 4.2 [Đã có gốc + đã vá] Lớp "chết sớm": `halt()` khi đang giữ BKL

**Bằng chứng từ artifact** (`registers.txt` của run 102/108):

| Hart | Trạng thái |
|---|---|
| CPU#0 | `CPL=0` (kernel), `IF=0`, `HLT=0` — quay trong `loop { hlt }`? |
| CPU#1 | `CPL=0`, `IF=0`, `HLT=1` — cũng trong kernel, đã halt |

**Cơ chế:** `fault::suspend` chạy trong trap handler — **đang giữ khoá BKL và
đã tắt ngắt (`IF=0`)**. Khi không còn TCB nào chạy được, nó gọi `A::halt()` =
`loop { hlt }`. Guard BKL **không bao giờ được nhả**. Hart còn lại quay khoá
BKL mãi mãi, với ngắt đã tắt ⇒ không có timer, không có IPI ⇒ **cả máy chết
câm**, không in thêm được gì.

**Bản vá** (`c2a8968`): `fault::suspend -> bool`; bốn call site **nhả BKL rồi
mới `halt()`**, thêm marker `SMP PARK (hart, cause)`. Hành vi cuối vẫn là halt
(đúng chốt của Agent 0), chỉ đổi **thứ tự**.

**Đánh giá của tôi:** đây là **sửa đúng**, và là loại sửa có giá trị kép —
(a) hết livelock câm, hart còn lại không bị treo oan; (b) **sự cố trở nên nhìn
thấy được** (`SMP PARK`), thay vì "máy im lặng". Nhưng cần nói rõ: nó sửa
**đường chết**, chưa sửa **lý do chết** — vẫn còn câu hỏi "vì sao hart có
`tcb=None` trong khi vẫn đang chạy".

**Đo sau vá:** 60 boot ⇒ `RETRY 1/61`, **0 lần `tcb=None`**; lần hỏng duy nhất là
flake **boot sớm** (thiếu `SELFTEST PASS`, serial rỗng).

**Cảnh báo thống kê:** 0/60 **chưa đủ** để đóng lớp này. Tỉ lệ trước vá là
1,3 %/boot; xác suất thấy 0 lần trong 60 boot dù lỗi còn nguyên là **45 %**.
Cần 300 boot (0/300 ⇒ tin được "dưới 1 %"). **Phép đo 300 đang chạy lúc viết
báo cáo**: bắt đầu 15:22, lúc 15:33 đã tới run **80/300**, dự kiến xong ~16:00,
cùng một sha với bản vá `c2a8968`.

### 4.3 [Còn mở] Nhóm T33a: nguồn tick của BSP **ngừng phát**

**Bằng chứng từ artifact run 36** (lần hỏng đầu tiên có serial đầy đủ):

| Hart | Trạng thái lúc đóng băng |
|---|---|
| CPU#0 | **`CPL=3`** (user), `RIP=0x1000206898`, `RFL=0x246` ⇒ **`IF=1`**, `CR2=0x1001003000` |
| CPU#1 | `CPL=0`, **`HLT=1`** (idle-halt), `IF=1` |

Serial dừng ngay sau `shell>`; **không** in `SMP WAIT` (chỉ in khi tick ≥ 99),
**không** `SMP NO READY`, **không** `KERNEL FAULT`.

**Suy luận:** hart 0 **không** kẹt trong kernel (nó ở user, ngắt bật). Vậy
không phải nó chết — **nguồn tick đã ngừng phát** trước khi tick tới 99. Loại
trừ được: (a) kẹt BKL/console (hart 0 ở CPL=3); (b) nhánh "thoát im lặng"
`pick_on == None` (không có marker chẩn đoán).

**Giả thuyết còn lại (theo thứ tự tôi cho là khả dĩ):**

1. **Bit in-service của IRQ0 trong PIC 8259 bị kẹt** — một nhánh xử lý timer
   không gửi EOI (hoặc EOI gửi sai thứ tự), nên PIC không bao giờ phát IRQ0
   tiếp. Đây là giả thuyết khớp nhất với "IF=1 mà không có tick".
2. **PIT bị nạp lại hỏng** ở một nhánh (reload sai giá trị ⇒ chu kỳ vô hạn).
3. Mask IRQ0 bị bật lại ở một nhánh (IMR bit 0 = 1).

**Cách kiểm rẻ nhất (đề nghị cho r295):**

- In `TICK n` mỗi 16 tick (sau `test-exit`) để biết **tick dừng ở con số nào**
  — nếu dừng đúng bội số của một sự kiện thì khoanh được nhánh.
- Đọc **PIC ISR (port 0x20)** và **IMR (port 0x21)** ngay trước khi dừng; nếu
  ISR bit 0 = 1 mà không có IRQ mới ⇒ xác nhận giả thuyết 1; nếu IMR bit 0 = 1
  ⇒ giả thuyết 3.
- Đếm số lần gửi EOI ở mọi nhánh (`ack_timer`) và so với số lần nhận IRQ0.

### 4.4 [Mới nổi] Flake boot sớm: serial rỗng, thiếu `SELFTEST PASS`

Trong 60 boot sau vá có 1 lần như vậy; artifact `flake-x86_64-6` cho thấy
`status.txt`/`tail.txt` **rỗng** (chỉ 1 byte) nhưng `registers.txt` đầy đủ:
CPU#0 ở **`CPL=0`**, `RIP=0x0ef611b8` (vùng thấp, kernel/UEFI), `CR3=0xfc01000`.

Nghĩa là máy chết **trước khi kernel in dòng đầu tiên**. Đây nhiều khả năng là
flake **UEFI/boot sớm**, không phải lỗi kernel — nhưng chưa đủ dữ kiện để kết
luận. Cần ít nhất: giữ serial đầy đủ (đã có) + biết QEMU có timeout không.

### 4.5 [Nhỏ] `T-KHAC` race debug-exit (1/305)

Guest in **đủ marker** rồi QEMU thoát mã lạ. Đây là race giữa `isa-debug-exit`
và đường `exit` của guest, không phải lỗi kernel. Retry cứu được.

### 4.6 [Phương pháp] Ba điều phải giữ khi đọc mọi số flake từ nay

1. **Luôn hỏi "mẫu bao nhiêu"**: 0/N chỉ nói được "dưới 3/N".
   | Quan sát | Kết luận được (95 %) |
   |---|---|
   | 0/60 | < 5 % |
   | 0/300 | < 1 % |
   | 0/600 | < 0,5 % |
2. **Luôn hỏi "cùng một sha không"**: JSON có `source_sha256`; cộng dồn hai
   sha khác nhau là vô nghĩa.
3. **Luôn phân loại trước khi kết luận**: vụ `T-FS=4` cho thấy nhãn sai làm
   lệch cả hướng điều tra.

### 4.7 Hệ quả cho `--strict` (r296)

`--strict` đặt `RETRY > 0 ⇒ đỏ`. Nếu flake còn ~1 %/boot và gate chạy
**3 arch × 3 lần = 9 boot**, xác suất gate đỏ vì flake là:

```
1 − (0,99)^9 ≈ 8,6 %
```

Tức **khoảng 1/12 lượt gate sẽ đỏ dù không ai sửa gì**. Đây không phải lỗi kỹ
thuật mà là **vấn đề chính sách**: cần ngưỡng dung sai (ví dụ `--strict` chỉ
đỏ khi RETRY ≥ 2, hoặc chỉ tính x86_64), nếu không gate sẽ mất giá trị tín hiệu.

---

## 5. Rủi ro và thứ tự khuyến nghị

| # | Việc | Vì sao | Công sức |
|---|---|---|---|
| 1 | **Chốt ngưỡng đóng lớp "chết sớm"** bằng phép đo 300 đang chạy | 0/60 chưa đủ; 0/300 mới nói được "< 1 %" | quyết định |
| 2 | **Chốt chính sách `--strict`** trước r296 | nếu không, gate sẽ đỏ ~8,6 % lượt do flake nền | quyết định |
| 3 | **T33a: đèn `TICK n` + đọc PIC ISR/IMR** | đây là nguồn flake lớn nhất còn lại (3/305) | ~1 vòng nhỏ |
| 4 | 293.4 riscv64 (tách doorbell) | P0-2 còn sống trên 1/3 arch; rẻ | 1 vòng nhỏ |
| 5 | 291.5 (fd đang mở) | rò dữ liệu thật, không có triệu chứng | 1 vòng |
| 6 | r294 tách `lib.rs` | nên làm khi flake đã có ngân sách rõ ràng | 1 vòng lớn |

**Nhận định tổng:** chất lượng thực thi cao — mọi mục đều có test đỏ-trước,
artifact, và số đo; chỗ lệch đều có lý do được ghi. Vấn đề còn lại **không
phải năng lực kỹ thuật mà là chính sách đo**: dự án đang có flake nền ~1–2,6 %
và chưa có quyết định "bao nhiêu flake là chấp nhận được". Không có con số đó
thì mọi vòng sau đều có nguy cơ tranh luận vô tận giữa "gate đỏ do flake" và
"gate đỏ do hồi quy".

---

## 6. Bảy câu cần Agent 0 chốt (để ra tệp điều chỉnh hướng)

**Q1. Ngưỡng đóng lớp "chết sớm" (`tcb=None`)?**
Phép đo 300 đang chạy. Nếu ra `0/300` ⇒ đóng được chưa? Nếu ra 1–2 lần ⇒ vẫn
coi là mở và đi tiếp, hay dừng để truy?

**Q2. Chính sách flake nền — chấp nhận bao nhiêu?**
Đề nghị một con số chính thức, ví dụ: *"G2 chấp nhận ≤ 0,5 %/boot; trên mức đó
là lỗi phải sửa"*. Có con số này thì mọi vòng sau tự quyết được mà không hỏi.

**Q3. `--strict` ở r296 — ngưỡng nào?**

| Phương án | Gate đỏ khi | Hệ quả với flake 1 % |
|---|---|---|
| A. `RETRY > 0` | bất kỳ retry | đỏ ~8,6 % lượt — mất tín hiệu |
| **B. `RETRY ≥ 2`** | từ 2 lần | đỏ ~0,4 % lượt — còn tín hiệu |
| C. chỉ x86_64 | arch hay flake nhất | bỏ sót arch khác |

*Khuyến nghị: B, kèm in đủ số để vẫn thấy flake.*

**Q4. T33a (tick PIT ngừng) — ưu tiên nào?**
Đề nghị làm ngay sau khi chốt Q1/Q2, vì đây là nguồn flake lớn nhất còn lại và
đã có hướng kiểm rẻ (đèn `TICK n` + đọc PIC ISR/IMR). Có được phép thêm đèn
chẩn đoán sau feature như r293 đã làm không?

**Q5. Flake boot sớm (serial rỗng, thiếu `SELFTEST PASS`) — thuộc phạm vi?**
Nghi là UEFI/boot sớm, không phải kernel. Loại khỏi thống kê (kèm lý do) hay
vẫn tính là lỗi phải truy?

**Q6. `T-KHAC` race debug-exit (1/305) — xử lý hay bỏ qua?**
Nếu bỏ qua, nên ghi vào `open-items.md` là "đã biết, ngoài phạm vi kernel".

**Q7. 291.5 (unlink inode đang mở) — có được trôi tiếp không?**
Đây là mục P1 duy nhất của FS chưa vá và là lỗi rò dữ liệu thật. Nếu không làm
trong đợt này thì nên ghi thành nợ có mã riêng để không mất dấu.

---

## 7. Phụ lục

### 7.A Commit trong đợt (r288 → nay, 25 commit)

| Commit | Nội dung |
|---|---|
| `c74ed59` | r289 P0-1 (nG + ASID) + P0-2 hộp thư chung |
| `1e5c051` | r289 payload trong hộp thư + P1-6 đối xứng + 289.4 + 289.5 |
| `fc6aec4` | r290 `for_each_cap` + `CapRef.cnode` |
| `77abe21`, `77e8db5` | r290 docs |
| `f5c28b4` | r290 290.3 invariant + 290.4 epoch |
| `75ffb6e` | r290 290.2 kịch bản 5 bước P0-3 |
| `c59bbb8`, `ca78f58`, `13887f9`, `91ab198` | r291 FS (P1-1…P1-4) |
| `927ca70` | r292 bằng chứng tái hiện (3/20 ở 256) |
| `9c6d29f`, `f884896`, `ad20369`, `29d16cf` | r293 chẩn đoán T33a/T33b + docs |
| `9a9a1ae` | r293 đèn chẩn đoán ra sau feature |
| `2a04bd2` | r293.1–293.3 thước đo (RETRY, artifact, `--strict`) |
| `2380cc5` | r293.5 + 293.6 (`other_caps_to`, epoch u16) |
| `50133fa` | r293.7 nhật ký + open-items |
| `39589cb` | theo chốt Agent 0: `KERNEL FAULT` guard về `halt()` mặc định |
| `4e6cc95` | manh mối T33a từ run 36/300 |
| `c2a8968` | **r294.4 nhả BKL trước khi halt** |
| `941109d`, `24c1ef3` | docs số đo nền + r294.4 |

### 7.B Số đo tổng hợp

| Chỉ số | Giá trị | Nguồn |
|---|---|---|
| `vifast-kernel` host test | 255 (+2 fuzz `#[ignore]`) | floor.json |
| Tổng test host (11 crate) | **368** | `test-floor.json`, `--check` PASS |
| Harness case âm | **647** | tự chạy |
| Gate (chế độ thường) | 10/10 | Agent 2 báo |
| Flake nền x86 (128, trước vá) | **8/305 = 2,6 %** (`T-FS=0`) | JSON + artifact |
| Flake nền x86 (sau vá, 60 boot) | 1/61, `0` lần `tcb=None` | Agent 2 báo |
| Flake nền x86 (sau vá, 300 boot) | **đang chạy** (từ 15:22) | — |
| TCB size | 1648 B | canary |

### 7.C Cách kiểm lại

```sh
export PATH="$HOME/.cargo/bin:$PATH"; cd <repo>

git log --oneline -8; git status --short          # cây phải sạch
python3 scripts/test-floor.py --check             # 11 crate / 368 test
python3 scripts/test-harness-spec.py              # 647 case

python3 - <<'PY'
import json; d=json.load(open('target/vifast-qemu-last.json'))
print(d.get('repeat'), d.get('results')); print(d.get('retry'))
PY

ls -d artifacts/flake-* | wc -l                   # số lần boot hỏng đã bắt được
tail -3 artifacts/flake-x86_64-212/tail.txt       # chữ ký "chết sớm"
grep -E "CPL=|HLT=" artifacts/flake-x86_64-212/registers.txt | head -4
```

### 7.D Bảng đối chiếu nhanh: còn gì phải làm

| Hạng mục | Trạng thái | Chặn bởi |
|---|---|---|
| 293.4 riscv64 hộp thư + doorbell | chưa làm | quyết định thứ tự (Q4) |
| 293.8 đo nền sau vá | **đang chạy** | — |
| r294 tách `lib.rs` | chưa | chính sách flake (Q2/Q3) |
| 291.5 fd đang mở | chưa | quyết định (Q7) |
| 291.7 persistence riêng | chưa | — |
| T33a tick PIT | chưa có gốc | hướng điều tra + đèn (Q4) |
| Flake boot sớm | chưa phân loại | quyết định (Q5) |
| BLOCKER FS | **ngủ** (`T-FS=0/305` ở 128) | chỉ mở lại khi nâng 256 |

---

*Bản kỹ thuật của đợt soát gốc: `docs/audits/2026-11-10-soat-tang-day.md`.
Báo cáo của đội thực thi: `docs/reviews/ViFastOs-bao-cao-agent2-r293.md`.*
