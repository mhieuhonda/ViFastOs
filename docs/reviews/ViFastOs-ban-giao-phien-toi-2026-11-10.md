# Bàn giao phiên tối — gửi Agent 0 để lập kế hoạch qua đêm cho Agent 2

**Từ:** phiên soát độc lập
**Lúc:** 2026-11-10, 17:35 (bàn giao trước khi người vận hành rời máy)
**Mốc:** HEAD `4e8e23e` (r294.B0 + B1′/B2′), cây đang dở `main.rs` (nhánh GetTime)
**Mục đích của tệp:** cung cấp đủ dữ kiện để Agent 0 viết **một tệp điều chỉnh
chạy qua đêm** cho Agent 2 — tệp đó phải tự đứng được 8–10 giờ không có người.

> Người vận hành **sẽ rời máy**. Mọi quyết định trong đêm phải nằm trong tệp
> của Agent 0; Agent 2 không được hỏi lại, và không có ai duyệt giữa chừng.

---

## 1. Trạng thái bàn giao

| Hạng mục | Giá trị |
|---|---|
| HEAD | `4e8e23e` r294.B1′+B2′ |
| Commit trong đêm nay (từ 16:21) | 2: `4c8f608` (B0), `4e8e23e` (B1′+B2′) |
| Cây làm việc | **đang dở**: `crates/vifast-root/src/main.rs` (nhánh GetTime) |
| Gate sau B1′ | **đã qua 6/8 bước** (build, kernel-test, tests-present, fuzz, qemu, boundary — tất cả PASS); đang ở bước persist/docs. Log `/tmp/gate-b1.log` |
| Test host | 369 (11 crate) — kernel 256 |
| Harness | 647 case |
| Flake x86 hiện tại | **~2 %/boot** (khoảng 10/421 quan sát; mục tiêu tạm 1 %) |
| `MAX_IO` | 128 (BLOCKER FS **ngủ**, `T-FS = 0/305`) |

---

## 2. Những gì đã chắc — đừng ra lệnh trùng

| Kết luận | Bằng chứng | Trạng thái |
|---|---|---|
| Lớp `tcb=None`/`SMP PARK` **đã đóng** | 0/301 sau `c2a8968` | đóng theo số đo, mở lại khi thấy 1 lần |
| **"Tick BSP ngừng phát" — BÁC** | `TICK` chạy tới 752–848, `eoi` khớp | bác bằng số đo |
| **"PIC kẹt in-service / bị mask" — BÁC** | `PIC STALL (isr=0x00, irr=0x01, imr=0xec)` | bác bằng số đo |
| `FS V2 OK` **không tái phát ở 128** | `T-FS = 0/305` | BLOCKER ngủ, chưa đóng |
| Lớp thật của "treo sau `shell>`" là **T-ROOT `main.rs:480`** | 6/6 lần trong 300 boot đều có `ROOT PANIC` + dòng 480 | **đây là điểm rẽ hiện tại** |
| Bất biến `current ↔ TCB` hai chiều đúng | `fc87374` + 3 test | xong, có "lệch có chủ đích" Agent 0 đã chấp nhận |

**Ba việc Agent 0 đã yêu cầu nhưng chưa làm:** D1 (marker `SMP NO CURRENT`),
D2 (`pick_on` từ chối khi current còn Running), D3 (dấu `:` trong `file:dòng`).

---

## 3. Điểm rẽ: `main.rs:480` — nhánh đang làm là **dead code**

### 3.1 Dữ kiện

Agent 2 đang sửa `smp_worker_c` để phân biệt "GetTime lỗi" với "thời gian lùi":

```rust
if sample.error != KError::Ok { … panic!("GetTime syscall failed on hart 1") }
let now = sample.value;
if now < last { … panic!("GetTime went backwards") }   // ← dòng 480
```

### 3.2 Vì sao nhánh mới không bao giờ chạy trên x86

Trap handler x86 có **fast path cho GetTime chạy trước khi lấy BKL**:

```rust
if frame.rax == SyscallId::GetTime as u64 {
    frame.rax = 0;                        // ← error LUÔN Ok
    frame.rdi = X86Platform::kernel().fast_time();
    return;
}
```

⇒ `error` luôn `Ok`; `value` luôn `fast_time()`. Nhánh `GETTIME ERR` là
**dead code** ở đúng kiến trúc đang hỏng.

### 3.3 Vì sao "thời gian lùi" là bất khả thi — và cái gì thật sự sai

`fast_time()` = `system_ticks` = `fetch_max(ticks[hart])` ⇒ **đơn điệu, không
giảm**. Nên `now < last` **không thể** do đồng hồ; cái sai là **`last` trên
stack user của worker**.

Ba cơ chế khả dĩ — **cả ba thuộc scheduler/context**:

| # | Cơ chế | Dấu hiệu phân biệt |
|---|---|---|
| (a) | Một TCB bị pick trên **hai hart** ⇒ hai hart ghi chung một context slot | cùng `tcb=`, khác `hart=` |
| (b) | Hai TCB **dùng chung context slot** (`context_slot(..).unwrap_or(0)`) | khác `tcb=`, cùng `slot=` |
| (c) | `suspend`/`resume` khôi phục **context cũ** (worker C đúng là bị suspend/resume) | `slot=` nhảy về giá trị cũ |

### 3.4 Chi phí phân biệt: **một lượt 60–120 boot (8–16 phút)**

Chỉ cần thêm đèn: `PICK hart= tcb= slot= state=` ở `pick_on`, và
`CTX SAVE/LOAD hart= slot=` ở chỗ lưu/khôi phục context. Flake ~2 %/boot nên
120 boot gần như chắc chắn có 1–2 lần.

**Nếu (a)/(b)/(c) đúng thì đây rơi đúng ngoại lệ của D2** trong tệp bổ sung:
panic chỉ vào scheduler ⇒ làm `D2.a` **ngay**, không chờ B6′.

---

## 4. Việc còn lại và ước thời lượng

| Việc | Ước thời lượng | Cần phán đoán? |
|---|---|---|
| Đèn pick/context + 1 lượt 120 boot | 20 + 16 phút | có (đọc kết quả) |
| Sửa gốc (a)/(b)/(c) + test đỏ-trước | 40–60 phút | có |
| D1.a marker `SMP NO CURRENT` | 20 phút | không |
| D1.b (đổi sang in `ROOT STEP` trước mỗi syscall) | 30 phút | không |
| D2.a `pick_on` từ chối + sửa 3 test | 40–60 phút | có (chạm scheduler) |
| D3 dấu `:` + regex hai dạng | 15 phút | không |
| **B3′/B4′** (đèn hàng đợi, điểm chờ IPC) | 45 phút | không |
| **294.C** (lớp T-BOOT/T-INFRA + 3 mốc boot) | 60–90 phút | không |
| **294.5** (riscv64 hộp thư + tách doorbell) | 90–120 phút | có (chạm 1 arch) |
| **r295** (FS 291.5 fd đang mở) | 90–120 phút | có |
| Đo 300 boot (x86) | 40 phút | không |
| Gate đầy đủ 3 arch | 20–25 phút | không |

---

## 5. Bảy ràng buộc của phiên qua đêm (Agent 0 nên chép vào tệp)

1. **Mỗi 60–90 phút phải commit** (kể cả khi dở) và ghi một mục `log.md`.
   Sáng ra phải đọc được "đêm qua làm gì" chỉ từ git log.
2. **Không có người duyệt ⇒ không mở rộng phạm vi.** Chỉ làm trong 294.B +
   phần 294.C/294.5/r295 nếu được chỉ định; không refactor, không đổi ABI.
3. **Cổng thoát giả thuyết:** nếu một nhánh 90 phút không sinh dữ kiện mới
   (không có dòng log mới, không có số đo mới) thì **đổi nhánh**, ghi lý do.
4. **Gate đỏ hai lần liên tiếp ⇒ dừng**, viết báo cáo, không sửa mò. (Điều kiện
   dừng cũ vẫn giữ.)
5. **Không chạy đo khi chưa có đèn đọc được nguyên nhân** (luật B1′/B2′ đã có).
   Mỗi lượt đo phải ghi sha + N + RETRY theo lớp.
6. **Tài nguyên — số đo thật:** `artifacts/` hiện **31 thư mục flake, 13 MB**;
   đĩa còn **54 GiB**. Nên **không** phải lo đĩa. Rủi ro thật là **QEMU mồ côi**:
   một lượt bị ngắt giữa chừng có thể để lại tiến trình `qemu-system` giữ CPU.
   Luật: trước mỗi lượt đo, `ps aux | grep -c qemu-system` phải bằng 0; nếu khác
   thì dọn trước khi chạy. Giữ 10 thư mục flake mới nhất để sáng đọc, phần cũ
   xoá được (không bắt buộc).
7. **Máy không được ngủ.** Đây là rủi ro vận hành số một của phiên qua đêm:
   nếu macOS sleep, mọi lượt QEMU đứng im và "kết quả" sẽ là 0/N giả. Cần chạy
   `caffeinate -dimsu` (hoặc `pmset noidle`) bao quanh, và ghi vào log rằng đã
   bật. Nếu không bật được thì phải nói rõ trong báo cáo sáng.

---

## 6. Ba phương án kế hoạch qua đêm

| | Phương án | Nội dung | Ưu | Nhược |
|---|---|---|---|---|
| **A** | Truy T-ROOT đến gốc | Đèn pick/context → 120 boot → sửa (a/b/c) → 300 boot → gate | giá trị cao nhất: có thể xoá 2 %/boot | nếu (a/b/c) đều sai thì mất 3–4 giờ |
| **B** | Dọn nợ chắc chắn | 294.C + 294.5 + r295 + D1/D3 | gần như chắc chắn xong, ít rủi ro | flake 2 % vẫn còn; sáng ra vẫn chưa biết gốc |
| **C** | Kết hợp có mốc | 0–2h: A (đèn + 120 boot + sửa nếu xác định). 2–4h: nếu A bế tắc → D1/D2/D3 (D2.a có thể chính là bản vá (a)). 4–7h: B (294.C hoặc 294.5). 7–8h: 300 boot + báo cáo | vừa có cơ hội xoá flake, vừa chắc chắn có sản phẩm | cần kỷ luật mốc giờ |

**Khuyến nghị: C.** Lý do: `D2.a` (chặn `pick_on` khi current còn Running)
**gần như chắc chắn** là bản vá cho cơ chế (a) nếu (a) đúng — nên nó là "hai
trong một": vừa là yêu cầu có sẵn của Agent 0, vừa là phép thử giả thuyết.
Và 294.C/294.5 là loại việc không cần phán đoán, rất phù hợp cho 3 giờ cuối
khi người vận hành đã ngủ.

---

## 7. Checklist những gì tệp qua đêm của Agent 0 **phải** có

1. **Mốc giờ cụ thể** cho từng khối (ví dụ "18:00–20:00 khối A") + lệnh đổi
   nhánh khi hết mốc.
2. **Thứ tự ưu tiên tuyệt đối** khi hết giờ giữa chừng (việc nào bắt buộc xong).
3. **Danh sách việc bị cấm** trong đêm (refactor, đổi ABI, nới harness, xoá
   test, sửa `MAX_IO`).
4. **Điều kiện dừng** (giữ các điều kiện cũ + 3 điều kiện mới ở §5.4).
5. **Mẫu báo cáo sáng** (xem §8).
6. **Chỗ ghi bằng chứng**: mỗi kết luận phải kèm sha + N + RETRY theo lớp +
   đường dẫn artifact.
7. **Quyết định về `caffeinate`** và về việc dọn `artifacts/`.

---

## 8. Mẫu báo cáo sáng (để Agent 2 điền)

```
## Báo cáo đêm 2026-11-10 (r294.B → …)
- Khối đã chạy: <mốc giờ> <nội dung> <kết quả>
- HEAD cuối: <sha>; cây: sạch/dở
- 294.B: đèn đã thêm; lượt đo (sha, N, RETRY theo lớp)
- T-ROOT main.rs:480: cơ chế xác định được là (a)/(b)/(c)/chưa; bằng chứng
- D1/D2/D3: xong/chưa; nếu chưa, vì sao
- 294.C / 294.5 / r295: xong/chưa
- Số đo: flake x86 hiện tại (sha, N, lớp), so với mốc 2 %
- Việc phát sinh + câu hỏi cần Agent 0
- Cảnh báo vận hành (máy ngủ? đĩa đầy? gate đỏ?)
```

---

## 9. Phụ lục

### 9.A Commit gần nhất

| Commit | Nội dung |
|---|---|
| `4c8f608` | B0: đèn TICK/PIC/LAPIC sau `diag-tick`; bác 2 giả thuyết; lộ T-ROOT `main.rs:480` |
| `4e8e23e` | B1′+B2′: một đường ghi artifact, giữ khối `ROOT PANIC` |
| `fc87374` | 294.3 bất biến `current ↔ TCB` |
| `c2a8968` | 294.4 nhả BKL trước khi halt |

### 9.B Log và artifact cần đọc sáng mai

| Đường dẫn | Nội dung |
|---|---|
| `docs/decisions/log.md` | nhật ký từng vòng (Agent 2 ghi) |
| `docs/reviews/ViFastOs-bao-cao-agent2-*.md` | báo cáo của Agent 2 |
| `target/vifast-qemu-last.json` | sha + N + RETRY theo lớp của lượt đo cuối |
| `artifacts/flake-*/serial.txt` | serial đầy đủ (từ B1′ trở đi) |
| `/tmp/gate-b1.log` | gate đang chạy lúc bàn giao |

### 9.C Lệnh kiểm nhanh cho người vận hành (nếu quay lại giữa đêm)

```sh
cd <repo>
git log --oneline -10                 # đêm qua làm gì
git status --porcelain                # cây có sạch không
python3 - <<'PY'
import json; d=json.load(open('target/vifast-qemu-last.json'))
print(d.get('repeat'), d.get('results')); print(d.get('retry',{}).get('classes'))
PY
ls -d artifacts/flake-* | wc -l       # số lần hỏng đã bắt
ps aux | grep -c "[t]est-qemu"        # còn chạy không
```

---

*Các báo cáo trước: `ViFastOs-bao-cao-agent0-kiem-tra-tien-do-2026-11-10.md`
(15:35), `ViFastOs-bao-cao-agent0-r294-r298-2026-11-10.md` (17:20),
`ViFastOs-bao-cao-agent0-r294B-tien-do-2026-11-10.md`,
`ViFastOs-ghi-chu-cho-agent2-r294B-2026-11-10.md` (17:28).*
