# Bàn giao Agent 2 — đêm 11→12/11/2026 (r296)

**Từ:** phiên điều phối (agent thi hành) · **Cho:** Agent 2 (thi hành đêm)
**Thời điểm:** 15:05 JST, 11/11/2026 · **HEAD:** `2d6e2a8` (14:59), `main` **ahead 1** so với `origin/main`
**Cây:** sạch (chỉ còn `docs/reviews/ViFastOs-bao-cao-agent0-r295-2026-11-11.md` chưa commit)
**Build:** OK · **QEMU:** không có tiến trình nào đang chạy · **`MAX_IO` = 128**

---

## 0. Đọc gì trước (theo thứ tự, ~15 phút)

1. `docs/reviews/progress-g1.md` — mục **`## r295 — T26: câu hỏi "FS hay IPC" ĐÃ CÓ ĐÁP ÁN`** (dòng ~2521). Đây là nguồn sự thật của vòng trước.
2. `docs/reviews/ViFastOs-bao-cao-agent0-r295-2026-11-11.md` — mục **4.0 ĐÍNH CHÍNH r295.1** (ba kiểm 0-boot đã xong).
3. `docs/audits/open-items.md` — `K32..K75: 43 CLOSED · 3 UNREACHABLE · 0 OPEN`.
4. `scripts/stress-fs-measure.py` — cách đo T26 (script **không** rebuild).

---

## 1. Trạng thái thật (đừng suy lại từ số cũ)

| Hạng mục | Trạng thái |
|---|---|
| **T26** | **Đã trả lời "FS hay IPC" ⇒ FS** (đáp án 4.D). Cơ chế **chưa xong** |
| Bằng chứng quyết định | Ring của **chính server** ghi `srv=111 req=50 op=8199 idx=6 res=1` — khớp đúng `VEND seq=50 idx=6` của client ⇒ **ENOENT do FS tạo**, không phải reply ghép lệch |
| Bằng chứng phụ | `V2MK open=1` (mkdir xong `open("/d")` thấy) + `V2CHK open=1 rd=2` (hai lần liệt kê sau đều thấy `d`) ⇒ entry **có thật**, chỉ một lần liệt kê ở giữa không thấy |
| Đèn reply/tham số/payload | `VSEQ=VARG=VHASH=0` trên toàn bộ các lượt ⇒ **đã bác** reply cũ / tham số cũ / payload bị đè |
| Vòng lặp trong 1 boot | **61.000 vòng, 0 miss** ⇒ lỗi **không** tái hiện bằng lặp đơn thuần; cần **hoạt động đồng thời của root lúc boot** |
| Tỉ lệ | **10/50 boot đỏ = 20 %** ở `MAX_IO=256`, `smp2` |
| `-smp 1` | 0/3 (mẫu nhỏ) — và `-smp 1` **không thoát được boot** ⇒ phải đặt `VIFAST_TIMEOUT` thấp nếu chạy lại |
| `tcg,thread=single` | **1/3 đỏ** ⇒ **race logic**, không phải race bộ nhớ host |
| `MAX_IO` | **giữ 128**. 256 chỉ dùng tạm khi đo; **không** bật ở gate/phát hành |

**Ba kiểm 0-boot Agent 0 yêu cầu — ĐÃ LÀM XONG, đừng làm lại:**

| # | Câu hỏi | Kết luận |
|---|---|---|
| 1 | `fsck=0` là sạch hay hỏng? | **SẠCH.** `vfs_fsck()` trả `outcome.is_ok()`; nhãn in `'0'+u8::from(!fsck)`. Còn `v2=1` nghĩa là **v2 SAI** |
| 2 | `vend 4 vs 2` có phải bộ dò? | **KHÔNG.** `client.rs:1627` `if group4 == 3` chạy thêm **2 vòng `readdir("/")`** ⇒ +2 ENOENT. Tương quan bị **rút** |
| 3 | `req=0` trong `VRING` là ai? | **root/pager.** `pager_vfs_call` gửi label thô, không `seq`; `8194 = 0x2002 = READ` |

⚠️ **Hai điều chỉnh so với ghi chú của Agent 0:**
- Máy nội bộ **cũng chạy x86_64** (TCG, `TSC HZ≈1,0 GHz`) — không phải "khác arch". Khác biệt là **tốc độ/thời điểm** ⇒ càng ủng hộ giả thuyết đồng thời.
- Ba lượt đo cũ có **ba `BUILD_ID` khác nhau** ⇒ muốn so sánh chéo máy phải **build một lần, chạy cả hai nơi**.

---

## 2. Tám quyết định (Agent 0 đã chốt)

```
D-01 A   giữ gate mềm, --strict chỉ dùng khi đo
D-02 A   đóng băng tính năng, chỉ sửa + đo
D-03 B   T26: đánh "ai/cái gì chen giữa", KHÔNG đánh nhánh fsck (đã đóng)
D-04 A   đo ở MAX_IO=256 (chỉ nhánh đo; không bật ở gate/phát hành)
D-05 A   teardown tối đa 2 chỗ, mỗi chỗ một vòng riêng, hồi quy ⇒ revert
D-06 A   soak nền ~600–1000 boot x86_64 trên máy ps
D-07 A   không đụng nợ kiến trúc (không tách lib.rs)
D-08 A   không đụng đồ hoạ / ViFastKey
```

---

## 3. Việc đêm nay — theo thứ tự

### 3.1 T26 (trọng tâm) — cơ chế trong FS

Vòng trước đã chỉ ra: **vi phạm read-your-own-write ở mức entry thư mục**. Việc của r296:

1. **Đèn `WRITE_THEN_READ`** đặt **bên trong** `write_dir_entry` và `remove_dir_entry` (thư viện `crates/vifast-fs`): ngay sau khi ghi entry, đọc lại **RAW** (`read_uncached`) và so — **có `seq`** trong bản ghi.
   * Vì sao RAW: cache có thể che đúng/sai; `read_uncached` là nhân chứng độc lập.
   * Vì sao **bên trong**: đèn "lần gần nhất" ở client (`SLO`/`LOOK`) **không có `seq`** nên không được dùng để khẳng định hai lần đọc cùng thời điểm — bài học đã ghi.
2. **Lượt S1b** (chưa từng chạy): cho vòng lặp `stress-fs` chạy **song song** với worker exec của root. Nếu lỗi tái hiện trong vòng lặp ⇒ có reproducer mạnh, không cần boot.
3. Chỉ khi có cơ chế: **test đỏ-trước trên host** (hai client xen đúng thứ tự ring cho thấy), rồi mới vá.
4. **Không** làm hàng rào bền vững kiểu "trả Ok sau khi xuống đĩa" trước khi hiểu cơ chế — dễ che lỗi khác.

Cách đo (script **không** rebuild):
```bash
VIFAST_CLIENT_FEATURES=stress-fs ./scripts/build.sh
python3 scripts/stress-fs-measure.py --label S5 --iterations 20
```
Nếu cần đo ở 256: sửa `MAX_IO` **tạm**, đo, rồi **trả về 128** trước khi commit.

### 3.2 Săn lớp hiếm (chạy nền trên `ps`, không tranh CPU)

~600–1000 boot x86_64, `VIFAST_RETRY_TIMEOUT=0`, `VIFAST_KEEP_SERIAL=1`:
- `BIN EXEC FAIL spawn` vs `wait err=` (đèn đã có, chưa gặp ca nào);
- đếm `USER FAULT … addr=0x1001001000` cho **T31** (2 ⇒ replay phía server; 3 ⇒ phía client);
- `T-ROOT` (`TIME MONOTONIC BAD` + `main.rs:480`), `T-SCHED` (treo thiếu `PREEMPT OK`).

### 3.3 Teardown (D-05) — chỉ sau 3.1 hoặc sau mốc 4:30

Tối đa **2 chỗ**, mỗi chỗ **một vòng riêng**:
- Bắt đầu bằng pager server (vùng 3066–3145).
- Mỗi lần: 3 arch × 3 boot; hồi quy ⇒ **revert ngay**, **không** thử lại ý đó trong đêm.
- **Cấm** gộp nhiều chỗ: `reap_task` gộp đã hỏng **0/3 cả ba arch**.

---

## 4. Lịch (mốc tính từ lúc bắt đầu)

| Mốc | Việc | Cổng |
|---|---|---|
| 0:00–0:25 | `git status` sạch; commit file báo cáo còn treo; build; gate 10/10; 3×3 QEMU | gate xanh + `RETRY 0/9` |
| 0:25–0:30 | Bật soak nền trên `ps` | chạy, log riêng |
| 0:30–2:15 | **Đèn `WRITE_THEN_READ`** trong `write_dir_entry`/`remove_dir_entry` + đo | có/không có vi phạm, kèm `seq` |
| 2:15–3:15 | **S1b**: vòng lặp + exec song song | tái hiện được trong vòng lặp? |
| 3:15–3:45 | Nếu có cơ chế: test đỏ-trước host → vá → đo 20 boot | tối đa **3 vòng**; hồi quy ⇒ revert |
| 3:45–4:15 | Đọc soak nền, phân loại, đếm `USER FAULT` | bảng có số |
| 4:15–5:45 | Teardown (D-05) | 3/3 xanh cả 3 arch mới giữ |
| 5:45–6:30 | Đo mở rộng / CI | mỗi số kèm N, máy, `BUILD_ID` |
| **6:30** | **ĐÓNG BĂNG CODE** | chỉ viết tài liệu |
| 6:30–7:15 | Viết `progress-g1.md` + báo cáo + bảng số | không sửa mã |
| 7:15–7:45 | Gate 10/10 + 3×3 trên HEAD cuối; ký SHA-256 | xanh, hoặc ghi rõ đỏ vì lớp nào |
| 7:45–8:00 | "3 việc tiếp theo" + "cần quyết gì" | bàn giao |

---

## 5. Luật tự quyết

**Được:** thêm đèn/marker chẩn đoán (không đổi hành vi); revert bản vá của mình khi hồi quy (D18); đổi feature chỉ-dùng-đo; chạy/soak dài; commit tài liệu.

**Cấm:** nới ngưỡng hoặc sửa test/`accept()` cho xanh; bật `--strict` mặc định; bật `MAX_IO=256` ở gate/phát hành; nâng `MAX_TCBS`/`MAX_OBJECTS`/`KERNEL_STATIC_BUDGET`; gộp nhiều chỗ teardown; xoá marker đang dùng để đo; sửa `vifast-fs` bằng khoá thô "chặn hết" mà không có test đỏ-trước.

---

## 6. Điều kiện dừng

| Tình huống | Hành động |
|---|---|
| Bản vá hồi quy 3 arch | revert ngay, ghi nhật ký, **không** thử lại ý đó |
| **4:30** mà T26 chưa có cơ chế | dừng đọc code, chuyển đo mở rộng + viết tài liệu (kết quả âm cũng là kết quả) |
| Build hỏng > 15 phút | revert về commit cuối, build lại, xác nhận 3×3 |
| Gate đỏ lớp `T-INFRA` | chạy lại tối đa **2 lần**, ghi nhận, **không** sửa cho xanh |
| Chữ ký mới | lưu artifact, chỉ đuổi nếu ≥ **3 lần** |
| `ps` rớt SSH | artifact từng boot, chạy bù phần thiếu, không làm lại từ đầu |
| **6:30** | đóng băng code tuyệt đối |

---

## 7. Bẫy đã biết (đừng mắc lại)

1. **`V2GROUP 06` không phải "nhóm 6"**: `v2_failed = 3 + group4` ⇒ 06 = nhóm 4c (readdir thiếu `d`).
2. **Ring chỉ đủ nếu chụp SÁT chỗ hỏng** — chụp ở cuối khối V2 thì ring đã trôi, mất trắng (đã xảy ra ở S1n: 7 boot đỏ mà ring không có bản ghi liên quan).
3. **Artifact bị giữ chọn lọc** ⇒ **không** đếm artifact để tính tỉ lệ; dùng bảng N trong `progress-g1.md`.
4. **`dup=[...]` = "số lần ≠ 1"**, không phải "in hai lần".
5. **`0/N` chỉ là trần**, phải ghi CI.
6. **Đo nhầm binary**: kiểm dòng `BUILD <tên>=<id> feats=<feature>` trong serial trước khi tin số.
7. **`-smp 1` không thoát boot** ⇒ mỗi lần đo tốn hết timeout; đặt `VIFAST_TIMEOUT` thấp.
8. **Máy `ps`** đã lên `rustc 1.99.1` — nếu build lỗi `rust-version`, kiểm toolchain trước khi nghi mã.
9. **`vfs_fsck()` trả bool; nhãn in `!x`** — đây là lần thứ **hai** mắc lỗi đọc ngược; trước khi kết luận, tra công thức in nhãn.

---

## 8. Bàn giao lúc 6:00–8:00

1. `progress-g1.md`: một mục cho **từng khối giờ** (kể cả khối thất bại, kèm số).
2. Bảng số: `cấu hình | máy | N | đỏ | CI 95 % | BUILD_ID`.
3. HEAD sạch, không file dở; **gate 10/10**; **3×3 QEMU** trên HEAD cuối.
4. Một trang: "T26 = (cơ chế hoặc chưa biết)", "đã bác thêm gì", "3 việc tiếp", "cần quyết gì".
5. Báo cáo ký SHA-256; artifact các ca đỏ trong `artifacts/`.
