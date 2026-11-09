# Open items — bảng tồn đọng audit (hợp nhất)

Cập nhật: r233 (G1/T6) · HEAD soát: `g1` sau `r232` · Người soát: Agent 2
Nguồn: `docs/audits/2026-11-07-round79..99`, `round82`, `round88`, `round92`.

Quy ước trạng thái:
- **CLOSED** — có dấu vết vá trong code (comment `K<n>` dẫn tới file:dòng) và/hoặc có test.
- **OPEN** — không tìm thấy dấu vết vá; cần đường tái hiện trước khi sửa (D11: không sửa mò).
- **WONTFIX / UNREACHABLE** — trạng thái lỗi không còn tồn tại do một fix khác đóng lại.

## 1. K32..K75 (round99) — 44 mục

| K | Trạng thái | Bằng chứng / ghi chú |
|---|---|---|
| K32 | CLOSED | `ipc.rs:175` — resume gỡ hàng chờ |
| K33 | CLOSED | `lib.rs:1277` — cấm resume `Running`/current |
| K34 | CLOSED | `lib.rs:1277` — cấm resume `Running`/current |
| K35 | CLOSED | `ipc.rs:175` — restart gỡ hàng chờ |
| K36 | CLOSED | `ipc.rs:373` — chặn restart khi còn là client `BlockedReply` |
| K37 | CLOSED | r251: **không phải lỗi**. `current[hart]` cố ý giữ TCB đã park cho tới khi runtime lưu context (`switch_blocked` đọc `current` để lưu); bất biến hiện có đã canh `current ⇒ Running|Suspended`. Không tái hiện được bằng test host. |
| K38 | CLOSED | r251: cùng kết luận K37 (`Sleep` chỉ đặt state + `wake_at`). Test `r251_parked_task_leaves_current_after_switch`. |
| K39 | CLOSED | r251: cùng kết luận K37 (ipc.rs đặt `Blocked*`; `current` được `pick_on` thay trong cùng một syscall). |
| K40 | CLOSED | r251: **vá nửa sửa được** — `best_in_queue` + đường steal nay chỉ chọn TCB `Ready` (trước đây chọn cả `Suspended`/`Blocked` trong runqueue). Nửa còn lại ("ghi đè `current`") **chứng minh không thể thành bất biến**: `Running` mà không có `current` là trạng thái hợp lệ — thêm bất biến đó làm 3 test sẵn có đỏ. Test: `r251_pick_on_skips_non_ready_entries`, `r251_running_without_current_is_a_valid_state`. |
| K41 | CLOSED | `lib.rs:4212` vùng — destroy VSpace gỡ hàng + current |
| K42 | CLOSED | r260 (T22, D18): `recv` nay poll `bound_ntfn` trước khi chặn — có bit chờ thì trả ngay event và tiêu thụ đúng một lần. Test đỏ-trước: `recv_polls_the_bound_notification_before_blocking` (bỏ nhánh poll ⇒ test đỏ). |
| K43 | CLOSED | `ipc.rs:531` — cấm bind khi Notification đã bound nơi khác |
| K44 | CLOSED | `ipc.rs:540` — bind kiểm object sống/đúng kiểu |
| K45 | CLOSED | r260 (T22, D18): **không phải lỗi**. Dựng đúng ca hỗn hợp (hàng `[stale, live]`, word có bit cũ): waiter sống nhận đủ bit cũ+bit mới, waiter treo không bị đánh thức, không bit nào sót lại. Test: `signal_mixed_stale_and_live_waiters_loses_no_bit`. |
| K46 | CLOSED | r260 (T22, D18): `poll` tiêu thụ word nay hạ `pending` của mọi IRQ trỏ tới notification đó, nên cạnh kế tiếp không bị coalesce ⇒ hết mất ngắt. Test đỏ-trước: `poll_clears_irq_pending_so_the_next_edge_is_delivered` (bỏ bản vá ⇒ test đỏ). |
| K47 | CLOSED | `lib.rs:808,825,861` — chặn IRQ ≥ 64 thay vì `& 63` |
| K48 | CLOSED | `ipc.rs:107,197` — trả về đầu hàng khi grant fail |
| K49 | CLOSED | `ipc.rs:88,155,202` — snapshot object thay vì chỉ số slot |
| K50 | CLOSED | r251: hệ quả của K37–K39 (không phải lỗi) và K45 (còn OPEN, độc lập). Không còn triệu chứng nào quy về K50. |
| K51 | CLOSED | `ipc.rs:531` — chỉ cấm khi đã bound nơi khác |
| K52 | CLOSED | `lib.rs:3873` vùng — thứ tự flush/unmap |
| K53 | CLOSED | `lib.rs:2585` vùng — giữ `harts_loaded` sau flush lớn |
| K54 | CLOSED | `lib.rs:2341` vùng — `map_page_table` shootdown theo mask |
| K55 | CLOSED | `lib.rs:2527,420` — khoá W^X chéo CPU/DMA |
| K56 | CLOSED | `lib.rs:4233` vùng — destroy IOMMU domain gỡ mapping |
| K57 | CLOSED | `lib.rs:5899,4709` — FrameWrite kiểm `mapped_x` |
| K58 | CLOSED | `lib.rs:3565` vùng — unmap page-table kèm flush |
| K59 | CLOSED | `lib.rs:660` vùng — chặn PA trùng khi tạo frame |
| K60 | CLOSED | r260 (T22, D18): kernel ghi `harts_seen` (tick, `pick_on`, nạp ASID) và `check_invariants` bác bit `harts_loaded` cho hart CHƯA TỪNG chạy. Test đỏ-trước: `check_invariants_detects_phantom_harts_loaded_bit`. |
| K61 | CLOSED | `lib.rs:3769` vùng — orphan bảng con có flush span |
| K62 | CLOSED | `lib.rs:1122` (v101) — **cấm bind CNode khi còn vé nhúng sống** |
| K63 | UNREACHABLE | Hệ quả K62: không thể có vé nhúng ẩn ⇒ `delete_cap` không còn mù |
| K64 | UNREACHABLE | Hệ quả K62: `other_caps_to` không còn bỏ sót vé ẩn |
| K65 | UNREACHABLE | Hệ quả K62: invariant không còn mù vé nhúng ẩn |
| K66 | CLOSED | `lib.rs:3697` vùng — `remove_caps_to` quét mảng nhúng |
| K67 | CLOSED | `lib.rs:3770` (v101) — destroy gỡ `bound_ntfn` của TCB |
| K68 | CLOSED | `lib.rs:793` vùng — revoke gỡ routing IRQ |
| K69 | CLOSED | `lib.rs:3201` vùng — chặn xoá Untyped cuối khi còn object |
| K72 | CLOSED | `lib.rs:6583` (v101) — `FrameWrite` phát `len` đã kẹp |
| K73 | CLOSED | `lib.rs:5214` vùng — `IoMap` kiểm quyền frame |
| K74 | CLOSED | `lib.rs:5232` vùng — `IoUnmap` đòi vé Frame + khớp pa |
| K75 | CLOSED | `lib.rs:5646,5655` vùng — `TcbConfigure` kiểm quyền/liveness |

**Tổng K:** 43 CLOSED · 3 UNREACHABLE · **0 OPEN** (r260 đóng K42, K45, K46, K60).

## 2. round82 — mục "Còn mở"

| Mục | Trạng thái | Ghi chú |
|---|---|---|
| `pick_on` chưa lọc `state == Ready` | **OPEN** | = K40. Audit tự ghi "cần đường tái hiện trước khi sửa" |
| `KERNEL_STATIC_BUDGET` rộng hơn gap 8 byte AArch64 | **CLOSED** | r229: gap nay **4104 byte** + canary + build FAIL khi < 4096 |
| `slot_arg(..).unwrap_or(0)` ở 53 chỗ | **OPEN** | Chưa đổi; rủi ro thấp (giá trị 0 rơi vào slot không tồn tại) |
| Fuzzer chạy trong gate | CLOSED | Gate step `fuzz` (2 test `#[ignore]`, có `tests-present`) |

## 3. round88 — P1/P2 còn lại

| Mục | Trạng thái | Ghi chú |
|---|---|---|
| P1-1 PIC EOI / spurious IRQ7 | CLOSED | Đã sửa v88 (`x86_irq_handler` + `ack_timer` hart-guard) |
| P1-2 spurious vector 0xFF thiếu IDT | CLOSED | Stub thêm v88 |
| P1-3 `fault::suspend` không lưu trap frame | CLOSED | v92 `TrapFrame::from_raw` |
| P1-4 x86 idle-entry race `AP_IDLE_WINDOW` | CLOSED | v88/150 `post_idle_ready` |
| P1-5 `enqueue` trả sớm ⇒ Ready nhưng không trong runqueue | **OPEN** | Cần đọc lại `enqueue`; liên quan K37–K40 |
| P1-6 aarch64 SGI 2 chưa enable trong GICD | **OPEN** | Đường panic-hart1 aarch64 vẫn PASS nên chưa kích hoạt |
| P1-7 `setup_ap_gdt_tss_idt` hard-code TSS[1] vs `MAX_HARTS=4` | **OPEN (P1)** | = audit92 §1; cần vòng riêng cho GDT/TSS per-hart |
| P2 `--quick` bỏ cả docs mà chỉ nói QEMU/persist | CLOSED | `gate.sh` nay in rõ "--quick: QEMU and persistence were SKIPPED" |

## 4. round92 — x86 2 hart + fault frame

| Mục | Trạng thái | Ghi chú |
|---|---|---|
| Hart index hard-code (`TSSS[1]`, `CPU_STATES[1]`, `pick_on(1)`) | **OPEN (P1)** | Latent: chỉ hart 1 dùng được trampoline |
| **3 assert vacuous trong `ap_entry`** | **CLOSED (r233)** | Thay bằng (a) APIC ID đọc từ LAPIC phải khớp `AP_APIC_ID`, (b) `GS_BASE` phải nằm trong `CPU_STATES`; thêm 3 const assert cho GDT 7 entry. Ba assert cũ đọc lại giá trị vừa ghi nên **không thể fail** (D11) |
| `fault_frame_complete()` | CLOSED | v92 |
| `AP_GDT` chưa nằm trong `kernel_data_ranges` ⇒ executable dưới NX | **OPEN (P2)** | Rủi ro thấp nhưng đúng là lỗ hổng NX |

## 5. Việc phải làm tiếp (đề xuất thứ tự)

1. ~~**K40 + P1-5**~~ — **XONG**: K40 vá nửa sửa được ở r251, nửa còn lại chứng
   minh không thể thành bất biến; r259 (T21/D19) thêm phép kiểm ở ranh giới user.
2. ~~**K37/K38/K39**~~ — **XONG** (r251): không phải lỗi, `current` cố ý giữ TCB đã
   park cho tới khi runtime lưu context.
3. ~~**K42/K45/K46**~~ — **XONG** (r260/T22): `recv` poll `bound_ntfn`; `signal`
   chứng minh không mất bit ở ca hỗn hợp; `poll` hạ `pending` của IRQ.
4. ~~**K60**~~ — **XONG** (r260/T22): `check_invariants` bác `harts_loaded` "ma"
   bằng mask `harts_seen`.
5. **P1-7 / audit92 §1** — GDT/TSS per-hart (T24, đụng trampoline; đang làm).
6. **P2** — đưa `AP_GDT` vào `kernel_data_ranges`.

> Ghi chú: 5 audit core kernel đang chạy song song (theo `bao-cao-agent0-vong-224.md`) chưa trả kết quả; khi về thì hợp nhất vào bảng này, mỗi P0 một vòng kèm test hồi quy.
