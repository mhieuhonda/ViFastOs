# ViFastOs — Báo cáo vòng 90 (fault resume: lưu frame khi task fault)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `8934ace` (sau vòng 89)
**Bối cảnh:** audit #4 P1-3/P1-4 — `fault::suspend` không lưu trap frame, task
được pager cứu sẽ chạy lại từ frame cũ (hoặc từ entry point).

## 1. Vấn đề

`timer::on_timer` lưu frame hiện tại vào `contexts[slot]` trước khi switch,
nhưng `fault::suspend` thì **không**. Khi một task fault vì trang chưa map và
pager được cứu (map trang rồi reply), task quay lại `contexts[slot]` — đó là
frame của lần preemption trước, không phải điểm fault. Task có thể chạy lại
đoạn mã đã chạy, hoặc tệ hơn là từ entry point.

## 2. Sửa

- `fault::suspend` lưu `*context` vào context slot của TCB đang fault (trước
  khi chọn task kế tiếp), nên nếu pager cứu, task tiếp tục đúng điểm fault và
  chạy lại instruction — lúc này trang đã được map.
- Cổng theo kiến trúc: thêm `Platform::fault_frame_complete()` (mặc định
  `true`). x86_64 trả `false` vì trap.S đẩy vector/error **trước** SAVE_REGS
  nên `x86_exception_handler`/`x86_page_fault` hiện truyền `TrapFrame::empty()`;
  lưu frame rỗng sẽ làm task chạy lại từ 0 và sinh `KERNEL EXCEPTION vector=13`
  (đã thử và xác nhận). Khi nào trap.S x86 xuất được frame đầy đủ, chỉ cần đổi
  cờ này thành `true`.

## 3. Kiểm chứng

- QEMU: riscv64 2/2, aarch64 2/2, x86_64 2/2 (sau khi thêm cổng).
- `gate.sh --with-tcp --with-input`: all steps passed.
- Đường fault endpoint vẫn in `USER FAULT ... FAULT EP OK` như trước.

## 4. Ghi chú quy trình

- Audit #5 (subagent) **thất bại hai lần** với lỗi phía agent (không có báo
  cáo). Không có phát hiện mới nào từ nó trong vòng này; audit sẽ được chạy
  lại ở vòng kế tiếp. Các mục còn mở vẫn lấy từ audit #4.

## 5. Còn mở

- x86 TSS hard-code `[1]` vs `MAX_HARTS = 4` (cần const assert + sửa vòng lặp).
- x86 chưa truyền được frame thật cho fault resume (đã có cổng tạm).
- P2: `--quick` cũng bỏ bước docs; `soak.py` đã sửa; hằng `MAX_TCBS` vs dung
  lượng runqueue nên có const assert.
