# ViFastOs — Báo cáo Agent 2, vòng 12

**Ngày:** 2026-11-05  
**Baseline:** `32c2e03` (code), bundle vòng 11

## 1. Bảng bước

| Bước | Trạng thái | Commit |
|---|---|---|
| 1. Bounded boot handoff + docs rules | ✅ | `1ff8644`, `e6309c3` |
| 2. ACPI MADT x86 | ✅ | `9b8f031` |
| 3a. LAPIC BSP | ✅ | `627f054`, `83875a7` |
| 3b. AP trampoline/long mode | ❌ chưa làm | — |
| 3c. Maintenance AP + SMP OK x86 | ❌ chưa làm | — |
| Tùy chọn BKL/panic | ❌ chưa làm | — |

## 2. Test cuối

```text
./scripts/build.sh                       PASS, không warning
vifast-kernel                            88/88
vifast-arch                               5/5
vifast-pte                                11/11
vifast-elf                                17/17
vifast-acpi                               8/8
QEMU --repeat 10                          10/10 cả 3 kiến trúc
RISC-V boot-handoff --repeat 10           10/10
x86_64 --repeat 10                        10/10
test-harness-spec.py                      172 case âm PASS
test-persist.py                           PASS 3 kiến trúc
cargo fmt --all -- --check                PASS
check-docs.py                             PASS
```

Headroom RISC-V: `454192` byte (`kernel_end=0x803911d0`).

## 3. Task 1 — Docs và bounded handoff

`check-docs.py` có luật mới cho văn xuôi **ngoài** marker sinh tự động:
không được chứa `\d+ test`, `\d+/\d+ PASS`, `headroom \d`, hoặc
`\d+/\d+ target PASS`. Khi thêm luật, các chỗ còn số cũ bị phát hiện:
`87 test`, `11 test`, `17 test` trong mục 14/15 và `3/3 target PASS`.
Đã thay bằng tham chiếu vùng `tests`/`status`; thêm các lệnh
`test-harness-spec.py`, `gen-docs.py --check`, `check-docs.py` và
`--feature test-boot-handoff`; bỏ khối exit code viết tay trùng với vùng
`pass`; sửa #40 về remote fence/IPI đã có.

`boot_handoff.S`: sau 1000 lần HSM start hart 0 thất bại, hart 1 **không stop**
mà ghi `BOOT HANDOFF FAIL` qua UART rồi SBI SRST. Chuỗi này nằm trong
`FORBIDDEN` của harness. Chưa có test tiêm lỗi cưỡng bức hết retry; nhánh
được compile trong feature build.

## 4. Task 2 — ACPI MADT x86

Crate mới `vifast-acpi`, `no_std`, `forbid(unsafe_code)`:
- RSDP v2 checksum (20 byte và toàn bảng), XSDT checksum, MADT checksum;
- entry LAPIC type 0, x2APIC type 9, LAPIC address override type 5;
- entry lạ được bỏ qua; length 0/vượt bảng trả lỗi; checksum sai trả lỗi;
- danh sách CPU có `enabled` và `online_capable`, ưu tiên entry type 5.

Host test `vifast-acpi`: **8/8**, gồm bảng đúng, checksum RSDP/MADT sai,
entry length 0, entry vượt bảng, entry lạ, CPU disabled, type 5 override,
XSDT không có MADT.

x86 UEFI đọc `EFI_ACPI_20_TABLE_GUID` từ configuration table trước khi
vào boot runtime; nếu RSDP/bảng nằm trên identity map 1 GiB, in
`ACPI SKIP (above identity map)` và tiếp tục. QEMU:

```text
ACPI MADT OK (cpus=2, bsp=0, lapic=0xfee00000)
```

## 5. Task 3a — LAPIC BSP

- Map LAPIC `0xFEE00000` bằng PDE 2 MiB supervisor-only, PCD/PWT/NX trong
  `PDPT[3]`; copy entry 3 vào mọi VSpace.
- Bật SVR (`0x1FF`), TPR=0; đọc LAPIC ID.
- Giữ PIC/PIT cho timer BSP.

Output x86:

```text
ACPI MADT OK (cpus=2, bsp=0, lapic=0xfee00000)
KERNEL DATA NX OK
LAPIC OK (id=0)
SMP SKIP (x86 AP bootstrap pending)
```

x86 `--repeat 10`: **10/10**.

## 6. Task 3b/3c — chưa làm

Chưa triển khai trampoline 16→32→64-bit, INIT/SIPI, AP entry và maintenance
polling. Vì vậy chưa có `SMP FAIL (ap timeout)`, chưa có register dump AP, và
x86 vẫn `SMP SKIP`, `sched_harts=0b01`.

Lý do dừng: 3b cần thêm allocation trang <1 MiB trước ExitBootServices, blob
trampoline, page table/EFER chuyển tiếp và AP entry; chưa có thời gian kiểm
chứng an toàn trong vòng này. Không giảm `-smp`, không tắt assertion.

## 7. Nợ mới

- 3b/3c x86 AP; `SMP SKIP` vẫn là trạng thái x86.
- `CapDelete` (13) vẫn placeholder ở syscall dù kernel có `delete_cap`;
  user chưa xóa được capability.
- LAPIC chỉ map một cửa sổ 2 MiB cố định 3..4 GiB; chưa hỗ trợ base khác.
- Chưa đo BKL, panic dừng toàn hệ thống, per-VA TLB, FS v2, IOMMU phần cứng,
  net/input, compositor.
