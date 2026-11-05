# ViFastOs — Báo cáo Agent 2, vòng 04

**Ngày:** 2026-11-05  
**Baseline:** `74b7d75` (code), `3fe8ebc` (docs vòng 03)  
**Trạng thái:** Task 1 xong; Task 2–4 **chưa làm**.

## 1. Task 1 — Docs cleanup

Commit:

```text
79132c5 docs: remove stale per-cpu and riscv storage notes
```

Đã sửa summary + handoff:

- Bỏ "RISC-V còn fallback RAM".
- Bỏ các câu "SMP cần remote fence"/"I-cache shootdown chưa có".
- Gộp mục per-CPU thành một mô tả duy nhất.
- Bỏ `ACTIVE_ROOT/ASID` khỏi danh sách static global; ghi chúng nằm trong
  `CpuState`.
- Sửa TOCTOU thành double-fetch nội dung user; BKL đã chặn đổi mapping.
- Thêm `vifast-arch 4/4` vào mục 2.
- Sắp xếp lại commit list.
- Cập nhật SHA-256 trong `docs/overnight-log.md`.

Grep kiểm tra (summary + handoff):

```text
fallback RAM                 0
SMP cần remote fence         0
I-cache shootdown            0
qua `tp`                     0
ACTIVE_ROOT/ASID             0
```

## 2. Task 2–4 — chưa làm

Không có commit cho Task 2, 3, 4.

- Task 2 (mailbox maintenance đối xứng + xác minh revoke 2 pha) chưa triển khai.
- Task 3 (hart phụ RISC-V chạy scheduler) chưa triển khai.
- Task 4 (hart phụ AArch64) chưa triển khai.

Không tuyên bố `SMP SCHED OK`; hart phụ vẫn maintenance-only.

## 3. Test baseline hiện tại

Code không đổi từ vòng 03:

```text
./scripts/build.sh       PASS, không warning
vifast-kernel            84/84
vifast-arch               4/4
vifast-pte               11/11
vifast-elf               17/17
QEMU 3/3 PASS × 3 lần
test-persist.py          3/3 kiến trúc PASS
```

Headroom RISC-V:

```text
kernel_end = 0x80390110
headroom   = 458480 byte
```

## 4. SHA-256

```text
ViFastOs-architecture-summary.txt:
adc576b50821936436a418a349c3ef6af5d814508e312d23b2c215249c297783
```

Code baseline vẫn là `74b7d75`.
