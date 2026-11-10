# Ghi chú kỹ thuật gửi kèm tệp bổ sung 294.B (cho Agent 2)

**Từ:** phiên soát độc lập · **Lúc:** 2026-11-10, 17:28
**Kèm theo:** `ViFastOs-dieu-chinh-agent2-r294B-bo-sung-2026-11-10.md`
**Mốc:** HEAD `4e8e23e` (B0 + B1′/B2′ đã commit), cây đang sửa `main.rs:480`

> Ghi chú này **không thay** tệp của Agent 0. Nó chỉ: (1) cập nhật phần đã lỗi
> thời, (2) chỉnh ba chỗ khó thi hành, (3) **sửa hướng điều tra `main.rs:480`**
> vì nhánh đang làm sẽ không bao giờ chạy trên x86.

---

## 0. Phần đã lỗi thời của tệp bổ sung

| Mục trong tệp bổ sung | Hiện trạng |
|---|---|
| §2.1 **B0** | **XONG** — `4c8f608` |
| §2.2 **B1′ + B2′** | **XONG** — `4e8e23e` (một đường ghi artifact, giữ khối ROOT PANIC) |
| §2.3 B3′, B4′, D1.b/c | chưa |
| §2.4 B6′ | chưa chạy lại sau B1′ |

Phần còn hiệu lực: **D1, D2, D3** và thứ tự B3′ → B8′ (xem §4 dưới).

---

## 1. Ba chỗ cần chỉnh để thi hành được

### 1.1 D1.a — tiêu chí "test host" phải thu hẹp

`dispatch` generic theo `Platform`; các crate `vifast-platform-*` chỉ build cho
target bare-metal ⇒ **không thể** "gọi dispatch khi không có current" trong test
host (sẽ phải dựng fake Platform 30+ method).

**Đề nghị:** tách hàm thuần trong `vifast-runtime`:

```rust
pub fn no_current_marker(hart: u32, number: u64, out: &mut [u8; 40]) -> usize
```

có test host (đỏ-trước: chưa có marker ⇒ đỏ); nhánh `dispatch` gọi hàm này rồi
ghi từng byte qua `A::putc` (không qua syscall). Giữ đúng tinh thần D1.a mà
không cần fake Platform.

### 1.2 D1.b — nên bỏ ô nhớ tĩnh, chỉ cần in trước mỗi bước

Kế hoạch: root ghi `ROOT_STEP` vào ô tĩnh, kernel đọc qua bảng trang của root.
Hai vướng thật:

1. Kernel phải biết **VA** của ô đó ⇒ phải thêm trường vào `BootInfo` hoặc hằng
   trong `vifast_abi::user_layout` ⇒ **đổi ABI**, đúng điều kiện dừng #3 của tệp
   r293–r298.
2. `Platform::read_user` chỉ đọc được khi VSpace của root **đang active**
   (x86: `active_root_matches_current`). Watchdog chạy trên hart 1 — lúc đó hart
   1 có thể đang chạy tiến trình khác ⇒ đọc sai hoặc lỗi.

**Đề nghị thay:** root in `ROOT STEP <n>` **ngay trước** mỗi syscall điều phối,
sau feature `diag-root-step`. Bước cuối cùng in ra **chính là** bước đang treo —
đúng thứ cần biết, không cần ô tĩnh, không đọc chéo, không đổi ABI. Nếu sau này
vẫn cần giá trị đọc được khi máy đã chết, mới quay lại phương án ô tĩnh.

### 1.3 D2.a — đổi chữ ký `pick_on` chạm 8 chỗ, cần chốt hành vi bản phát hành

`pick_on` hiện trả `Option<ObjRef>`; có **8 call site ngoài kernel** (5 trong
`vifast-runtime`, 3 trong `vifast-platform-*`) + 3 trong kernel.

**Đề nghị ghi rõ:** bản phát hành trả `None` + in `PICK REFUSED (hart, current)`
**một lần**, không exit; assert trong `test` và `test-exit`. Nếu trả lỗi mà
caller lại `exit(1)` thì có nguy cơ biến một bất biến đúng thành sập máy.

---

## 2. Sửa hướng điều tra `main.rs:480` (quan trọng nhất)

### 2.1 Nhánh `sample.error != Ok` **không bao giờ chạy** trên x86

`vifast_user::get_time()` → `syscall(GetTime)`. Trap handler x86 có **fast path**
chạy **trước** khi lấy BKL:

```rust
if frame.rax == vifast_abi::SyscallId::GetTime as u64 {
    frame.rax = 0;                       // ← error LUÔN Ok
    frame.rdi = X86Platform::kernel().fast_time();
    return;
}
```

⇒ trên x86, `error` luôn `Ok`, `value` luôn `fast_time()`. Nhánh mới
`if sample.error != KError::Ok { … panic!("GetTime syscall failed") }` là
**dead code** ở kiến trúc đang hỏng. Giữ lại cũng rẻ (phòng arch khác), nhưng
**đừng dành vòng này cho nó** — nó sẽ không bắt được lần hỏng nào.

### 2.2 Vì `fast_time()` đơn điệu, `now < last` nghĩa là **`last` trên stack user bị sai**

`fast_time()` = `system_ticks` = `fetch_max(ticks[hart])` ⇒ **không giảm**.
Nên giá trị trả về không thể nhỏ hơn lần trước. Cái sai **không phải đồng hồ**
mà là trạng thái user của `smp_worker_c`.

Ba cơ chế khả dĩ — **cả ba đều thuộc scheduler/context**, không phải GetTime:

| # | Cơ chế | Dấu hiệu phân biệt |
|---|---|---|
| (a) | Cùng một TCB bị pick trên **hai hart** ⇒ hai hart ghi chung một context slot | cùng `tcb=` xuất hiện ở hai `hart=` |
| (b) | Hai TCB **dùng chung một context slot** (`context_slot(..).unwrap_or(0)` — lỗi đã nêu trong báo cáo soát gốc) | hai `tcb=` khác nhau, cùng `slot=` |
| (c) | `suspend`/`resume` khôi phục **context cũ** (worker C được suspend/resume trong test SMP — đúng chỗ này) | `slot=` nhảy về giá trị cũ sau resume |

### 2.3 Đèn phân biệt rẻ nhất (đề nghị làm ngay, sau feature)

Thêm log tại `pick_on` và chỗ lưu/khôi phục context:

```
PICK hart=1 tcb=17 slot=5 state=Running
CTX SAVE hart=1 slot=5
CTX LOAD hart=1 slot=5
```

Đọc 20 dòng cuối trước `ROOT PANIC`: chỉ cần **một** lượt 60–120 boot là phân
biệt được (a)/(b)/(c) — vì flake hiện ~2 %/boot.

### 2.4 Hệ quả với D2 và thứ tự

Nếu (a), (b) hoặc (c) đúng thì **panic này chỉ vào scheduler**, tức rơi đúng
**ngoại lệ của D2**: làm `D2.a` **ngay**, không chờ B6′. Đây là cách biến "lỗi
câm" thành "lỗi có vị trí" mà tệp bổ sung đã nói.

---

## 3. Hai việc nhỏ phát sinh

- Chữ ký mới `GETTIME ERR <code>` và panic mới `GetTime syscall failed on hart 1`
  phải **đặt tên trong bảng phân loại** ngay, nếu không chúng thành T-KHAC chưa
  đặt tên ⇒ `--strict` đỏ.
- **D3:** sửa **cả hai phía** — panic handler của root in `file:dòng` **có dấu
  `:`**, harness regex nhận `\.rs:?(\d+)` (để vẫn đọc được artifact cũ).

---

## 4. Thứ tự đề nghị (thay §2 của tệp bổ sung, vì B0/B1′/B2′ đã xong)

1. **Đèn §2.3** (pick/context slot) → chạy **một** lượt 60–120 boot, cùng sha.
2. Phân biệt (a)/(b)/(c) ⇒ **sửa gốc + test đỏ-trước** (có thể chính là D2.a).
3. Chỉ khi bước 1 chưa đủ mới làm **B3′/B4′** (hàng đợi, điểm chờ IPC).
4. **B6′/B8′** đo lại theo ngân sách; **D2** nếu chưa làm ở bước 2.
5. Sau đó: 294.C, 294.5, r295 trở đi.

Lý do đảo B6′ xuống: B7′ đã có dòng panic (`main.rs:480`) từ việc phân tích lại
294.A, nên **không cần thêm 120 boot nữa mới phân tích được** — chạy thêm bây
giờ chỉ tốn ~15 phút.

---

## 5. Một dòng nhắc

Tệp bổ sung **thay X1**: **đừng** ghi vào `log.md` câu "Không yêu cầu lại:
Running không kéo theo có current". Kết luận đó nay đã bị D2 thay bằng "chặn
tại `pick_on`".
