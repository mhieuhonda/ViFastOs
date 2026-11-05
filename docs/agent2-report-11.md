# ViFastOs — Báo cáo Agent 2, vòng 11

**Ngày:** 2026-11-05  
**Baseline:** `014ee65` (code), bundle vòng 10

## 1. Bảng bước

| Bước | Trạng thái | Commit |
|---|---|---|
| 1. Kiểm tra/phục hồi harness | ✅ | `17aee2a` |
| 2. Timer đánh thức hart phụ khỏi WFI | ✅ | `d8424e5` |
| 3. Boot-hart fallback có test tiêm lỗi | ✅ | `b122c05` |
| 4. Syscall status + status block + docs | ✅ | `32c2e03` + docs vòng 11 |
| 5. ACPI MADT x86 | ❌ chưa làm | — |
| Tùy chọn BKL stats / panic toàn hệ thống | ❌ chưa làm | — |

Không gộp commit giữa các bước.

## 2. Test cuối

```text
./scripts/build.sh                     PASS, không warning
vifast-kernel                          88/88
vifast-arch                             5/5
vifast-pte                             11/11
vifast-elf                             17/17
test-qemu.py --repeat 10               10/10 cả 3 kiến trúc
test-qemu.py --arch riscv64 --repeat 50        50/50
test-qemu.py --arch aarch64 --repeat 50        50/50
test-qemu.py --arch riscv64 --feature test-boot-handoff --repeat 10  10/10
test-harness-spec.py                   166 case âm PASS
test-persist.py                        RISC-V/AArch64/x86_64 PASS
cargo fmt --all -- --check             PASS
scripts/check-docs.py                  PASS
```

Headroom: `454192` byte, `kernel_end=0x803911d0`.

## 3. Task 1 — Harness

So sánh `3631ed7` (trước tách spec) với vòng 11:

| Nhóm điều kiện | Bản cũ | Vòng 11 |
|---|---|---|
| `BLOCK OK`, `FS DISK OK` cả 3 kiến trúc | có trong `run_qemu` | khôi phục trong `COMMON` của spec |
| `SMP SKIP` x86 | có trong output contract, chưa thành assertion | thêm `ARCH["x86_64"]` |
| `KERNEL DATA NX OK` x86 | có | giữ trong spec |
| `SMP OK`, `ipi=1` RISC/AArch | có | giữ trong `ARCH` |
| 4 USER FAULT + 3 địa chỉ | có | giữ trong `accept()` |
| `ROOT SWITCH n > 1` | có | giữ trong `accept()` |
| `ROOT PANIC`, `STALL`, `PERCPU CLOBBER BAD`, `VFS BAD` | có | giữ trong `FORBIDDEN` |
| exit code 0/0/3 | `ok_codes` trong từng test | thêm `EXIT_CODES` và sinh vào docs |
| `SMP IDLE`, `SMP SCHED`, `SMP WORKERS` | RISC; AArch từ vòng 10 | giữ, thêm `SMP POST IDLE OK` |

`scripts/harness_spec.py` giờ là nguồn duy nhất: `test-qemu.py` gọi
`accept(...)`, generator in mọi điều kiện, `check-docs.py` kiểm tra số dòng
sinh ra khớp spec (60 điều kiện). `scripts/test-harness-spec.py` có **166 case
âm**: bỏ từng chuỗi bắt buộc, duplicate exact-one, sai fault/root switch/
exit code, sai N/M/W/S/I, sai Tp/Ip/Wtp, thiếu BLOCK/FS, và kiểm tra
`BOOT HANDOFF OK` chỉ chấp nhận ở feature mode.

Lỗi R1 là thật: bản vòng 10 thiếu `BLOCK OK`, `FS DISK OK`, `SMP SKIP` x86
và exit code trong spec sinh tự động. Đã khôi phục, không nới điều kiện nào.

## 4. Task 2 — Post-worker timer/WFI

Worker sau 50 Yield gửi message rồi **Wait** trên notification slot 2 không bao
giờ được signal. Khi cả hai block, kernel bắt đầu pha `post`:

```text
SMP POST IDLE OK (ticks=Tp, idle_entries=Ip, wake_timer=Wtp)
```

Điều kiện: `Tp >= 5`, `Wtp >= Tp - 2`, `Ip >= Tp - 2`. Test-exit trên hart 0
chỉ thoát khi `post_idle_ready()`; vì vậy pha post có đủ thời gian.

10 lần đầu trên mỗi kiến trúc (Tp, Ip, Wtp):

| Lần | RISC-V | AArch64 |
|---:|---|---|
| 1 | 21, 22, 21 | 21, 22, 21 |
| 2 | 18, 19, 18 | 21, 22, 21 |
| 3 | 20, 21, 20 | 21, 22, 21 |
| 4 | 20, 21, 20 | 21, 22, 21 |
| 5 | 20, 21, 20 | 20, 21, 20 |
| 6 | 21, 22, 21 | 18, 19, 18 |
| 7 | 20, 21, 20 | 18, 19, 18 |
| 8 | 19, 20, 19 | 20, 21, 20 |
| 9 | 21, 22, 21 | 21, 22, 21 |
| 10 | 21, 22, 21 | 21, 22, 21 |

`--repeat 50` cả RISC-V và AArch64: **50/50**.

`SMP IDLE OK` tổng vẫn giữ `T > 0`, `I >= 1`; bỏ kiểm tra vô nghĩa
`K >= Wt + Ws`. Khi worker bận, timer đến khi hart 1 ở user và đi qua vector
IRQ user, không qua WFI; vì vậy `Wt=0` trong pha tổng là hợp lý. Pha post
chứng minh điều ngược lại: sau khi worker block, timer thật sự đánh thức WFI.

## 5. Task 3 — Boot-hart handoff

Thêm feature build-only `test-boot-handoff`; `boot_handoff.S` giả lập OpenSBI
chọn hart 1, sau đó hart 1 chạy đúng nhánh promote hart 0 hiện có. Vì hart 0
có thể chưa kịp stop, nhánh promote retry HSM start hart 0 (tối đa 1000 lần)
trước khi stop hart 1. Flag `__boot_handoff_from` làm runtime in:

```text
Boot HART ID                : 0
ViFastOS booting on riscv64 (hart=0, dtb=0x0)
BOOT HANDOFF OK (from=1)
SMP OK (harts=2, ipi=1)
SMP POST IDLE OK (ticks=21, idle_entries=22, wake_timer=21)
```

`./scripts/build.sh` tạo `vifast-kernel-riscv64-handoff` rồi phục hồi binary
thường. `test-qemu.py --feature test-boot-handoff` yêu cầu đúng một dòng
`BOOT HANDOFF OK`; build thường cấm dòng này. Feature 10/10.

## 6. Task 4 — Syscall status và docs

`SYSCALL_STATUS` trong `vifast-abi` là nguồn trạng thái. Generator in
`implemented` hoặc `placeholder (NotImplemented)`.

Placeholder hiện tại:

```text
11 CapMint             12 CNodeMint
13 CapDelete           17 TcbSuspend
18 TcbRestart          19 TcbSetPriority
21 TcbBindNotification 22 SchedContextConfigure
23 SchedContextBind    29 IrqGet
30 IrqSetNotification  31 IrqAck
```

Đã thêm host test `syscall_status_matches_notimplemented_fallback`: mọi syscall
`implemented` không trả `NotImplemented` (và placeholder phải trả
`NotImplemented`) với tham số tối thiểu. Tổng host test kernel tăng 87 → 88.

Mục 2 có vùng `status` sinh từ build log + `target/vifast-qemu-last.json`:
headroom dạng byte/hex và tỉ lệ QEMU gần nhất. Đã bỏ các câu viết tay
`3/3 PASS, lặp 3 lần` và `458.336`.

## 7. Nợ / chưa làm

- Task 5 ACPI MADT x86 chưa làm; x86 vẫn `SMP SKIP`.
- Chưa có INIT/SIPI, per-hart runqueue (vẫn global), khóa mịn, per-VA TLB,
  FS v2, IOMMU phần cứng, net/input, compositor.
- `SMP IDLE OK` tổng trên RISC-V vẫn có `Wt=0` khi worker bận; pha post là
  bằng chứng timer/WFI thay thế.
- Boot-hart fallback chỉ được chứng minh bằng feature injection; chưa thấy
  OpenSBI tự chọn hart 1 trong các lần chạy thường.
