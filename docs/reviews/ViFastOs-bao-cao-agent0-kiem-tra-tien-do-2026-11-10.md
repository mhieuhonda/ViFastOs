# Báo cáo kiểm tra tiến độ thực thi r289–r294

**Gửi:** Agent 0 · **Ngày:** 2026-11-10, 13:00 (ban ngày, không phải phiên qua đêm)
**Người kiểm:** phiên soát độc lập (không sửa mã nguồn, không chạy QEMU)
**Đáp ứng:** `ViFastOs-quyet-dinh-agent2-r289-r294.md`
**Mốc kiểm:** HEAD `927ca70` (r292, 12:28) + cây làm việc đang dở r293

---

## 0. Phương pháp và giới hạn

**Đã kiểm trực tiếp trong phiên này:**

| Việc | Cách kiểm | Kết quả |
|---|---|---|
| 8 commit sau r288 | `git log` + `git show --stat` | đúng thứ tự vòng, message có mã mục |
| Host test 5 crate lõi | tự chạy `cargo test` | **xanh toàn bộ** |
| Sàn test mới (289.5) | tự chạy `python3 scripts/test-floor.py --check` | PASS, 365 test / 11 crate |
| P0-3 có thật sự được vá | đọc mã `for_each_cap`, `resolve_cap_ref`, `reparent_tcb_caps` | vá đúng **theo cách khác** kế hoạch (mục 3.2) |
| P1-6 x86 đối xứng | đọc `flush_va_asid` của port x86 | đã sửa trong mã, không chỉ trong mô hình |
| `reply_epoch` (290.4) | đọc `ipc.rs` | tăng mỗi lần Call, guard hết rỗng |
| riscv64 có dùng hộp thư chung | `grep vifast_maint` port riscv | **KHÔNG** — còn giao thức cũ |
| Flake x86 (2 lần trong giờ) | đọc `artifacts/flake-x86_64-37/` và `flake-x86_64-57/` | phát hiện mới, xem 4.1 |

**Chưa kiểm, và lý do:**

- **Gate 10/10 và soak 299/300 do Agent 2 báo** — tôi không chạy lại. Lý do: lúc
  kiểm có lượt `test-qemu.py --arch x86_64 --repeat 80` **đang chạy** (12:43);
  chạy QEMU chồng sẽ làm nhiễu cả hai phép đo. Lượt đó đã xong lúc 12:56 và
  **báo 80/80**, nhưng vẫn sinh một artifact hỏng — xem 4.1.
- **Không xác nhận được tỉ lệ hỏng thật** — cỡ mẫu 140 lần boot chỉ cho khoảng
  tin cậy 0,02 % – 4 % (4.4), chưa đủ để kết luận.

**Ngưỡng tin cậy:** mọi con số dưới đây là số **đo được**, trừ khi ghi rõ
"Agent 2 báo".

---

## 1. Trạng thái một bảng

| Vòng | Nội dung | Trạng thái | Đánh giá |
|---|---|---|---|
| **r289** | P0-1, P0-2, P1-6, BKL, sàn test, gate+soak | **XONG 5/6 mục** (riscv64 chưa nối hộp thư) | đúng hướng, chất lượng tốt |
| **r290** | Bộ duyệt chìa, P0-3, invariant, reply_epoch, fuzz | **XONG** | **lệch cách làm** so với chỉ đạo (3.2) |
| **r291** | FS P1-1…P1-5 | **3,5/7 mục** | 291.5 và 291.7 còn trống; 291.6 mới một nửa |
| **r292** | Gốc race `FS V2 OK` | **CHƯA CÓ GỐC** — mới có bằng chứng tái hiện | BLOCKER vẫn mở; và nay có thêm dữ kiện mới (4.1) |
| **r293** | Kernel/platform còn lại + dispatch + mã chết | **CHƯA BẮT ĐẦU** (cây đang dở là chẩn đoán flake) | — |
| **r294** | Tách `lib.rs` | **CHƯA** | phụ thuộc r293 xanh |

**Tổng khối lượng:** 12 commit (7 mã + 5 tài liệu), 24 file, **+2 612 / −190 dòng** trong khoảng 10:04 → 12:28
(tức khoảng 2,5 giờ). Tiến độ nhanh, nhưng xem mục 3 về các khoảng trống.

**Ba điểm cần đọc trước tiên:**

1. **P0-2 vẫn còn sống trên riscv64** (mục 3.1) — 2/3 kiến trúc đã vá, 1/3 chưa,
   và lý do chưa vá là một vướng thiết kế có thật (4.3), không phải lười.
2. **Ở `MAX_IO=128`, x86_64 vẫn flake ~1 %** — và **kiểu hỏng không phải**
   `FS V2 OK` (4.1). Nguy hiểm hơn: harness **thử lại khi hỏng**, nên lượt
   80 lần vừa rồi báo "80/80" trong khi có ít nhất một lần phải chạy lại.
   Con số PASS cuối đang **che** tỉ lệ flake thật.
3. **Tiêu chí "0/100" chỉ chứng minh được tỉ lệ hỏng < 3 %** (4.4), và nếu đo
   bằng "PASS cuối" thì retry làm nó gần như luôn xanh. Cần đổi cả *đại lượng*
   (đếm RETRY) lẫn *cỡ mẫu*.

---

## 2. Chi tiết từng vòng

### r289 — khép tầng đáy (commit `c74ed59`, `1e5c051`)

| Mục | Trạng thái | Bằng chứng |
|---|---|---|
| 289.1 soát diff đang dở | xong | `cargo test` pte + maint xanh; 3 target `cargo check` exit=0 (tôi chạy lại: vẫn xanh) |
| 289.2 payload vào hộp thư | xong (2/3 port) | `two_publishers_both_get_covered`, `publishers_with_different_asids_cover_both`, `scope_merge_never_narrows` |
| 289.3 x86 shootdown đối xứng | xong | mã x86 nay cho `target == 0`, dùng `APIC_IDS[target]`; test `ap_can_publish_to_the_bsp_mailbox` |
| 289.4 BKL `held_by` thay `is_held` | xong, 8 chỗ / 3 port | test `held_by_is_owner_only_while_a_waiter_sees_is_held` |
| 289.5 sàn test theo số đo | xong | `scripts/test-floor.json` + `test-floor.py --check` trong gate; giảm test ⇒ đỏ |
| 289.6 gate + soak | Agent 2 báo 10/10 và 299/300 | artifact `target/qemu-status-standard.json` 12:11 = 3/3 mỗi arch |

**Nhận xét:** đây là vòng chất lượng cao nhất của đợt. Cách xử lý 289.2 hay hơn
đề bài: thay vì "gộp yêu cầu chồng nhau thành flush-all", họ đưa **payload vào
hộp thư kèm khái niệm `Scope` và quy tắc hợp nhất chỉ nới**
(`scope_merge_never_narrows`) — tức là chứng minh được "không bao giờ thu hẹp
phạm vi flush", mạnh hơn yêu cầu gốc.

### r290 — tầng capability (commit `fc6aec4`, `f5c28b4`, `75ffb6e`)

| Mục | Trạng thái | Bằng chứng |
|---|---|---|
| 290.1 bộ duyệt chìa dùng chung | xong | `for_each_cap` + `TableId` + `cap_slot_capacity`; test `for_each_cap_visits_every_slot_of_every_live_table` |
| 290.2 vá P0-3 | **xong, nhưng theo cách khác** | `CapRef.cnode` + `resolve_cap_ref`; test 5 bước `orphan_cap_in_a_detached_cnode_is_reparented_and_revoked` (xem 3.2) |
| 290.3 invariant `cap.parent` mọi nhánh | xong | `check_invariants` nay duyệt qua `for_each_cap`; test `check_invariants_rejects_a_parent_pointing_at_a_dead_table` |
| 290.4 `reply_epoch` | xong | tăng ở **mỗi lần Call** (cả 2 nhánh giao ngay và qua hàng); test `every_call_issues_a_fresh_reply_epoch` |
| 290.5 đếm trùng | "không phát sinh" | — |
| 290.6 fuzz 100k × 3 định dạng | xong | bước `fuzz` của gate |

**Nhận xét:** 290.4 làm *mạnh hơn* đề bài. Đề bài nói "tăng ở mọi điểm
restart/destroy server"; họ tăng ở **mỗi lần Call**, nên guard chống reply cũ
đúng cho cả trường hợp client bị restart **và** trường hợp client gọi lần mới.
Đây là bất biến đúng và rẻ hơn.

### r291 — filesystem (commit `c59bbb8`, `ca78f58`)

| Mục | Trạng thái | Bằng chứng |
|---|---|---|
| 291.1 / P1-1 capacity theo superblock | xong | `mount_accepts_a_device_larger_than_the_formatted_area` — tôi đã tái hiện lỗi cũ trước đó, nay có test |
| 291.2 / P1-2 biên chỉ số indirect | xong | `read_with_a_forged_size_returns_corrupt_instead_of_panicking` |
| 291.3 lỗ file thưa | xong | `writing_past_eof_leaves_a_readable_zero_filled_hole` (read trả 0, fsck bỏ qua lỗ) |
| 291.4 rollback `create` | xong | `failed_create_rolls_back_the_allocated_inode` (đỏ cũ: `left 18, right 17`) |
| 291.5 unlink inode đang mở | **CHƯA** | cần bảng fd phía server — đây là **lỗi rò dữ liệu thật**, xem 3.3 |
| 291.6 fsck đọc đĩa thật | **một nửa** | `fsck_mark_block` đã theo capacity; phần "đọc lại từ đĩa" chưa |
| 291.7 persistence 7 pha | chưa chạy riêng | bước `persist` của gate vẫn xanh |

`vifast-fs`: 40 → **44 test** (tôi chạy lại: 44/44 xanh).

### r292 — BLOCKER race `FS V2 OK` (commit `927ca70`)

**Chỉ có bằng chứng tái hiện, chưa có gốc.** Đo lại trên cây HEAD: tạm đặt
`MAX_IO=256` rồi trả về 128 ngay sau khi đo:

```
python3 scripts/fs-smp-regression.py --iterations 20 --expect red
→ 3/20 lần đỏ (thiếu marker "FS V2 OK")
```

Khớp bảng đo cũ (12/49 ≈ 24 % ở 256; 0/32 ở 128). Ba nghi phạm đã xếp thứ tự
điều tra nhưng **chưa loại trừ cái nào**:

1. cache block 32 entry của FS server bị đường ghi staged (journal) ghi đè mà
   không invalidate;
2. `commit()` một-chunk-512B để lộ trạng thái trung gian cho đường đọc song song;
3. thứ tự publish/ack của hộp thư r289 khiến hart đọc thấy TLB cũ của vùng FS.

**Chưa có test host tái hiện.** Đây là điểm yếu lớn nhất của vòng: đã 2 vòng
liên tiếp (r262 → r292) mà BLOCKER vẫn chỉ được *đo*, chưa được *giải thích*.

---

## 3. Lệch so với kế hoạch

### 3.1 riscv64 không dùng hộp thư chung ⇒ P0-2 còn sống trên 1/3 kiến trúc

Đã kiểm: `grep vifast_maint crates/vifast-platform-riscv64/` = **0 kết quả**;
port này vẫn dùng `MAINT_SEQ` / `MAINT_KIND` / `MAINT_ACK` với **đúng giao thức
sai cũ**: `seq` tăng trước, `kind` ghi sau, và consumer `ACK` vô điều kiện.

Agent 2 đã nêu đúng vấn đề này (báo cáo của họ, "Lỗi mới phát hiện", dòng 79:
*"riscv64 chưa dùng hộp thư maintenance chung (nối vào thì boot 0/5)"*) và
**đặt câu hỏi cho Agent 0** — nhưng câu hỏi đó chưa được trả lời trong tệp quyết
định, nên mục này đang treo.

Nguyên nhân kỹ thuật của "boot 0/5" — xem 4.3. Đây không phải việc "nối dây",
mà là một quyết định thiết kế.

**Đánh giá mức độ:** trên riscv64, cửa sổ race vẫn nguyên. Nhưng riscv64 là
arch **duy nhất chưa từng** có triệu chứng `FS V2 OK` hỏng trong soak — nên rủi
ro thực tế thấp hơn x86, dù lỗi lý thuyết vẫn đó.

### 3.2 290.2: vá đúng, nhưng không theo cách đã chỉ đạo

**Chỉ đạo:** "Viết lại 4 chỗ duyệt độc lập (reparent, revoke, destroy/HasDependents,
check_invariants) dùng bộ duyệt ở 290.1."

**Đã làm:** thay vì hợp nhất 4 vòng lặp, họ **sửa cách biểu diễn**: `CapRef` nay
có trường `cnode` (con trỏ tới **bảng vật lý**), và thêm `resolve_cap_ref()`.
Nhờ vậy mỗi vòng lặp tự phân giải được "chìa cha nằm ở bảng nào" mà không cần
liệt kê mọi bảng.

Kiểm chứng bằng grep:

| Hàm | Dùng `for_each_cap`? | Dùng `resolve_cap_ref`? |
|---|---|---|
| `check_invariants` | **có** (1 chỗ duy nhất) | có |
| `reparent_tcb_caps` | không | có |
| `is_descendant` / `cap_ref_is_descendant` | không | có |
| `other_caps_to` (HasDependents) | không | không (vẫn tự lặp) |

**Đánh giá của tôi: cách đã làm tốt hơn đề bài, nhưng chưa xong theo chữ.**
Sửa gốc (một `CapRef` không còn mơ hồ) đúng là cách bền hơn sửa triệu chứng
(bốn vòng lặp). Nhưng hiện **vẫn còn 4 vòng lặp độc lập**, trong đó `other_caps_to`
vẫn tự viết lại quy tắc "bỏ qua TCB có CNode, quét CNode riêng" — đúng cái hình
dạng đã sinh ra lỗi đếm trùng (P2-2) trước đây. Rủi ro bảo trì còn nguyên.

**Cần Agent 0 quyết:** chấp nhận cách này (và đóng 290.2), hay yêu cầu thêm
một bước nhỏ "buộc mọi chỗ duyệt phải đi qua `for_each_cap`" (khoảng 30–50 dòng).

### 3.3 291.5 (unlink inode đang mở) — lỗi rò dữ liệu thật, đang bị hoãn

Đây là lỗi **client với tới được**: mở file A, `unlink` A, tạo file B → inode
của A được cấp lại cho B → fd cũ của A **đọc ra nội dung của B**. Không phải
lỗi lý thuyết: tôi đã xác nhận ở bản soát trước bằng thực nghiệm của mũi soát phụ.

Kế hoạch ghi "cần bảng fd ở phía FS server (`vifast-fs` không giữ fd)" — đúng,
và đó là lý do nó bị đẩy sang sau. Nhưng nó là **mục P1 duy nhất của FS chưa
được vá**, và là loại lỗi khó phát hiện về sau (không có triệu chứng, `fsck`
không thấy). Đề nghị không để trôi qua đợt này.

### 3.4 r293 và r294 chưa bắt đầu

Cây làm việc hiện có 3 file sửa, nội dung là **chẩn đoán**, không phải mục 293:

- `vifast-kernel`: thêm `live_tcb_count()` — đếm TCB còn sống (không `Exited`);
- `vifast-runtime`: thêm marker chẩn đoán `SMP NO READY (hart, ticks, exitted_current, alive)`
  in ngay trước chỗ `A::exit(1)` khi hart không còn TCB chạy được.

Tức là **đội thực thi đang truy flake "hart 0 hết việc"** — chính là họ lỗi
T33/T33a mà r288 tưởng đã đóng. Điều này khớp với phát hiện 4.1.

### 3.5 Tài liệu

- `docs/decisions/log.md` có mục r289, r290, r291 — **thiếu r292** (nhật ký
  đang nằm trong `progress-g1.md` và file đó **chưa commit**).
- `docs/audits/open-items.md` **chưa cập nhật K49** dù 290.4 yêu cầu đánh dấu
  "mở lại rồi đóng lại".

---

## 4. Phát hiện mới trong lần kiểm này

### 4.1 [Quan trọng nhất] Flake x86 ở `MAX_IO=128` vẫn còn — và **harness đang che nó bằng retry**

**Hai lần đo liên tiếp trong giờ này (tôi đọc trực tiếp artifact):**

| Lượt | Kết quả báo cáo | Artifact của lần hỏng |
|---|---|---|
| 12:39, `test-qemu.py --arch x86_64 --repeat 60` | **59/60** | `artifacts/flake-x86_64-37/` (12:35:50) |
| 12:56, `test-qemu.py --arch x86_64 --repeat 80` | **80/80** | `artifacts/flake-x86_64-57/` (12:52) |

**Lượt 60× (hỏng thật):** `status.txt` có dòng **`ROOT PANIC`** (marker bị cấm);
`tail.txt` chạy tới `FS V2 OK` (dòng 59), `SHELL READY`, `shell> ` rồi dừng —
**thiếu `PREEMPT OK`**. Tức là **không phải** race đọc/ghi của FS.

**Lượt 80× (báo 80/80 nhưng CÓ flake):** artifact `flake-x86_64-57` tồn tại,
nghĩa là lần thử thứ 57 đã hỏng và **được chạy lại**. Nội dung cuối serial:

```
USER FAULT tcb=None cause=0x0 addr=0x0
no runnable thread after fault
```

Đây là chẩn đoán rất giá trị: một **fault giả** xảy ra khi kernel nghĩ hart đó
**không có TCB hiện tại** (`tcb=None`, vector 0 = #DE trên x86, địa chỉ 0), rồi
không còn TCB nào chạy được ⇒ halt.

**Ba kết luận:**

1. **Đây không phải lỗi FS.** Cả hai lần đều thuộc họ "`current[hart]` = None
   trong khi vẫn có việc đang chạy" — tức **T33/T33a chưa đóng hẳn**, dù r288
   đã sửa `has_ready_on`. Lỗi này độc lập với BLOCKER `FS V2 OK`.
2. **Giả định "128 ⇒ 0 %" không còn đúng** — ở 128 vẫn còn ít nhất hai kiểu
   hỏng khác nguồn gốc, và chúng cũng làm gate đỏ.
3. **Số "80/80" không có nghĩa "không còn flake".** `scripts/test-qemu.py` có
   cơ chế **thử lại** (`VIFAST_RETRY_TIMEOUT`, retry khi timeout / chưa boot /
   exit / thiếu `SHELL READY`), nên một lần boot hỏng vẫn được tính là PASS nếu
   lần sau xanh. Thư mục `artifacts/flake-*` là **nơi duy nhất** còn dấu vết, và
   hiện **không ai đếm nó**.

**Ước lượng thô (không đồng nhất — xem cảnh báo dưới):** 1 lần hỏng / 140 lần
boot ≈ **0,7 %**; khoảng tin cậy 95 % là **0,02 % – 4 %**.

> ⚠ **Hai lượt đo không phải một mẫu đồng nhất.** `source_sha256` trong JSON
> đổi giữa hai lượt (`6def457b…` → `58732cb…`) — tức binary đã được build lại
> giữa 12:39 và 12:56 (cây có thêm chẩn đoán r293). Cộng dồn 60 + 80 chỉ là
> ước lượng thô, **không** phải một phép đo hợp lệ.

**Đề nghị (quan trọng hơn cả việc sửa lỗi):** đổi thước đo từ "tỉ lệ PASS cuối"
sang **"số lần phải RETRY"**. Một dòng `grep -c RETRY` trên log là đủ, và nó
phản ánh đúng tỉ lệ flake thay vì bị retry che.

### 4.2 Artifact flake không đủ để chẩn đoán nguyên nhân

`artifacts/flake-x86_64-37/` chỉ giữ **3 tệp**: `status.txt` (danh sách dòng đã
lọc), `tail.txt` (80 dòng cuối), `registers.txt` (dump thanh ghi QEMU).

Không có **serial đầy đủ**. Vì vậy không thể biết root task panic vì lý do gì
(`ROOT PANIC` chỉ là chuỗi in ra trong panic handler; thông điệp panic thật nằm
trước đó và đã bị lọc mất).

So sánh: `artifacts/soak/iter-*/` có **serial đầy đủ** cho từng arch. Tức là
harness đã có khả năng giữ serial, nhưng đường "flake của gate" thì không dùng.

**Đề nghị (rẻ, 1 dòng):** khi gate đỏ, copy serial đầy đủ vào artifact. Không có
nó thì mọi lần flake đều phải chạy lại 60–80 lần boot để bắt lại.

### 4.3 riscv64: vì sao "nối hộp thư chung vào thì boot 0/5"

Port riscv64 dùng **một** từ `MAINT_KIND` cho **hai** loại tín hiệu khác nhau:

- bit TLB (`MAINT_TLB`, `MAINT_TLB_VA`, `MAINT_ICACHE`) — cần *được làm rồi ACK*;
- bit `MAINT_RESCHEDULE` — chỉ là `doorbell` báo "có việc mới", **không cần ACK**,
  và được đọc ở nơi khác (`MAINT_KIND[hart] & MAINT_RESCHEDULE`) để quyết định
  hart phụ có thức dậy hay không.

`Mailbox::take()` của `vifast-maint` **tiêu thụ toàn bộ** từ `kind` và **trả
`None` khi kind = 0**. Nối thẳng vào thì:

- tín hiệu reschedule bị `take()` "nuốt" và ACK như thể đã xử lý ⇒ hart phụ
  không thức dậy ⇒ treo boot (đúng 0/5 như Agent 2 báo);
- hoặc nếu `take()` trả `None` cho kind chỉ có RESCHEDULE thì reschedule bị mất.

**Kết luận:** muốn thống nhất 3 port, phải **tách kênh doorbell khỏi kênh
maintenance** (hai hộp thư, hoặc một hộp thư có trường `wake` riêng không ACK).
Đây là quyết định thiết kế, không phải sửa dây — cần Agent 0 chốt.

### 4.4 [Phương pháp] Hai lỗi đo đang làm mọi kết luận về flake mất giá trị

**Lỗi đo 1 — dùng "PASS cuối" thay vì đếm flake.** `test-qemu.py` thử lại khi
timeout / chưa boot / exit / thiếu `SHELL READY`. Với tỉ lệ hỏng mỗi lần boot
là p, xác suất một *lượt* (đã có retry) hỏng là p² — nên p = 2 % cho ra
"thành công" 99,96 %. Đo bằng đại lượng này thì **không bao giờ thấy flake**,
đúng như lượt 80/80 vừa rồi vẫn có artifact hỏng.

⇒ Đại lượng đúng là **số lần RETRY / số lần boot**, không phải tỉ lệ PASS.

**Lỗi đo 2 — cỡ mẫu quá nhỏ.** Quy tắc "three": quan sát **0 lần hỏng trong N
lần** chỉ cho phép kết luận tỉ lệ thật **< 3/N** (95 %).

| Quan sát | Kết luận được (95 %) |
|---|---|
| 0/100 | tỉ lệ < **3 %** |
| 0/300 | tỉ lệ < **1 %** |
| 0/600 | tỉ lệ < **0,5 %** |
| 1 hỏng / 140 (thực tế hôm nay) | **0,02 % – 4 %** — gần như không nói được gì |

**Hai đề nghị:**

1. **Đổi đại lượng:** gate in thêm `RETRY n/N`; tiêu chí 292.4 phát biểu lại
   thành **"0 lần RETRY trên 300 lần boot"** thay vì "0/100 PASS".
2. **Đổi cỡ mẫu:** `0/300` là mức tối thiểu để nói "dưới 1 %"; muốn nói
   "dưới 0,5 %" thì phải `0/600` (khoảng 40–50 phút máy, boot ~3 s).

### 4.5 [Nhỏ] `reply_epoch` là `u8`, tăng bằng `wrapping_add`

Sau 256 lần Call, epoch quay về 0. Một record `caller` cũ còn sót có thể khớp
lại. Thực tế khó xảy ra (client chỉ có một call treo tại một thời điểm), nhưng
nếu đã coi guard này là "an toàn" thì nên ghi rõ giới hạn, hoặc dùng `u16`
(miễn phí: `Tcb` còn dư chỗ trong ngân sách tĩnh).

---

## 5. Rủi ro và thứ tự khuyến nghị

| # | Việc | Vì sao trước | Công sức |
|---|---|---|---|
| 1 | **Đếm RETRY trong gate** (in `RETRY n/N`, gate đỏ nếu > 0) | mọi số "PASS" hiện tại đang bị retry che; không có nó thì không đo được gì | ~10 dòng |
| 2 | **Đổi tiêu chí 292.4** thành "0 RETRY / 300 boot" | tiêu chí "0/100 PASS" hiện không kiểm được điều nó tuyên bố | quyết định |
| 3 | Chốt cách xử lý riscv64 (tách kênh resched) | P0-2 còn sống trên 1 arch; 2 port kia đã đổi nên càng để càng khó | 1 vòng nhỏ |
| 4 | Giữ serial đầy đủ khi gate đỏ | không có nó thì mỗi lần flake phải chạy lại 60–80 lần mới bắt lại được | ~1 dòng |
| 5 | Truy flake "`current[hart] = None` khi đang chạy" (4.1) | là họ T33 chưa đóng; độc lập với BLOCKER FS | 1 vòng |
| 6 | 291.5 (fd đang mở) | lỗi rò dữ liệu thật, chưa có cách phát hiện về sau | 1 vòng |
| 7 | r293 như kế hoạch | — | 1 vòng lớn |
| 8 | r294 tách `lib.rs` | chỉ sau khi r293 xanh | 1 vòng lớn |

**Rủi ro lớn nhất hiện tại không phải lỗi kỹ thuật mà là *cách đo*.** Hai vòng
liên tiếp (r262, r292) dồn công sức cho BLOCKER `FS V2 OK` mà vẫn chưa có gốc;
nay phát hiện thêm một nguồn flake khác **và** phát hiện chính thước đo đang
che cả hai. Nếu không sửa thước đo trước, mọi kết luận "đã hết flake" sau này
đều không đáng tin — kể cả khi chúng xanh.

---

## 6. Câu hỏi cần Agent 0 quyết

**Q1. riscv64 với hộp thư maintenance — chọn cách nào?**

| Phương án | Ưu | Nhược |
|---|---|---|
| **A. Tách kênh**: thêm trường `wake` (không ACK) vào `Mailbox`, hoặc hộp thư thứ hai cho reschedule | thống nhất cả 3 port, đúng bản chất | chạm `vifast-maint` + port riscv, cần test mới |
| B. Giữ riscv ở giao thức cũ thêm một vòng, ghi nợ | không chặn r293 | P0-2 còn sống trên 1 arch; nợ dễ quên |
| C. Cho riscv dùng hộp thư chung **chỉ cho TLB**, giữ `MAINT_RESCHEDULE` ở đường riêng | rẻ nhất, vá được P0-2 | hai cơ chế song song trong một port — dễ hiểu nhầm về sau |

*Khuyến nghị: C trước (vá P0-2 ngay, rẻ), A sau nếu muốn dọn hẳn.*

**Q2. 290.2 — đóng hay yêu cầu thêm?**

Cách `CapRef.cnode` đã sửa đúng gốc, nhưng vẫn còn 4 vòng lặp (chỉ 1 dùng
`for_each_cap`). Đóng luôn (chấp nhận rủi ro bảo trì), hay bắt buộc thêm bước
"mọi chỗ duyệt phải qua `for_each_cap`" trước r294?

**Q3. Đổi *đại lượng* đo và tách tiêu chí — có được không?**

Ba thay đổi đề nghị:

- (a) đại lượng: **đếm `RETRY`** thay vì tỉ lệ PASS (vì retry đang che flake);
- (b) cỡ mẫu: **300 lần boot** thay 100 (để nói được "dưới 1 %");
- (c) tách làm hai tiêu chí riêng: `FS V2 OK` và "không `ROOT PANIC` / không
  thiếu `PREEMPT OK`" — vì nay đã chứng minh chúng là **hai lỗi khác nhau**,
  và một bản vá cho lỗi này không làm lỗi kia đổi số.

**Q4. Flake "`current[hart] = None` khi vẫn có việc chạy" (4.1) xử lý ở đâu?**

Có **hai** biểu hiện đã bắt được trong giờ này: `ROOT PANIC` + thiếu `PREEMPT OK`
(lượt 60×), và `USER FAULT tcb=None` → `no runnable thread after fault`
(lượt 80×). Cả hai đều là họ T33 chưa đóng, **không** nằm trong 6 vòng của kế
hoạch (r289–r294 nhắm chỗ khác). Mở mục mới (r295?) hay để đội thực thi tự
quyết trong r293 — và nếu tự quyết thì có được phép dùng thêm chẩn đoán
(`SMP NO READY`) như cây đang có không?

**Q5. 291.5 (unlink inode đang mở) có được phép trôi qua đợt này không?**

Nếu không, nó cần một vòng riêng vì phải thêm bảng fd ở FS server — thay đổi
kiến trúc nhỏ, không phải bug fix thuần.

---

## 7. Phụ lục

### 7.A Commit trong đợt (từ r288)

| Commit | Giờ | Nội dung |
|---|---|---|
| `c74ed59` | 10:04 | r289 P0-1 (nG + flush gắn ASID) + P0-2 hộp thư chung |
| `1e5c051` | 10:22 | r289 payload trong hộp thư + P1-6 đối xứng + 289.4 BKL + 289.5 sàn test |
| `fc6aec4` | 10:43 | r290 (một phần) bộ duyệt chìa + `CapRef.cnode` |
| `77abe21` | 11:25 | r290 docs + soak 299/300 |
| `f5c28b4` | 11:38 | r290 290.3 invariant + 290.4 reply_epoch |
| `77e8db5` | 11:38 | r290 docs |
| `75ffb6e` | 11:50 | r290 290.2 xong — kịch bản 5 bước P0-3 |
| `c59bbb8` | 12:01 | r291 P1-1, P1-2, 291.3, fsck theo capacity |
| `13887f9` | 12:01 | r291 docs |
| `ca78f58` | 12:15 | r291 291.4 rollback inode |
| `91ab198` | 12:15 | r291 docs |
| `927ca70` | 12:28 | r292 bằng chứng tái hiện (3/20), giữ `MAX_IO=128` |

### 7.B Số đo tôi tự chạy lại (12:45)

| Crate | Test | Kết quả |
|---|---|---|
| `vifast-kernel` | 250 (+2 `#[ignore]`) | xanh |
| `vifast-fs` | 44 | xanh |
| `vifast-pte` | 12 | xanh |
| `vifast-arch` | 8 | xanh |
| `vifast-maint` | 7 | xanh |
| **Sàn** (`test-floor.py --check`) | 11 crate / **365** | **PASS**, không crate nào giảm |

### 7.C Cách kiểm lại (cho người có máy)

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cd <repo>

# 1) trạng thái
git log --oneline -15; git status --short

# 2) test host + sàn
cargo test -p vifast-kernel -p vifast-fs -p vifast-pte -p vifast-arch -p vifast-maint
python3 scripts/test-floor.py --check

# 3) lần hỏng x86 gần nhất (4.1)
cat target/vifast-qemu-last.json
sed -n '1,15p' artifacts/flake-x86_64-37/status.txt   # thấy ROOT PANIC
grep -n "FS V2 OK\|PREEMPT OK\|shell>" artifacts/flake-x86_64-37/tail.txt

# 4) riscv64 còn giao thức cũ (3.1)
grep -c vifast_maint crates/vifast-platform-riscv64/src/platform.rs   # = 0

# 5) đếm flake thật: mỗi artifact = một lần boot hỏng (đã bị retry che)
ls -d artifacts/flake-* | wc -l

# 6) đo lại x86 với mẫu đủ lớn (4.4) — chạy khi máy rảnh
python3 scripts/test-qemu.py --arch x86_64 --repeat 300
```

### 7.D Bảng đối chiếu nhanh: 6 vòng

| Vòng | Mục | Xong | Dở/chưa |
|---|---|---|---|
| r289 | 6 | 5 | riscv64 chưa nối hộp thư |
| r290 | 6 | 6 | 290.2 lệch cách làm; 290.5 "không phát sinh" |
| r291 | 7 | 3 | 291.5, 291.7; 291.6 một nửa |
| r292 | 5 | 1 (tái hiện) | 292.2 gốc, 292.3 sửa, 292.4 đo, 292.5 điều kiện |
| r293 | 10 | 0 | đang chẩn đoán flake |
| r294 | 1 | 0 | chưa bắt đầu |

---

*Hết báo cáo. Bản kỹ thuật chi tiết (số dòng, trích đoạn mã) của đợt soát gốc:
`docs/audits/2026-11-10-soat-tang-day.md`. Báo cáo tiến độ trước đó:
`docs/reviews/ViFastOs-report-agent0-2026-11-10.md`.*
