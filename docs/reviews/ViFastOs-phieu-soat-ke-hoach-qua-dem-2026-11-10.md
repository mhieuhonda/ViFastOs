# Phiếu soát kế hoạch qua đêm (trước khi gửi Agent 2)

**Soát bởi:** phiên kiểm độc lập · **Lúc:** 2026-11-10, 17:45
**Đối tượng:** `ViFastOs-ke-hoach-qua-dem-agent2-2026-11-10.md`
**Mốc đối chiếu:** HEAD `4e8e23e`, cây dở `crates/vifast-root/src/main.rs`
**Hiệu lực:** phiếu này **sửa năm điểm** của kế hoạch qua đêm. Chỗ nào phiếu
không nhắc thì **kế hoạch vẫn đúng**. Khi hai tệp mâu thuẫn: **phiếu này thắng**.


> **Kết luận: kế hoạch dùng được, cấu trúc và kỷ luật mốc giờ tốt.** Có **năm
> điểm** nếu không vá sẽ làm mất chính bằng chứng mà khối A cần tìm — trong đó
> điểm 1 và 3 có thể làm cả khối A trắng tay. Sửa tổng cộng khoảng **10 phút**.
> Ba điểm nhỏ còn lại chỉ để tiết kiệm thời gian.

---

## 1. Năm điểm phải vá trước khi gửi

### 1.1 [Chặn] Trigger đổ vòng đệm sẽ **không bao giờ chạy** trên đúng ca cần nó

Kế hoạch A2 viết: *"Đổ vòng đệm ra serial khi: root kết thúc với mã khác 0,
hoặc watchdog hart 1 bắn, hoặc có `SMP NO CURRENT`"*.

Nhưng `vifast_user::exit()` gửi `Exit` với `a0 = 0` — **luôn mã 0**:

```rust
pub fn exit() -> ! {
    let _ = syscall(SyscallId::Exit as u64, [0; 6]);   // a0 = 0
    loop { core::hint::spin_loop(); }
}
```

⇒ điều kiện "root kết thúc với mã khác 0" **không bao giờ đúng**. Mà ca hỏng
của ta đúng là `ROOT PANIC` → `exit()`. Còn watchdog hart 1 thì có thể không bắn
kịp (hart 1 idle-halt). Kết quả: vòng đệm ghi được sự kiện nhưng **không ai đổ
nó ra**.

**Sửa (1 dòng):** đổ vòng đệm khi **xử lý `UserAction::Exit`** trong runtime
(mọi lần exit, không cần mã), cộng thêm hai trigger đã có. Nếu muốn hẹp hơn:
chỉ khi TCB vừa exit là **root**.

### 1.2 [Chặn] D1.a vẫn đòi "test host đỏ-trước" — không thi hành được

`dispatch` generic theo `Platform`; `vifast-platform-*` chỉ build cho bare-metal
⇒ không thể gọi `dispatch` trong test host. Ở khối D (T0+2h, đêm), Agent 2 sẽ
hoặc dựng fake Platform 30+ method (rất tốn), hoặc bỏ test (vi phạm luật
"đỏ-trước" của chính kế hoạch).

**Sửa (2 dòng trong kế hoạch):** tiêu chí thành *"hàm thuần
`fn no_current_marker(hart: u32, number: u64, out: &mut [u8; 40]) -> usize` có
test host đỏ-trước; nhánh `dispatch` gọi hàm đó rồi `A::putc` từng byte"*.

### 1.3 [Chặn] Thiếu phép kiểm trực tiếp cho giả thuyết (b)

Kế hoạch A2 nói: *"thay mọi `context_slot(..).unwrap_or(0)` bằng xử lý lỗi rõ
ràng… nếu thay đổi này làm đỏ test nào thì đó là bằng chứng cho (b)"*. Lập luận
này **không đúng**: test đỏ chỉ chứng minh có code path dựa vào fallback, **không**
chứng minh hai TCB đang dùng chung một slot lúc chạy.

Hiện `check_invariants` **không** kiểm tính duy nhất của context slot:
`context_slot_used` là bitmask, hai TCB cùng slot vẫn chỉ bật **một** bit nên
không lộ ra.

**Sửa (thêm 3 dòng vào `check_invariants`, và gọi nó trong đường diag):**

```
đếm = số TCB sống có context_slot
nếu đếm != context_slot_used.count_ones()  ⇒ IllegalOperation
```

Bằng nhau ⇒ không thể có hai TCB chung slot. Đây là phép kiểm **quyết định**
cho (b), và đúng lớp lỗi "hai nơi phải đồng ý với nhau" mà dự án đã gặp nhiều
lần.

### 1.4 [Mất bằng chứng] Đừng `git checkout --` mà không lưu bản vá

§0.4 yêu cầu bỏ nhánh GetTime đang dở bằng `git checkout --`. Quyết định **bỏ
là đúng** (nhánh đó là mã chết trên x86), nhưng nên giữ lại 2 dòng diff để sáng
có thể đối chiếu:

```sh
git diff crates/vifast-root/src/main.rs > /tmp/wip-gettime-$(date +%H%M).patch
git checkout -- crates/vifast-root/src/main.rs
```

### 1.5 [Nhập nhằng] `T-BOOT` chưa được định nghĩa theo marker

294.C nói T-BOOT = *"không có dòng serial nào của kernel"*, nhưng đồng thời đưa
3 mốc `UEFI ENTRY` / `EXIT-BS OK` / `KERNEL ENTRY`. Nếu `UEFI ENTRY` đã in mà
chết trước `KERNEL ENTRY`, câu "không có dòng nào" sai và Agent 2 sẽ phân loại
lẫn.

**Sửa:** `T-BOOT ⇔ không thấy **KERNEL ENTRY***. Thấy `KERNEL ENTRY` mà chết
trước marker khác thì **không** phải T-BOOT (rơi về lớp theo chữ ký, hoặc
T-KHAC).

---

## 2. Ba điểm nhỏ (tiết kiệm thời gian, không bắt buộc)

### 2.1 (d) có thể loại bằng 3 dòng đọc mã, không cần 15 phút test

`fast_time()` là **một** biến toàn cục:

```rust
pub fn fast_time(&self) -> u64 { self.system_ticks.load(Acquire) }
```

`system_ticks` chỉ được cập nhật bằng `fetch_max` ⇒ đơn điệu **toàn cục**, không
phụ thuộc hart. Giả thuyết (d) "trả `ticks[hart]` cục bộ" không tồn tại trong
mã hiện tại.

Vẫn nên có test host (rẻ, khoá lại bất biến), nhưng đừng dành 15 phút điều tra.

### 2.2 Thêm giả thuyết (e): giá trị hỏng **giữa kernel và user**, không chỉ trên stack

Kế hoạch giả định `last` sai. Nhưng `value` cũng có thể bị hỏng trên đường trả
về: fast path ghi `frame.rdi = fast_time()` rồi `iretq` khôi phục `rdi`. Nếu
trap frame bị dùng chung/ghi đè trên hart 1, user nhận **giá trị rác** dù kernel
trả đúng.

**Cách phân biệt (rẻ):** trong panic tại dòng 480, in thêm **raw `a0`/`a1`** mà
syscall trả về (không chỉ `sample.value`). Nếu raw cũng sai ⇒ frame hỏng; nếu
raw đúng mà `now` sai ⇒ stack hỏng.

### 2.3 Chi phí build mỗi vòng đo

`build.sh` build **cả 3 arch** (~6–8 phút). Các vòng đo x86 chỉ cần x86 ⇒ nên
build đúng target x86 (hoặc dùng `build.sh` một lần rồi chỉ rebuild crate đã
sửa). Khối A có 3–4 vòng ⇒ tiết kiệm được ~20 phút.

---

## 3. Hai chỗ về thời gian

- **Khối Z (1 giờ) hơi chặt:** 300 boot ≈ 40 phút + gate 3 arch ≈ 20–25 phút =
  60–65 phút. Đề nghị bắt đầu Z ở **T0+6h45** thay vì T0+7h, hoặc chạy gate
  trước rồi mới 300 boot.
- **Khối A** có luật "xong sớm thì sang D sớm" ✓ — giữ nguyên.

---

## 4. Nếu bạn không kịp sửa gì

Gửi kèm kế hoạch **ba dòng** này là đủ để Agent 2 không mất bằng chứng:

```
1) A2: đổ vòng đệm khi XỬ LÝ Exit (exit() luôn mã 0), không phải "exit mã khác 0".
2) D1.a: test host = hàm thuần no_current_marker(); dispatch chỉ gọi nó.
3) A2: thêm invariant "số TCB sống có context_slot == popcount(context_slot_used)"
   vào check_invariants, gọi trong đường diag.
```

---

## 5. Những gì kế hoạch làm đúng (đừng đổi)

- **Cấu trúc khối + mốc giờ tuyệt đối + luật commit 75 phút** — đúng thứ phiên
  không người cần.
- **Cổng thoát 90 phút** và **"240 boot có đèn mà 0 lần ⇒ nghi đèn làm lỗi biến
  mất"** — đúng về thống kê (với p≈2 %, xác suất 0/240 chỉ ~0,8 %).
- **Bỏ nhánh GetTime** — đúng, nhánh đó là mã chết trên x86.
- **Hoãn D1.b (ROOT_STEP)** — đúng, tránh đổi ABI trong đêm.
- **Danh sách cấm** đủ chặt; **điều kiện dừng** đủ rõ.
- **Mẫu báo cáo sáng** đủ chi tiết để không phải hỏi lại.

---

*Tệp liên quan: `ViFastOs-ban-giao-phien-toi-2026-11-10.md` (17:35),
`ViFastOs-ghi-chu-cho-agent2-r294B-2026-11-10.md` (17:28).*
