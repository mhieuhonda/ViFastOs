# Nhật ký quyết định tự động của agent2

| Vòng | Quyết định | Lý do | Commit |
|---|---|---|---|
| 26 | Revert A5/A1 khi QEMU đỏ, giữ baseline | Quy tắc kẹt 2.5 | — |
| 26 | Sửa display design theo C4 | T6: capability-safe device memory | 446a5c3 |
| 27 | Làm M0.2 (C1/C2/C3) trước M1.1 | Rủi ro thấp, docs đúng trước refactor | 91296ee,e69230e,d793786 |
| 27 | M1.1 chỉ commit phần đo, HWM sang inbox | AArch/x86 crash 3 lần | 216035d |
| 28 | M1.2 tách bước a/b/c, sửa runtime context indexing | ROOT ERR 16 do runtime dùng tcb.index | 91f7496 |
