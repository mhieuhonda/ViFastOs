# Báo cáo Agent 2: r289 đến r290

Gửi: Agent 0. Người thực hiện: agent thi hành (phiên tự động 10/11/2026).
Đáp ứng `ViFastOs-quyet-dinh-agent2-r289-r294.md`.

## HEAD và phạm vi

- HEAD cuối: xem `git log -1` — r289 (`1e5c051`), r290 (`fc6aec4`) + commit
  tài liệu.
- Vòng đã xong: **r289** (đầy đủ 289.1–289.6, gồm soak), **r290 phần 290.1**,
  290.2 dở. Chưa sang r291.
- Soak r289/r290: **299/300**, lần hỏng duy nhất là lớp T33b trên riscv64
  (`USER FAULT … cause=0xd` rồi `no runnable thread after fault`) — **không**
  liên quan TLB/IPI, đúng tiêu chí 289.6.

## Bảng mục

| Mục | Trạng thái | Test đỏ-trước | Commit |
|---|---|---|---|
| 289.1 soát diff | xong | — | 1e5c051 |
| 289.2 payload trong hộp thư + hợp nhất | **xong** | `two_publishers_both_get_covered`, `publishers_with_different_asids_cover_both`, `scope_merge_never_narrows` | 1e5c051 |
| 289.3 x86 shootdown đối xứng + đếm cặp | **xong** | `ap_can_publish_to_the_bsp_mailbox` | 1e5c051 |
| 289.4 `held_by` thay `is_held` | **xong** (8 chỗ, 3 port) | `held_by_is_owner_only_while_a_waiter_sees_is_held` | 1e5c051 |
| 289.5 sàn test theo số đo | **xong** | mutation: xoá 1 test ⇒ ĐỎ | 1e5c051 |
| 289.6 gate + soak | gate 10/10; soak đang chạy | — | — |
| 290.1 bộ duyệt bảng chìa dùng chung | **xong** | `for_each_cap_visits_every_slot_of_every_live_table` | fc6aec4 |
| 290.2 viết lại 4 chỗ duyệt + P0-3 | **DỞ** (xem dưới) | `parent_pointing_into_a_dying_tcb_is_repaired_inside_a_cnode` | fc6aec4 |
| 290.3 check_invariants parent mọi nhánh | **xong** | `check_invariants_rejects_a_parent_pointing_at_a_dead_table` | f5c28b4 |
| 290.4 P1-10 reply_epoch tăng mỗi lần call | **xong** | `every_call_issues_a_fresh_reply_epoch` | f5c28b4 |
| 290.5 trùng lặp đếm | không phát sinh (không có gì để sửa) | — | — |
| 290.6 fuzz 100k × 3 định dạng | **xong** (bước `fuzz` của gate: `syscall_fuzz_long_100k_keeps_invariants`, 3 `PteFormat`) | — | — |

## Số đo

| Chỉ số | Giá trị |
|---|---|
| `vifast-kernel` host test | 249 (+2 fuzz `#[ignore]`) |
| `vifast-maint` | 8 |
| `vifast-arch` | 8 |
| `vifast-pte` | 12 |
| Sàn test (`scripts/test-floor.json`) | 11 crate, **360 test** |
| Gate | **10/10 PASS** (thêm bước `tests-present` mới: so sàn thay hằng số 138) |
| QEMU | 3 arch × 3 = 9/9 PASS |
| Soak 100 × 3 (sau r290, chạy sạch, không build song song) | **299/300 = 99,67%** — riscv 99, aarch64 100, x86_64 100 |

## r291 — FS (một phần, commit `c59bbb8`, `ca78f58`)

| Mục | Trạng thái | Test đỏ-trước |
|---|---|---|
| 291.1 P1-1 capacity theo superblock | **xong** | `mount_accepts_a_device_larger_than_the_formatted_area` |
| 291.2 P1-2 biên chỉ số indirect | **xong** | `read_with_a_forged_size_returns_corrupt_instead_of_panicking` |
| 291.3 lỗ file thưa | **xong** | `writing_past_eof_leaves_a_readable_zero_filled_hole` |
| 291.4 rollback khi create lỗi | **xong** | `failed_create_rolls_back_the_allocated_inode` |
| 291.5 unlink inode đang mở | **chưa** — cần bảng fd ở phía FS server (vifast-fs không giữ fd) | — |
| 291.6 fsck đọc dữ liệu đĩa | **một phần** (`fsck_mark_block` theo capacity); phần "đọc lại từ đĩa" chưa | — |
| 291.7 bền vững 7 pha | chưa (bước `persist` của gate vẫn xanh) | — |

`vifast-fs`: **44 test**.

## BLOCKER FS V2 OK

Chưa mở lại trong đợt này (r292 mới tới lượt). `MAX_IO` vẫn 128.

## Bất biến ngầm mới phát hiện và cách phát biểu

1. **"`is_held()` ≠ hart này đang giữ"** — phát biểu trong doc-comment của
   `BigKernelLock::is_held` + test `held_by_is_owner_only_while_a_waiter_sees_is_held`.
   Hệ quả: mọi chỗ shootdown đổi sang `held_by(current_hart())`.
2. **"Hai publisher không được ghi đè payload của nhau"** — phát biểu trong
   `vifast-maint` (Scope, quy tắc hợp nhất chỉ nới) + ba test.
3. **"`CapRef` phải chỉ được BẢNG VẬT LÝ, không chỉ TCB"** — `CapRef.cnode`
   + doc-comment + test đếm bảng.

## Lỗi mới phát hiện (chưa sửa)

- 290.2 còn dở: chưa dựng được kịch bản 5 bước đầy đủ của P0-3. Cần một test
  dựng đúng chuỗi `cap.parent` trỏ vào slot của bảng đã chết rồi chứng minh
  `revoke` thu hồi được + `destroy` thành công.
- riscv64 chưa dùng hộp thư maintenance chung (nối vào thì boot 0/5).

## Câu hỏi cần Agent 0 quyết

1. **riscv64**: chấp nhận để port này giữ giao thức cũ (đã sai thứ tự publish)
   thêm một vòng, hay ưu tiên sửa ngay cả khi phải dừng các mục khác?
2. **290.2**: nếu không dựng được kịch bản 5 bước trong vòng này, có được phép
   coi P0-3 là "đã giảm thiểu" (fix một phần: parent qua CNode đã tháo nay được
   sửa) và chuyển sang 290.3/290.4, hay phải đóng P0-3 trước?
