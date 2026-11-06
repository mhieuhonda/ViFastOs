# ViFastOs — Báo cáo vòng 57 (tổng kết pha 4 display, giữ nguyên code)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `37b1bdc`
**Mốc:** Tổng kết pha 4 ở mức hạ tầng probe đã kiểm chứng, không đổi code.

## 1. Pha 4 đã đạt (vòng 52–56)

| Vòng | Nội dung |
|---|---|
| 52 | gắn `ramfb` cả 2 arch + probe fw_cfg sig `0x554d4551` cả 2 arch |
| 53 | ghi selector fw_cfg treo guest → revert sig-only (bài học sequencing) |
| 54 | probe DTB magic trong `init_platform` — RISC-V `0xd00dfeed`, ARM `0x0` |
| 55 | header FDT OK nhưng quét struct treo → giữ header-only |
| 56 | header FDT đầy đủ v17 (`size=10319 strings_off=5716`) trong vùng an toàn |

Trạng thái: ramfb gắn, fw_cfg/DTB đọc được ở mức header trên RISC-V.
Chưa có: map DTB trên ARM, copy DTB parse framebuffer, vẽ pixel, `DISPLAY OK`.

## 2. Vì sao chốt ở đây

Parse sâu (fw_cfg selector / FDT struct) treo guest ở boot sớm — cần làm ở
user-space hoặc sau MMU ổn định, là việc lớn vượt một vòng. Theo quy tắc
vertical slice + bảo toàn gate, chốt hạ tầng probe đã xanh thay vì mở rộng
khi chưa có test bảo vệ.

## 3. Số liệu cổng (giữ nguyên vòng 56)

- Host 203, build 0 warning (+check static PASS, khe hở 8B), `fmt` PASS.
- QEMU 3/3×3, persist 7×3, harness 483, docs PASS.
