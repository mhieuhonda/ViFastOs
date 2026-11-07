# ViFastOs — Báo cáo vòng 92 (x86: frame fault thật, pager cứu được task)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `40c97af`
**Bối cảnh:** audit #6 chỉ ra cách truyền `TrapFrame` đầy đủ cho x86 mà không
phải sửa trap.S, mở đường bật `fault_frame_complete() = true`.

## 1. Ba layout frame khác nhau (audit #6)

Sau `SAVE_REGS` (15 thanh ghi = 120 B):
- `int80`/`irq0`/`irq4`/`ap_irq`: rip@120 … ss@152 → **đúng** `TrapFrame`.
- `exc_common`: có thêm **vector + error** trước rip ⇒ rip@136.
- `pf_entry`: chỉ có error ⇒ rip@128.

Code cũ cast thẳng `rsp` thành `*TrapFrame` cho mọi đường nên least không thể
ổn cho exc/pf; đó là lý do phải truyền `TrapFrame::empty()`.

## 2. Sửa (Tier 1, không đụng asm)

```rust
unsafe fn from_raw(frame: *const u64, has_vector: bool) -> TrapFrame {
    let mut out = *(frame as *const TrapFrame); // 15 thanh ghi trùng layout
    let tail = if has_vector { 17 } else { 16 };
    out.rip = *frame.add(tail);
    out.cs = *frame.add(tail + 1);
    out.rflags = *frame.add(tail + 2);
    out.rsp = *frame.add(tail + 3);
    out.ss = *frame.add(tail + 4);
    out
}
```
- `x86_exception_handler`: dùng `from_raw(frame, true)`, `suspend(vector, 0)`.
- `x86_page_fault`: dùng `from_raw(_frame, false)`, giữ `(error, cr2)`.
- `fault_frame_complete()` → `true`.

## 3. Chống lệch layout

```rust
const _: () = assert!(size_of::<TrapFrame>() == 160);
const _: () = assert!(offset_of!(TrapFrame, rip) == 120);
const _: () = assert!(offset_of!(TrapFrame, cs) == 128);
```
Lệch một word là fail lúc build, không phải lúc chạy.

## 4. TSS/hart (audit #6 câu 1)

- x86 hiện chỉ hỗ trợ **một** AP: `AP_HART = 1` + `assert!(AP_HART < MAX_HARTS)`;
  `setup_ap_gdt_tss_idt`/trampoline/`init_cpu` dùng `AP_HART` thay literal, thêm
  assert hart id trong `ap_entry`.
- Audit nhấn mạnh: để bật hart 2/3 phải có GDT/TSS + trampoline **per-hart**
  (một GDT chung `5 + 2*MAX_HARTS` entry, `tss_sel(hart)`, `ap_entry(hart)`).
  Việc này để vòng riêng; hiện hart 3/4 không thể chạm tới nên là latent.

## 5. Kiểm chứng

- QEMU: x86_64 2/2, riscv64 1/1, aarch64 1/1; `gate.sh --with-tcp --with-input`
  all steps passed.
- Soak 12 vòng trước đó: 12/12 cả ba kiến trúc.
- Harness **508 case** (thêm marker `EL0 FP TRAP` cho aarch64 và `SSE USER OFF`
  cho x86 để hồi quy FP bị bắt).
