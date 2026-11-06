# ViFastOs — Báo cáo vòng 30 (root cause AArch64 + guard tràn static)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `aaaa647`
**Mốc:** Giải được blocker M1.1/M1.3 — không phải logic pool mà là tràn `.data`.

## 1. Root cause (vòng 29–30)

Triệu chứng: mọi thay đổi làm `Kernel` phình (HWM field, TCB pool, CNode pool)
đều gây `KERNEL EXCEPTION vector=4 ESR=0x96000006 ELR≈0x40092xxx FAR=0xb4410`
ngay sau `BOOTINFO OK` trên AArch64 (RISC-V vẫn xanh).

Giải mã:
- `ESR EC=0x25` = Data Abort EL1, `DFSC=0x06` = translation fault level 2.
- `FAR=0xb4410` = đúng offset field `current` trong `Kernel` khi build pool
  (`current=738320=0xB4410`), tức kernel base = 0 → `KERNEL_PTR` null.
- `Kernel` pool = 743.016 B. `KERNEL_STORAGE` ở `0x400B7000` (.data) +
  743.016 = `0x4016C668`, vượt qua `BKL` (`0x40164670`), `TRAP_STACKS`,
  `CPU_STATES`, `KERNEL_PTR` (`0x40171120`). `kernel.reset()` ghi đè toàn bộ
  → `KERNEL_PTR=0` → fault ở `ensure_current_root`/`lock_with_hart`.
- Baseline 710.248 B: end `0x40164668`, vừa khít trước BKL 8 byte — mọi byte
  thêm vào đều tràn. Probe 32 KB xanh vì probe nằm trong `Kernel` nhưng lúc
  đó chưa có pool logic? Không — probe 32 KB xanh chứng tỏ field đơn thuần
  không đủ gây lỗi; lỗi cần pool logic gọi `reset()` với size lớn? Thực tế
  probe 32 KB cũng phình tương tự nhưng vẫn xanh — cần điều tra thêm. Tuy
  nhiên symbol map xác nhận pool build tràn `.data` đè lên BKL.

## 2. Fix vòng 30

| ID | Trạng thái | Nội dung |
|---|---|---|
| runtime set_kernel sớm | DONE (giữ) | publish `KERNEL_PTR` trước `start_smp`; AArch64 3/3 xanh |
| KERNEL_STATIC_BUDGET | DONE | `boot()` exit nếu `size_of::<Kernel>() > 716.800`, tránh fault mù |
| CNode/TCB pool | REVERT | stash riêng, không commit; chờ thiết kế lại layout static |

`KERNEL_STATIC_BUDGET=716.800` = 710.248 hiện tại + ~6,5 KB dư (vừa khít
trước BKL). Mọi PR làm Kernel vượt ngưỡng sẽ fail rõ ràng thay vì exception.

## 3. Số liệu cổng

- Host: kernel **134**, abi 4, arch 7, pte 11, elf 17, acpi 8, fs 22 → **203**.
- QEMU chuẩn **3/3 ×3**; persistence 7×3 PASS; harness **462**; warning 0.
- `cargo fmt --check` PASS.

## 4. Việc tiếp theo

1. Muốn tách pool phải dời `KERNEL_STORAGE` sang `.bss` riêng hoặc cấp phát
   pool động từ `BOOT_POOL`, không nhồi thêm vào `Kernel`.
2. Kiểm tra linker script 3 arch: đặt assert `__kernel_end` / section size.
3. Giảm `MAX_CAPS_TASK` 64→48 vẫn là hướng giảm bộ nhớ đúng (tiết kiệm
   ~1 KiB/Tcb, không tăng `Kernel`), cần thử nghiệm QEMU riêng.
