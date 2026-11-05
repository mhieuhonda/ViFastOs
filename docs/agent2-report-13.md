# ViFastOs — Báo cáo Agent 2, vòng 13

**Ngày:** 2026-11-05  
**Baseline:** `83875a7` (code), bundle vòng 12

## 1. Bảng bước

| Bước | Trạng thái | Commit |
|---|---|---|
| 1a. Trang trampoline <1 MiB | ✅ | `78af406` |
| 1b. INIT-SIPI-SIPI, AP real mode | ✅ | `dd07e6b` |
| 1c. AP vào long mode | ✅ | `7dc8a94` |
| 1d. AP online, maintenance mailbox | ✅ | `49c4522` |
| 2. PDPT[3] là kernel slot + docs | ✅ | `03e8c1d` + docs |
| 3. Boot-handoff-fail / BKL stats | ❌ chưa làm | — |

## 2. Test cuối

```text
build.sh                          PASS, không warning
vifast-kernel                     88/88
vifast-arch                        5/5
vifast-pte                        11/11
vifast-elf                        17/17
vifast-acpi                        8/8
QEMU --repeat 10                  10/10 cả 3 kiến trúc
x86_64 --repeat 50                50/50
boot-handoff --repeat 10          10/10
test-harness-spec.py              176 case âm PASS
test-persist.py                   3/3 PASS
cargo fmt --check                 PASS
check-docs.py                     PASS
```

Headroom RISC-V: `454192` byte (`kernel_end=0x803911d0`).

## 3. Task 1 — x86 AP

### 1a. Trampoline
`AllocatePages(AllocateMaxAddress, LoaderData, 1, 0x9F000)` trả trang
**0x9E000** (căn 4 KiB, vector SIPI `0x9E` < `0xA0`). Trang nằm trong
identity map 0..1 GiB và vẫn executable; không phải tách PTE bỏ NX.

### 1b. Real mode
Blob 16-bit ghi magic `A55A` vào `page+0x800`, sau đó `hlt`. BSP gửi
INIT/SIPI/SIPI qua ICR; chờ 10 ms/200 µs bằng vòng TSC (`rdtsc`) xấp xỉ.
LAPIC ID: BSP `0`, AP `1`. Log:

```text
AP TRAMPOLINE OK (page=0x9e000)
AP REAL MODE OK (apic=1)
```

### 1c. Long mode
Trampoline có GDT tạm, protected mode, PAE, CR3 = PML4 boot (`<4 GiB`),
EFER.LME|NXE **trước** paging, rồi far jump 64-bit và `ap_entry`.
`ap_entry` chạy trên `TRAP_STACKS[1]`, in:

```text
AP LONG MODE OK (cpu=1)
```

Lỗi gặp và đã sửa: offset far jump 16-bit cần `0x66`; patch offset 4-byte
của `ljmp`; địa chỉ đích phải là physical `page+offset`, không phải offset 0;
ICR destination phải nằm trong low dword.

### 1d. Online + maintenance
AP nạp GDT/IDT hiện có, bật LAPIC, SMEP/SMAP; IF luôn 0 (D2), vòng poll
mailbox `AP_MAINT_SEQ/ACK` + `pause`, không `hlt`. BSP gửi maintenance TLB +
I-cache thử, AP ACK rồi in:

```text
LAPIC OK (id=0)
AP LONG MODE OK (cpu=1)
AP REAL MODE OK (apic=1)
SMP OK (harts=2, ipi=1)
```

`iptables`/vector LAPIC cho trường hợp AP ở ring 3 chưa được cài; AP hiện
không chạy user và maintenance dùng mailbox, nên chưa có đường nhận vector
thật. Đây là nợ được ghi rõ, không phải đã hoàn tất.

AP online dùng TSC polling; chưa đo chính xác thời gian SIPI→online.

## 4. Task 2 — PDPT[3] và docs

- `vifast-pte::x86_64::KERNEL_SHARED_SLOTS` đổi thành `[0, 3]`; host test
  xác nhận slot 0/3 không giao user entry 64.
- `install_kernel` copy toàn bộ slot kernel từ danh sách; `check_kernel_mappings`
  thêm `lapic_mapping_ok`: supervisor-only, PCD/PWT, NX, và user range không
  giao slot 3.
- Docs: mục 13 x86 cập nhật AP online; mục 4 thêm ACPI/LAPIC; mục 19 thêm
  `crates/vifast-acpi`; mục 20 bỏ "x86 AP/ACPI" khỏi khoảng trống; mục 18 có
  D2 và vòng 11/12/13. `check-docs.py` thêm luật mỗi bundle round phải có mục
  tương ứng trong mục 18; lần chạy đầu thiếu vòng 11/12/13 nên fail, sau đó
  đã bổ sung.

## 5. Nợ

- AP chưa chạy scheduler/user; IDT vector IPI thật cho AP ring3 chưa cài.
- Maintenance AP hiện là mailbox polling, chưa phải LAPIC IPI vector thực.
- `CapDelete` vẫn placeholder; per-VA TLB, FS v2, IOMMU phần cứng, net/input,
  compositor chưa làm.
- Optional `test-boot-handoff-fail`, BKL stats, panic toàn hệ thống chưa làm.
