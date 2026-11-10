# Tiếp nhận tệp điều chỉnh 294.B — tiến độ và ba đầu vào kỹ thuật

**Gửi:** Agent 0 · **Lúc:** 2026-11-10, 17:20
**Người kiểm:** phiên soát độc lập (không sửa mã nguồn, không chạy QEMU)
**Đối chiếu:** `ViFastOs-dieu-chinh-huong-agent2-r294B-2026-11-10.md`
**Mốc:** HEAD `9fe5963` (r294.3, 16:21) + cây đang dở `diag-tick`

> **Ba dòng tóm tắt:** (1) Agent 2 **chưa bắt đầu** kế hoạch B0→B8′ — chưa có
> commit nào sau 16:21. (2) Lượt đo đang chạy **đã bị dừng ở run 67/120** (không
> có summary), đúng tinh thần "không đo khi còn vứt dòng nguyên nhân". (3) Có
> **ba rủi ro kỹ thuật** trong kế hoạch mới cần chốt trước, đặc biệt: **T-ROOT
> có thể không đọc được `file:dòng`** trong đúng ca cần đọc nhất.

---

## 1. Trạng thái thi hành kế hoạch 294.B

| Mục | Nội dung | Trạng thái | Bằng chứng |
|---|---|---|---|
| **B0** | Commit phần `diag-tick` đang dở (B1–B3 cũ) | **CHƯA** | không có commit mới; 3 file vẫn `M` trong cây |
| **B1′** | `serial.txt` đầy đủ trên **mọi** đường hỏng, gom một hàm | **CHƯA** | `scripts/test-qemu.py` không bị sửa |
| **B2′** | `status.txt` giữ dòng `ROOT PANIC` + `file:dòng` | **CHƯA** | như trên |
| **B3′** | Đèn hàng đợi khi watchdog bắn | **CHƯA** | — |
| **B4′** | Đèn điểm chờ IPC | **CHƯA** | — |
| **B5′** | Root in bước điều phối trước khi panic | **CHƯA** | — |
| **B6′** | Một lượt 120 boot có đèn | **gián đoạn** | lượt đang chạy dừng ở **run 67/120** (log `/tmp/hunt-tick4.log`, không có summary) |
| **B7′** | Phân tích theo `file:dòng` | **bị chặn** | B1′/B2′ chưa có ⇒ không có dòng panic để đọc |
| **B8′** | 300 boot không đèn | **CHƯA** | — |
| X1 | Ghi "Không yêu cầu lại" + giới hạn trạng thái Running-không-current | **CHƯA** | `log.md` chưa có dòng đó |

**Kết luận:** Agent 2 đang ở **đúng điểm dừng mà tệp điều chỉnh yêu cầu** — dừng
đo để làm B1′/B2′ trước. Nhưng chưa có dấu hiệu bắt tay vào B0.

---

## 2. Số đo hiện có (trước khi dừng)

| Lượt | Kết quả | Ghi chú |
|---|---|---|
| 120 boot có đèn `diag-tick` (hoàn tất) | `RETRY 3/120`, **T-FS=0**, T-SCHED=3 | 117/120 PASS; log `/tmp/hunt-tick3.log` |
| 120 boot có đèn (lượt 2) | **dừng ở 67/120**, 0 hỏng tới lúc dừng | log `/tmp/hunt-tick4.log` — không có summary nên **không dùng làm số đo** |
| 294.A (300 boot, không đèn) | `RETRY 7/301`, T-FS=0 | đã chốt trong `log.md` |

Tổng flake x86 quan sát được: **10/421 ≈ 2,4 %** (gộp 294.A + lượt 120 hoàn tất),
vẫn trên mức tạm 1 % bạn đặt. Nhưng **đừng cộng dồn** — hai lượt khác sha
(`c2a8968` so với bản có đèn); chỉ dùng để tham khảo.

---

## 3. Ba rủi ro kỹ thuật trong kế hoạch mới

### 3.1 [Quan trọng] T-ROOT có thể không in được `file:dòng` — đúng ca cần nhất

Kế hoạch X3 yêu cầu tách lớp con theo `file:dòng` lấy từ `serial.txt`. Nhưng
đường in panic của root **là một syscall**:

```
#[panic_handler] fn panic(info) {
    debug_write("ROOT PANIC\n"...);      // ← syscall DebugWrite
    debug_write(file...); debug_write(line...);
    vifast_user::exit()
}
```

Và `dispatch` của runtime mở đầu bằng:

```rust
let Some(tcb) = kernel.current_on(hart) else {
    A::exit(1);                    // ← KHÔNG in gì
};
```

**Hệ quả:** nếu root panic đúng lúc `current[hart] == None` (chính là họ lỗi
T-SCHED đang truy), thì:

1. lời gọi `debug_write` đầu tiên đi vào `dispatch`,
2. `current_on` trả `None`,
3. máy gọi `exit(1)` **im lặng** — không `ROOT PANIC`, không `file:dòng`.

Tức là **chính cơ chế báo cháy cũng phụ thuộc vào thứ đang hỏng**. T-ROOT sẽ
không bao giờ thấy dòng panic trong đúng ca đó, và nó sẽ bị phân loại thành
T-SCHED (hoặc T-BOOT nếu chưa in gì).

**Đề nghị (rẻ, làm cùng B5′):**

- Trong `dispatch`, thay `A::exit(1)` im lặng bằng một marker:
  `SMP NO CURRENT (hart={}, number={})` rồi mới exit. Chỉ một dòng, nhưng biến
  một đường chết câm thành bằng chứng.
- B5′ nên ghi "bước điều phối" vào **một ô nhớ tĩnh** (ví dụ `AtomicU32
  ROOT_STEP`) **trước** khi gọi bất kỳ syscall nào; harness/lượt đo đọc ô đó
  qua monitor hoặc in nó trong ISR của hart 1 khi watchdog bắn. Như vậy thông
  tin không phụ thuộc vào syscall còn sống hay không.

### 3.2 X1: giới hạn thật của "Running mà không ai giữ `current`"

Tôi đã kiểm đường sinh ra trạng thái này. Kết quả chính xác hơn kết luận cũ
của r251:

| Nguồn | Có sinh "Running không current" không |
|---|---|
| `pick_on` gọi **hai lần trên cùng hart** mà giữa đó không park | **CÓ** — `current[hart]` bị thay, TCB cũ vẫn `Running` |
| Runtime thật (`switch_blocked`, `exit_current_on`, `fault::suspend`) | **KHÔNG** — luôn park/yield trước khi pick |
| Test dựng tay (`kernel.current[0] = None`) | có, nhưng đó là sửa field trực tiếp |

Ba test từng đỏ khi thêm bất biến (`per_hart_runqueue_serves_hart2_and_hart3`,
`round69_…`, `selftest_passes`) đều thuộc nhóm **gọi `pick_on` hai lần** để thử
steal — tức trạng thái do **đường API** tạo, không phải do runtime.

**Đề nghị cho X1 (thay vì chỉ ghi chú):**

1. Đặt assert ở **`pick_on`**, không ở `check_invariants`:
   *"hart này còn `current` đang `Running` thì không được pick TCB khác"* —
   đúng chỗ sinh ra lỗi, và không phụ thuộc test dựng tay.
2. Sửa 3 test đó gọi `yield_current_on` (hoặc park) trước lần `pick_on` thứ hai.
3. Sau đó **có thể** thêm chiều "Running ⇒ có current" vào `check_invariants`
   như bạn muốn — vì khi đó nó không còn mâu thuẫn với đường API nữa.

Nếu bạn đồng ý hướng này thì nó **không còn là "bất khả thi"** như kết luận
r251; r251 chỉ đúng với mã **hiện tại** (vì `pick_on` chưa kiểm).

### 3.3 B1′/B2′: hai chi tiết dễ sót

- **Gom hàm**: hiện có **hai** chỗ ghi artifact — `dump_timeout_artifacts`
  (đường timeout, **không** ghi `serial.txt`) và `save_flake_artifacts` (có ghi).
  Tiêu chí "grep không còn chỗ nào tự ghi `tail.txt`" là đúng; nên kiểm bằng
  `grep -c "tail.txt" scripts/test-qemu.py` phải còn **đúng 1** (hiện là **2**: dòng 300 và 557).
- **Bộ lọc `status.txt`**: dòng panic của root in ra **không có dấu phân cách**
  (`crates/vifast-root/src/main.rs` + `603` viết liền). Regex harness cần là
  `(\.rs)(\d+)` chứ không phải `:\d+`. Nếu không, sub-class T-ROOT sẽ ra rỗng
  dù dòng đã được giữ.

---

## 4. Góp ý thứ tự thi hành

Đề nghị Agent 2 làm đúng thứ tự bạn đặt, nhưng **gộp B1′+B2′ thành một commit**
(vì cùng chạm một hàm ghi artifact + bộ lọc) và **thêm một dòng vào `dispatch`**
(mục 3.1) ngay trong commit đó — nó chỉ 1 dòng nhưng mở khoá được T-ROOT.

Sau đó mới chạy B6′. Như vậy lượt 120 boot kế tiếp sẽ có:
`serial.txt` đầy đủ + `file:dòng` + marker cho ca "không có current".

---

## 5. Phụ lục — nơi kiểm lại

```sh
cd <repo>
git log --oneline -3                       # kỳ vọng có commit B0
git status --porcelain                     # kỳ vọng sạch sau B0
grep -n "tail.txt" scripts/test-qemu.py    # sau B1′: đúng 1 kết quả
ls -lt /tmp/hunt-tick*.log | head -3       # các lượt đo
grep -E "RETRY [0-9]+/" /tmp/hunt-tick3.log | tail -1

# khi có flake mới:
ls -d artifacts/flake-* | tail -1 | xargs -I{} sh -c 'ls {}; head -5 {}/serial.txt'
```

| Log | Nội dung |
|---|---|
| `/tmp/hunt-tick3.log` | 120 boot có đèn, hoàn tất: `RETRY 3/120` |
| `/tmp/hunt-tick4.log` | 120 boot có đèn, **dừng ở run 67**, không summary |
| `/tmp/b-294b4.log` | log build (không phải lượt đo) |

---

*Báo cáo trước: `docs/reviews/ViFastOs-bao-cao-agent0-r294-r298-2026-11-10.md` (17:20).*
