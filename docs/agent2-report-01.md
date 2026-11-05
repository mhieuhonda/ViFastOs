# ViFastOs — Báo cáo Agent 2, vòng 01

**Ngày:** 2026-11-05  
**Người thực hiện:** Agent 2  
**Tệp quyết định:** `ViFastOs-decision-agent2-01.md`  
**Baseline handoff:** `1b3dc12` (code), `f4fc060` (handoff doc)  
**HEAD sau vòng này:** `4293406` (trước commit chứa báo cáo này)  
**Trạng thái repo:** sạch sau từng task; không commit đỏ.

---

## 1. Tổng hợp trạng thái

| Task | Nội dung | Trạng thái | Commit |
|---|---|---|---|
| 1 | Đồng bộ architecture summary | ✅ Xong | `1e5d113` |
| 2 | Persistence test hermetic | ✅ Xong | `096faf8` |
| 3 | RISC-V block, thí nghiệm trước | ⏸️ Dừng chờ quyết định | không commit |
| 4 | BKL + static mut groundwork | ✅ Xong | `4293406` |

Không có task nào bị đánh dấu xong bằng cách giả lập hoặc làm yếu test.

---

## 2. Test đã chạy

### Build

```text
./scripts/build.sh
PASS, không warning
```

### Host test

```text
vifast-kernel : 83/83 PASS
vifast-arch   :  2/2 PASS
vifast-pte    : 11/11 PASS
vifast-elf    : 17/17 PASS
vifast-runtime: compile
vifast-user   : compile
```

### QEMU

```text
RISC-V  PASS
AArch64 PASS
x86_64  PASS
```

Chạy `python3 scripts/test-qemu.py` 3 lần liên tiếp: 3/3 PASS cả 3 lần.

### Persistence

```text
python3 scripts/test-persist.py
aarch64: PASS
x86_64 : PASS
riscv64: SKIP (no block backend; see Task 3)
```

AArch64 và x86_64 đều qua:

- Phase 1 disk trắng: `FS DISK OK`, không có `FS PERSIST OK`.
- Phase 2 seed nonce: `FS PERSIST OK` + đọc lại đúng nonce.
- Phase âm magic sai: không có `FS PERSIST OK`.

---

## 3. Task 1 — Đồng bộ tài liệu

Commit:

```text
1e5d113 docs: sync architecture summary with current code
```

Đã sửa trong `ViFastOs-architecture-summary.txt`:

- Mục 9: bỏ câu “ASID/PCID chưa có”; ghi ASID 12-bit và PCID có điều kiện CPUID.
- Mục 10: server VFS đã có persistence trên AArch64/x86_64; RISC-V fallback RAM.
- Mục 11: thêm syscall 27 `IoMap`, 28 `IoUnmap`, 41 `TcbSetIpcBuffer`,
  42 `FrameWrite`, 43 `BlockRead`, 44 `BlockWrite`.
- Mục 13: sửa trạng thái SMP; IPI maintenance + full TLB shootdown đã có,
  nhưng scheduler đa hart/BKL/per-hart runqueue vẫn chưa.
- Mục 14: danh sách PASS khớp `scripts/test-qemu.py`:
  - `SELFTEST PASS`, `booting on`, `BOOT UNTYPED`, `PREEMPT OK`
  - `Isolation`, `Server`, `Client`, `Mapped`, `Cap OK`
  - `ViFastOs ramfs`, `ViFastOs write`, `IPC BUF OK`, `Reply OK`
  - `VFS SERVER OK`/`VFS OK`/`VFS STAT OK`/`IOMMU OK` đúng 1 lần
  - không `VFS BAD`
  - đúng 4 `USER FAULT` với các địa chỉ bắt buộc
  - `ROOT RUST`/`BOOTINFO OK`/`RUST MODULES OK`/`CLIENT VSPACE OK`/
    `FAULT EP OK`/`ROOT READY` đúng 1 lần
  - không `ROOT PANIC`, `Zero OK`, `ROOT SWITCH n > 1`
  - x86 `KERNEL DATA NX OK`
  - RISC-V/AArch64 `SMP OK` + `ipi=1`
  - AArch64/x86_64 `BLOCK OK` + `FS DISK OK`
  - x86 chấp nhận `SMP SKIP`
- Mục 15: cập nhật DATA UNTYPED hiện tại:
  - RISC-V 36/64 trang, AArch64 37/64, x86 41/64
  - object RISC-V/AArch 21/48, x86 25/48

Đã đối chiếu danh sách assertion giữa:

- `scripts/test-qemu.py`
- mục 14 của summary
- mục 8 của handoff

Ba nơi đều có đầy đủ các chuỗi bắt buộc. SHA-256 summary mới được ghi vào
`docs/overnight-log.md`.

---

## 4. Task 2 — Persistence test hermetic

Commit:

```text
096faf8 test: hermetic two-boot persistence check
```

### Thay đổi

- Thêm `scripts/test-persist.py`.
- Disk nằm trong `tempfile.mkdtemp(prefix="vifast-persist-...")`, xóa trong
  `finally`; không dùng `/tmp/vifast-*.disk` cố định.
- `scripts/test-qemu.py` cũng tạo disk mới trong temp dir riêng cho mỗi lần
  chạy; cleanup trong `finally`.
- Server in thêm:

```text
FS PERSIST DATA <bytes>
```

khi đọc được sector 0 có magic `VF01`. Dòng này chỉ để test đọc lại nonce;
không thay đổi protocol VFS.

### Cách test

**Phase 1 — disk trắng**

- Boot.
- Bắt buộc `FS DISK OK`.
- Cấm `FS PERSIST OK`, cấm `FS PERSIST DATA`.
- Kiểm tra sector 0 sau khi boot:
  - magic `VF01`
  - length đúng
  - payload đúng `ViFastOs write\n` do client ghi.

**Phase 2 — cùng disk**

- Harness tự ghi sector 0 với `VF01` + nonce ngẫu nhiên 16 hex ký tự.
- Boot.
- Bắt buộc `FS PERSIST OK`.
- Bắt buộc server in `FS PERSIST DATA <đúng nonce>`.
- Đây là lựa chọn được phép trong decision file khi không muốn truyền nonce
  xuyên kernel.

**Phase âm — magic sai**

- Ghi magic `BAD!`.
- Boot.
- Bắt buộc `FS DISK OK`, cấm `FS PERSIST OK`/`FS PERSIST DATA`.

AArch64 và x86_64 đều dùng VirtIO/ATA thật; không có fallback RAM.
RISC-V skip có lý do, chờ quyết định Task 3.

---

## 5. Task 3 — RISC-V block: dừng chờ quyết định

**Không commit code Task 3.** Chỉ chạy thí nghiệm; diagnostic tạm đã được
revert bằng `git checkout`.

### 5.1 OpenSBI PMP

Log OpenSBI thực tế:

```text
Domain0 Region00 : 0x80040000-0x8005ffff M:(F,R,W) S/U:()
Domain0 Region01 : 0x80000000-0x8003ffff M:(F,R,X) S/U:()
Domain0 Region02 : 0x00100000-0x00100fff M:(I,R,W) S/U:(R,W)
Domain0 Region03 : 0x10000000-0x10000fff M:(I,R,W) S/U:(R,W)
Domain0 Region04 : 0x02000000-0x0200ffff M:(I,R,W) S/U:()
Domain0 Region05 : 0x0c400000-0x0c5fffff M:(I,R,W) S/U:(R,W)
Domain0 Region06 : 0x0c000000-0x0c3fffff M:(I,R,W) S/U:(R,W)
Domain0 Region07 : 0x00000000-0xffffffffffffffff M:() S/U:(R,W,X)
```

**Kết luận PMP:** Region07 cho S/U đọc/ghi/chạy toàn bộ không gian còn lại.
Không có bằng chứng PMP chặn `0x10001000`. Giả thuyết PMP trong handoff là sai.

### 5.2 QEMU `info mtree`

Có 8 slot VirtIO-MMIO:

```text
0x10001000-0x100011ff
0x10002000-0x100021ff
0x10003000-0x100031ff
0x10004000-0x100041ff
0x10005000-0x100051ff
0x10006000-0x100061ff
0x10007000-0x100071ff
0x10008000-0x100081ff
```

### 5.3 Page table RISC-V

`build_kernel_page_tables()`:

```rust
(*root).0[0] = pte(table_pa(device_l1), PTE_V);
(*device_l1).0[128] = pte(0x1000_0000, PTE_V | PTE_R | PTE_W);
```

Entry 128 của bảng L1 là leaf 2 MiB:

```text
VA 0x10000000 .. 0x101fffff
RW, non-user, không execute
```

Vì vậy `0x10001000` **đã được map** (không phải lỗi page table).

### 5.4 Đọc magic từ kernel

Diagnostic tạm đọc trực tiếp:

| Cấu hình QEMU | `0x10001000` | `0x10008000` | version | device |
|---|---:|---:|---:|---:|
| Không có `virtio-blk-device` | `0x74726976` | `0x74726976` | 1 | 0 |
| Có device, legacy mặc định | `0x74726976` | `0x74726976` | 1 | 2 |
| Có device, `force-legacy=false` | `0x74726976` | `0x74726976` | 2 | 2 |

Magic `0x74726976` là `"virt"` little-endian. UART đọc được, mapping đọc được.

### 5.5 Kết luận

Đây là **(c) nguyên nhân khác**, không phải (a) page-table mapping và không
phải (b) PMP:

1. QEMU đặt `virtio-blk-device` ở **slot cuối `0x10008000`**, không phải
   `0x10001000`.
2. QEMU VirtIO-MMIO mặc định là **legacy version 1**.
3. `vifast-virtio` hiện chỉ scan cửa sổ AArch64
   `0x0a000000..0x0a004000` và yêu cầu `version >= 2`.
4. Vì vậy RISC-V không khởi tạo driver, dẫn tới `BLOCK SKIP`; không liên quan
   PMP/OpenSBI.

**Đề xuất kỹ thuật (không tự chọn A/B):**

- Không cần custom OpenSBI.
- Không cần VirtIO-PCI.
- Có thể thêm một hướng C nhỏ:
  - Parameterize `vifast_virtio::init_range(start, end)`.
  - RISC-V scan `0x10001000..0x10009000` hoặc scan ngược từ slot cuối.
  - Thêm `-global virtio-mmio.force-legacy=false` và
    `-device virtio-blk-device` vào harness RISC-V.
  - Gọi `init_block`/`block_read`/`block_write` từ RISC-V platform.
  - Bật `require_block/require_disk` cho RISC-V.
  - Chạy lại `test-persist.py` cho RISC-V.

Theo decision file, Agent 2 **dừng ở đây** và chờ Agent 0 chọn A, B hoặc C.

---

## 6. Task 4 — BigKernelLock groundwork

Commit:

```text
4293406 refactor: add big kernel lock and per-cpu state groundwork
```

### 6.1 BKL

- Thêm `crates/vifast-arch/src/bkl.rs`.
- `BigKernelLock` dùng `AtomicBool::compare_exchange` + RAII guard.
- `try_lock()` trả `None` nếu đang giữ; host test kiểm tra re-entry.
- `lock()` spin; `drop(guard)` release.
- `vifast-arch` đổi thành `#![cfg_attr(not(test), no_std)]` để chạy host test.
- `build.sh` thêm `cargo test -p vifast-arch`.

Bọc BKL:

- RISC-V: `riscv_trap_handler`.
- AArch64: `aarch64_trap_handler`, `aarch64_irq_handler`.
- x86_64: `x86_trap_handler`, `x86_irq_handler`.
- x86 fault/page-fault path **chưa bọc** vì nhánh đó gọi `enter_user` trực tiếp;
  ghi nhận là nợ cho vòng SMP.

### 6.2 Phân loại `static mut`

| Nhóm | Biến | Xử lý |
|---|---|---|
| Page table/boot-only | `ROOT`, `KERNEL_L1`, `DEVICE_L1`, `L0/L1/L2_*`, `PML4/PDPT/PD_IDENTITY`, `SPLIT_TABLES`, `GDT/IDT/TSS` | Boot-only; không ghi sau init hoặc chỉ boot |
| Kernel state | `KERNEL_PTR`, `KERNEL_STORAGE`, `ACTIVE_ROOT`, `ACTIVE_ASID`, `CONTEXTS` | Dưới BKL khi vào trap; tương lai per-CPU |
| Arena | `USER_DATA`, `METADATA`, `BOOT_POOL`, `TIMER_INTERVAL` | Boot/runtime; dưới BKL |
| Per-CPU tương lai | `TRAP_STACK`, `IST_STACK` | Hiện một bản; phải tách per-CPU |
| x86 feature flags | `NX_ENABLED`, `SMAP_ENABLED`, `PCID_ENABLED` | Boot-only |
| User-space server | `FILES`, `FILE_COUNT`, `DISK_OK`, `PERSISTED` | Chưa khóa; cần khóa server khi đa luồng |
| VirtIO driver | `QUEUE`, `BASE`, `READY`, `LAST_*` | Chưa khóa; một request/lần; cần khóa driver khi SMP |
| Runtime | không có `static mut` thực tế; chỉ trả `&'static mut` qua Platform | Vẫn cần kiểm soát khi SMP |

Không thêm `unsafe` vào `vifast-kernel`/`vifast-runtime`.

---

## 7. Hạn chế/nợ mới

- BKL hiện là test-and-set, chưa fair/ticket; chưa dùng cho SMP thật.
- x86 exception/page-fault chưa đi qua BKL.
- `static mut` chưa per-CPU; server/virtio chưa có khóa riêng.
- RISC-V block vẫn chưa có; chờ quyết định A/B/C.
- Task 2 chỉ áp dụng AArch64/x86_64; RISC-V skip.
- `test-qemu.py` cố ý không yêu cầu `FS PERSIST OK` vì disk mới; persistence
  được kiểm tra bởi `test-persist.py`.
- Handoff document gốc vẫn mô tả baseline `1b3dc12`; báo cáo này là nguồn cập
  nhật cho vòng Agent 2.

---

## 8. SHA-256 và trạng thái cuối

```text
ViFastOs-architecture-summary.txt:
6788309537024a3e3cdc64e2fdf13cdf1c252ad7425e9e8eecadac43718de25a
```

Commit cuối trước báo cáo:

```text
4293406 refactor: add big kernel lock and per-cpu state groundwork
```

---

## 9. Cần Agent 0 quyết định

1. **RISC-V block:** chọn A (custom OpenSBI), B (VirtIO-PCI), hoặc C (sửa
   scan/transport VirtIO-MMIO; khuyến nghị vì không cần firmware/PCI).
2. Sau khi có quyết định, Agent 2 có thể:
   - làm RISC-V block + bật `require_block/require_disk` + chạy test-persist;
   - hoặc chuyển sang full SMP/BKL mở rộng.

Không có thay đổi nào bị mất: mọi thay đổi Task 1/2/4 đã commit; diagnostic
Task 3 đã revert.
