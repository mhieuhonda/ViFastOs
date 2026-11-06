# Nhật ký quyết định tự động của agent2

| Vòng | Quyết định | Lý do | Commit |
|---|---|---|---|
| 26 | Revert A5/A1 khi QEMU đỏ, giữ baseline | Quy tắc kẹt 2.5 | — |
| 26 | Sửa display design theo C4 | T6: capability-safe device memory | 446a5c3 |
| 27 | Làm M0.2 (C1/C2/C3) trước M1.1 | Rủi ro thấp, docs đúng trước refactor | 91296ee,e69230e,d793786 |
| 27 | M1.1 chỉ commit phần đo, HWM sang inbox | AArch/x86 crash 3 lần | 216035d |
| 28 | M1.2 tách bước a/b/c, sửa runtime context indexing | ROOT ERR 16 do runtime dùng tcb.index | 91f7496 |
| 28 | HWM thử lần 4 (runtime read-only) vẫn lỗi AArch → BLOCKED, đi M1.3 | Quy tắc 2.5: 2 phiên BLOCKED thì đổi nhánh | — |
| 28 | Thử tách TCB pool, revert vì AArch 0/3 | Baseline xanh ưu tiên; lỗi AArch cần debug riêng | — |
| 29 | Wrap CPU_STATES + KERNEL_STORAGE AArch bằng UnsafeCell, không đổi hành vi | Giảm static mut, QEMU 3/3×3 + check-all xanh | d266ab7,36a5b91 |
| 29 | Stash CNode pool WIP, chỉ commit layout probe | CNode pool AArch 0/3; tách riêng để giữ baseline xanh | stash@{0},6682fa5 |
| 30 | Root cause tràn .data; guard KERNEL_STATIC_BUDGET; revert pool | Kernel pool đè KERNEL_PTR; exit rõ thay vì exception mù | (vòng này) |
| 31 | Giảm MAX_CAPS_TASK 64→48 sau audit slot | User max 11, test tới 36; tiết kiệm 150 KB Kernel | d6272f1 |
| 32 | Budget dùng chung kernel/runtime; mở goal dài hạn | Một ngưỡng duy nhất, tránh lệch; pha 1 M1→M3 | 25ac72c |
