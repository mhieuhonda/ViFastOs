# ViFastOs — Báo cáo vòng 105 (gate sau mốc 1-9: 20 vòng lõi)

**Ngày:** 2026-11-08. **HEAD:** xem `git log`.

## 1. Tổng kết 20 vòng (mốc 1 → 9, 11)

- Mốc 1: Exit theo task + `Exited` + `exit_current_on` + `TcbWait` rendezvous
  (syscall 51) + chặn destroy khi có waiter + exit-demo QEMU (worker F,
  `WAIT REAP OK` cả 3 arch) + 5 test hồi quy (exit wake client, wait,
  sleep-exit, destroy-dọn-sạch, restart-tái-sinh, syscall-lỗi-value-0).
- Mốc 5: Reply mang cap GRANT + 4 fix park/runq họ K32/K35.
- Mốc 3: hồi quy journal txn 16 block + FS→ELF→parse (bước 1 exec).
- Mốc 8: shell 256B + lịch sử 4 + Ctrl-C/L/P/N + interactive echo test.
- Mốc 11: runbook 6 sự cố thật.
- Đo đạc: static budget còn 3328 B; BKL hart1 spin ~247/acq (baseline khóa mịn).

## 2. Gate hiện tại

- Kernel 164, FS 27, ABI 4; QEMU 3/3×3; persist 7×3; harness 514;
  docs + fmt PASS; 0 warning. `gate.sh --quick` PASS.
- 2 bài học hạ tầng: x86 cạn data pages cuối boot (spawn tối giản);
  `TcbWait` race Ok/ObjectDestroyed đều đúng.

## 3. Việc tiếp theo (ưu tiên)

1. Mốc 2: refactor cspace (nới static budget) → owner cho destroy-cả-nhóm,
   buffer shell/exec từ Untyped.
2. Mốc 5: per-hart runq lock (đo lại BKL).
3. Mốc 9: exec MVP (cần 1 sau khi có 1) → shell `run`.
