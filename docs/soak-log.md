# ViFastOs soak log — vòng 19

Một chu kỳ soak cuối (2026-11-05):

```text
test-qemu.py --arch riscv64  --repeat 50  50/50 PASS
test-qemu.py --arch aarch64  --repeat 50  50/50 PASS
test-qemu.py --arch x86_64   --repeat 50  50/50 PASS
test-skip-tlb-ipi riscv64 --repeat 5       5/5 PASS (mutation detected)
test-skip-tlb-ipi x86_64  --repeat 5       5/5 PASS (mutation detected)
test-panic-hart1 5/5 mỗi kiến trúc         PASS
test-boot-handoff --repeat 10             10/10 PASS
test-persist.py                           7 phase × 3 PASS
test-harness-spec.py                      327 case âm PASS
test-qemu.py --repeat 10                  10/10 cả 3 kiến trúc
./scripts/build.sh                        0 warning
```

FAIL: 0. Artifact: không có. Ghi chú: các lần chạy 50/50 nhạy cảm với việc
sửa binary giữa chừng; chu kỳ trên dùng đúng binary của commit `8342553`.

## Vòng 20 — per-hart scheduler + VFS bulk + hard link

```text
q50 riscv64/aarch64/x86_64             50/50 mỗi kiến trúc
test-skip-tlb-ipi 5 lần                RISC 5/5, x86 5/5 (expected fail detected)
panic-hart1 5 lần                      5/5 mỗi kiến trúc
boot-handoff --repeat 10              10/10
test-persist 7 phase                   3/3 kiến trúc, hard-link readback
test-qemu --repeat 10                  10/10 mỗi kiến trúc
test-harness-spec                      336 case âm
```

FAIL: 0. Artifact: không có sau khi dọn. Binary dùng đúng commit `925f633`.
