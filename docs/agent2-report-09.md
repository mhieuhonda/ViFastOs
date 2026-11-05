# ViFastOs — Báo cáo Agent 2, vòng 09

**Ngày:** 2026-11-05  
**Baseline:** `b9bfad9` (code), bundle vòng 08

## 1. Bảng task

| Task | Trạng thái | Commit |
|---|---|---|
| 1.0 Tái hiện lỗi AArch64 | ❌ chưa làm | — |
| 1.1 AArch64 GIC/timer CPU1 | ✅ gộp với 1.2 | `c7c45ed` |
| 1.2 AArch64 idle WFI | ✅ | `c7c45ed` |
| 1.3 Worker CPU1 AArch64 | ❌ chưa bắt đầu | — |
| 2. Docs checker | ✅ một phần | `3631ed7`, docs cuối vòng |
| 3. Panic toàn hệ thống | ❌ chưa làm | — |
| 4. Đo BKL | ❌ chưa làm | — |
| 5. ACPI x86 | ❌ chưa làm | — |

1.1 và 1.2 được gộp vì cùng một vòng idle; harness AArch64 kiểm tra cả
`SMP TIMER OK` và `SMP IDLE OK`.

## 2. Test

```text
./scripts/build.sh                 PASS, không warning
vifast-kernel                      87/87
vifast-arch                         5/5
vifast-pte                         11/11
vifast-elf                         17/17
QEMU AArch 1.1+1.2                 10/10
test-persist.py                    RISC-V/AArch64/x86_64 PASS
cargo fmt --check                  PASS
scripts/check-docs.py              PASS
```

AArch64 10 lần:

```text
T = 20,20,20,20,20,20,20,20,20,20/21 (hart1_ticks)
K = 1 cả 10 lần (hart1_wakeups)
```

## 3. Task 1 — AArch64 CPU1

### 1.0

Chưa làm tái hiện lỗi cũ. Không có dòng `KERNEL EXCEPTION` mới.

### 1.1 + 1.2

- `smp.S`: `msr daifset, #0xf` trước khi vào Rust.
- CPU1 tự cấu hình GIC banked:
  - GICD enable + PPI 30 + SGI 0;
  - GICC_PMR=0xff, GICC_CTLR=1.
- Timer `CNTP_TVAL_EL0`/`CNTP_CTL_EL0` riêng; interval tính tại chỗ.
- Vòng idle:
  - assert `DAIF.I == 1` mỗi vòng;
  - tăng `idle_entries`;
  - `wfi`;
  - polling `GICC_IAR` đến 1023;
  - PPI 30: tăng `hart1_ticks`, nạp lại timer;
  - SGI: tăng `ipi_wakes`, `poll_maintenance`;
  - ghi `GICC_EOIR`.
- Runtime in `SMP TIMER OK (hart1_ticks=T)` và `SMP IDLE OK`.
- Harness AArch64 bắt buộc mỗi dòng đúng 1 lần, T/K > 0.

1.3 worker chưa làm. Lý do: cần BKL owner per-hart, current_on(1), enter_user
trên CPU1 và reschedule SGI; thời gian còn lại ưu tiên ổn định 1.1/1.2 và
Task 2.

## 4. Task 2 — Docs checker

`check-docs.py` hiện:
- exact commit order từ `git log --reverse 456d5ef..HEAD`, bỏ `docs:`;
- headroom mới;
- positive SMP/TcbSetAffinity strings;
- forbidden strings.

Chưa đạt đầy đủ yêu cầu:
- chưa parse toàn bộ enum `SyscallId`;
- harness chưa tách danh sách dùng chung; checker vẫn kiểm tra anchor;
- chưa đọc trực tiếp file kết quả cargo test.

Lần chạy đầu script (bản trước) báo:
1. `hart phụ idle` còn trong summary.
2. Thiếu commit `43d5200` trong danh sách.
Sau đó phát hiện thêm thứ tự commit và headroom lệch; đã sửa.

## 5. Headroom / SHA

```text
RISC-V kernel_end = 0x80390120
headroom          = 458336 byte
summary SHA-256   = 1f01502cbd99179c95ee9bbd7e8bc49620d48b0df80eaf6bb1bf411ed3ad2d9d
```

## 6. Nợ

- 1.0 tái hiện, 1.3 worker AArch64.
- Checker chưa tự parse SyscallId/harness như yêu cầu.
- Panic toàn hệ thống, BKL stats, ACPI x86 chưa làm.
- RISC-V hiếm khi gặp timeout/flake khi chạy 10 lần; cần theo dõi.
- x86 AP, per-hart runqueue, per-VA TLB, FS v2, IOMMU phần cứng, net/input,
  compositor vẫn ngoài vòng.
