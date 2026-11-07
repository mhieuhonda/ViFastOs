# ViFastOs — Báo cáo vòng 87 (đóng rò rỉ thanh ghi FP/SIMD giữa task)

**Ngày:** 2026-11-07
**HEAD đầu vòng:** `8d06b16`
**Bối cảnh:** P1-2 của audit #3: kernel không lưu FP/SIMD khi switch task.

## 1. Vấn đề

- aarch64: trap.S chỉ lưu x0..x30, nhưng CPACR được set FPEN=0b11 nên **EL0
  cũng dùng được FP/SIMD**. Task B đọc q0..q31 thấy giá trị còn sót của task A.
- x86_64: trap.S không lưu xmm; OVMF bật OSFXSR nên ring 3 dùng SSE được.
- riscv64imac **không có F/D**, nên user không thể dùng FP — không rò rỉ.

## 2. Cách sửa: chặn thay vì lưu

Lưu trữ FP per-task sẽ tốn 512 byte × 147 object → vượt ngân sách static
(AArch64 chỉ còn 8 byte tới BKL). Vì vậy kiểm tra thực tế trước:

```
llvm-objdump -d target/*/root-task/vifast-client | grep -c 'ldr q|str q|fmov|xmm'
→ 0 cho cả aarch64, x86_64 và riscv64
```

Không ELF user nào (và kernel x86) dùng FP/SIMD, nên:

- **aarch64**: CPACR `FPEN = 0b01` (EL1 dùng được, EL0 trap). FP của user
  tương lai sẽ trap rõ ràng thay vì rò rỉ.
- **x86_64**: `disable_sse()` xoá CR4.OSFXSR + OSXMMEXCPT, gọi cho **cả BSP
  và AP** (CR4 per-hart); log `SSE USER OFF` / `SSE USER OFF (bsp)`.

## 3. Phụ

- `soak.py`: kết quả `(0,0)` trước đây được coi là PASS (`passed == total`);
  nay coi là hỏng vì nghĩa là bước đó không thực sự chạy.

## 4. Kiểm chứng

- aarch64: `--repeat 2` PASS; x86_64: `--repeat 2` PASS, serial có
  `SSE USER OFF` đúng 2 lần (BSP + AP).
- `gate.sh --with-tcp --with-input`: all steps passed.
- Ghi chú: audit #3 còn P2 `send_ipi` no-op trên aarch64/x86 — đã kiểm tra,
  `wake_hart` dùng đường phần cứng thật (GIC SGIR / LAPIC ICR), `Arch::send_ipi`
  hiện không có caller; sẽ dọn hoặc hiện thực ở vòng sau.
