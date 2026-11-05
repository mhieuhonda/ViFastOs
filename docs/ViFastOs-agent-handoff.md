# ViFastOS — Báo cáo bàn giao đầy đủ cho agent

**Ngày lập:** 2026-11-05  
**Repo:** `/Users/mps/dvchd/ViFastOs`  
**Mốc code hiện tại:** `925f633` (per-hart runqueue, VFS bulk IO, FS hard link)
**Handoff gốc:** `1b3dc12`; **vòng Agent 2 gần nhất:** vòng 19
**Trạng thái:** baseline xanh — 3/3 kiến trúc QEMU PASS, host test PASS, không warning.  
**Mức trưởng thành:** nguyên mẫu microkernel nghiên cứu/giáo dục, **không phải** hệ điều hành production.

> Tài liệu này viết để:
> 1. Người dùng tải xuống và đưa cho một agent khác.
> 2. Agent mới hiểu ngay ViFastOs đang ở đâu, cái gì đã chạy, cái gì còn thiếu.
> 3. Agent có danh sách việc ưu tiên, tiêu chí hoàn thành và các "điều cấm" để không làm hỏng baseline.
>
> Code là nguồn sự thật cuối cùng. Nếu báo cáo và code khác nhau, hãy đọc code và cập nhật lại báo cáo.

---

## 0. Tóm tắt cực ngắn cho agent

### Đã có

- Microkernel Rust, `no_std`, kernel/runtime `forbid(unsafe_code)`.
- Boot QEMU trên **RISC-V 64**, **AArch64**, **x86_64 UEFI**.
- User mode, nạp Rust ELF, VSpace riêng, capability, IPC, notification.
- Scheduler cơ bản, timer preemption, 4 fault demo.
- VFS server chạy user-space qua IPC: FS v3 journal, permission, file 69 KiB, 7 phase persistence.
- Block device:
  - AArch64: VirtIO-MMIO, polling.
  - x86_64: ATA PIO, polling.
  - RISC-V: VirtIO-MMIO, polling; slot `0x10008000`, version 2.
- FS v3 bền vững trên cả 3 kiến trúc; legacy VF01 migrate, VF02 mount-compatible.
- Software IOMMU: domain, reverse map, `IoMap`/`IoUnmap`, 32 mapping/domain.
- SMP: cả 3 kiến trúc có 2 hart, scheduler + worker hart phụ; RISC/x86 targeted per-VA TLB IPI, AArch64 broadcast; x86 AP đã bật.
- BKL ticket FIFO + RAII, mailbox maintenance đối xứng, per-CPU state dùng sscratch/swapgs/TPIDR_EL1; test `PERCPU CLOBBER OK`.
- ASID/PCID tagging; x86 PCID chỉ bật khi CPUID hỗ trợ, QEMU TCG thường fallback CR3.

### Chưa có / còn yếu

- Khóa mịn/x2APIC, hardware IOMMU.
- Hardware IOMMU/VT-d/SMMU; hiện chỉ là mô hình phần mềm.
- Block cache/double indirect/hard link/symlink; flush barrier cho đĩa thật.
- VirtIO network/input.
- Compositor/ViFastKey/GUI.
- ABI placeholder CNode/SchedContext/Irq.
- RISC-V panic QEMU rc=1 đã đạt; giữ test 5/5.
- FS v3 single-indirect + hard link, persistence 7 phase, per-hart ready queue, badge D6, tick lock-free, CapDelete/suspend/priority đã xong.

### Việc nên làm ngay, theo thứ tự

1. **Hardware IOMMU** (VT-d/SMMU) sau mô hình phần mềm.
2. **Khóa mịn/x2APIC** (per-hart runqueue đã xong).
3. **Block cache/double indirect/hard link/symlink + barrier**.
4. **VirtIO net/input**.
5. **Compositor/ViFastKey**.

---

## 1. Thông tin baseline

| Mục | Giá trị |
|---|---|
| Repo | `/Users/mps/dvchd/ViFastOs` |
| Git baseline | `c37982e` |
| Feature commit gần nhất | `925f633` — per-hart scheduler, VFS bulk IO, hard link |
| Working tree lúc lập báo cáo | sạch |
| Ngôn ngữ | Rust |
| Kernel/runtime | `no_std`, `forbid(unsafe_code)` |
| Platform/arch | có `unsafe` để chạm thanh ghi/cổng I/O |
| QEMU | 3 kiến trúc |
| Host test | `vifast-kernel` 83/83, `vifast-pte` 11/11, `vifast-elf` 17/17 |
| QEMU test | 10/10 cả 3 kiến trúc; RISC-V 100/100, AArch64 50/50 |
| Warning | không |
| Tài liệu kiến trúc | `ViFastOs-architecture-summary.txt` |
| SHA-256 summary | `dcc0eb6bd7d929711d137e3c9977683f3dd9b30c64fdbba048d3dcde9d9276be` |

### Lệnh kiểm tra baseline

```bash
cd /Users/mps/dvchd/ViFastOs

# Build + host test + toàn bộ kernel/module
./scripts/build.sh

# Chạy QEMU cả 3 kiến trúc
python3 scripts/test-qemu.py

# Chạy lặp để bắt lỗi race/flaky
for i in 1 2 3; do
  python3 scripts/test-qemu.py || exit 1
done

# Format
cargo fmt --all -- --check
```

### Output QEMU kỳ vọng

```text
RISC-V : SELFTEST PASS
         SMP OK (harts=2, ipi=1)
         BLOCK OK
         FS DISK OK

AArch64: SELFTEST PASS
         SMP OK (harts=2, psci=0, ipi=1)
         BLOCK OK
         FS DISK OK

x86_64 : SELFTEST PASS
         SMP OK (harts=2, ipi=1)
         BLOCK OK
         FS DISK OK
```

`test-qemu.py` dùng disk mới nên không yêu cầu `FS PERSIST OK`.
`scripts/test-persist.py` kiểm tra 2 lần boot + nonce + negative cho cả 3 kiến trúc.

---

## 2. ViFastOS là gì?

ViFastOS là một **microkernel** viết bằng Rust, hướng tới nhiều kiến trúc:

- RISC-V 64
- AArch64
- x86_64

Mục tiêu dài hạn: một hệ điều hành kiểu capability-based, có driver user-space, filesystem, network, compositor, ViFastKey và về lâu dài có thể formal verification.

Hiện tại nó giống một **phòng thí nghiệm OS** hơn là sản phẩm:

- Chứng minh boot đa kiến trúc.
- Chứng minh kernel portable không phụ thuộc phần cứng.
- Chứng minh cách ly user/kernel, capability, IPC, VSpace.
- Chứng minh một số driver và dịch vụ user-space chạy được.

### Mô hình microkernel

```text
+--------------------------------------------------+
| user mode: ring3 / EL0 / U-mode                   |
| root task | VFS server | client | fault demos     |
+--------------------------------------------------+
| vifast-user: syscall wrappers                     |
+--------------------------------------------------+
| vifast-abi: BootInfo, syscall IDs, VFS protocol   |
+--------------------------------------------------+
| vifast-runtime: boot flow, Platform trait, timer  |
+--------------------------------------------------+
| vifast-kernel: object, capability, IPC, scheduler |
| VSpace, page-table walk, ramfs, IOMMU model       |
+--------------------------------------------------+
| vifast-arch-*: trap/MMU/timer/SMP primitives      |
| vifast-platform-*: UEFI/OpenSBI/PSCI, devices     |
+--------------------------------------------------+
| QEMU                                              |
+--------------------------------------------------+
```

Kernel không biết chi tiết phần cứng. Nó gọi các trait/primitive do platform cung cấp.

---

## 3. Hiện trạng tính năng

| Khu vực | Trạng thái | Bằng chứng / ghi chú |
|---|---|---|
| Boot RISC-V 64 | ✅ | QEMU `virt`, OpenSBI `-bios default`, `-smp 2` |
| Boot AArch64 | ✅ | QEMU `virt,gic-version=2`, kernel ELF trực tiếp |
| Boot x86_64 | ✅ | OVMF/UEFI, q35, file `.efi` trên FAT ESP |
| Kernel `no_std` | ✅ | `vifast-kernel`, `vifast-runtime` |
| Kernel/runtime `forbid(unsafe_code)` | ✅ | platform/arch được phép `unsafe` |
| User mode | ✅ | ring3 / EL0 / U-mode |
| Nạp Rust ELF | ✅ | `vifast-elf`, root/server/client |
| VSpace riêng | ✅ | root, server, client, fault tasks |
| Capability | ✅ | copy, mint, revoke, quyền monotonic |
| IPC | ✅ | Send/Call/Recv/Reply/ReplyRecv |
| Notification | ✅ | signal/wait/poll, chuyển cap notification |
| Fault endpoint | ✅ | 4 fault demo, fault handler tối giản |
| Scheduler | ✅ cơ bản | priority, timer, preemption |
| SMP | ✅ | 3 kiến trúc: 2 hart, worker, IPI, per-VA TLB, preempt 50/50 |
| ASID/PCID | ✅ | ASID theo VSpace; PCID nếu CPUID |
| VFS user-space | ✅ | `vifast-fs` v3, journal, directory, permission, indirect, 7 phase persistence |
| VFS đầy đủ | ⚠️ | chưa block cache/indirect/hard link/symlink |
| Block AArch64 | ✅ | VirtIO-MMIO, polling |
| Block x86_64 | ✅ | ATA PIO, polling |
| Block RISC-V | ✅ | VirtIO-MMIO slot `0x10008000` |
| FS bền vững 3 kiến trúc | ✅ | sector 0, magic `VF01`, test-persist |
| BKL | ✅ ticket | FIFO + maintenance poll; trap/syscall/IRQ + x86 fault |
| Per-CPU | ✅ secure | MAX_HARTS=4; sscratch/swapgs/TPIDR; clobber test |
| SMP idle | ✅ RISC-V | affinity, mailbox, timer/IPI, `SMP IDLE OK` |
| SMP worker | ✅ RISC-V | hai worker hart 1, `SMP WORKERS OK`, `SMP SCHED OK`, WFI idle |
| SMP idle AArch | ✅ | CPU1 GIC banked/timer/WFI, `SMP TIMER OK` |
| IOMMU phần mềm | ✅ | domain, 32 mapping, `IoMap`/`IoUnmap` |
| IOMMU phần cứng | ❌ | chưa VT-d/SMMU |
| VirtIO net | ❌ | chưa có |
| VirtIO input | ❌ | chưa có |
| Compositor/ViFastKey | ❌ | chưa có |
| Formal verification | ❌ | mới có fuzz syscall hạn chế |

---

## 4. Kiến trúc và luồng boot

### 4.1 x86_64 — UEFI

```text
OVMF firmware
    → FAT EFI System Partition
    → EFI/BOOT/BOOTX64.EFI
    → kernel UEFI application
```

- Target: `x86_64-unknown-uefi`.
- Kernel là PE/COFF `.efi`, không phải ELF thông thường.
- QEMU: `-machine q35`, `-cpu max`, OVMF code/vars, FAT ESP.
- Exit test: `isa-debug-exit` ở `0x501`; kernel ghi giá trị 1, QEMU trả exit code 3.
- Ổ đĩa test: `isa-ide` + `ide-hd` ở `bus=ide.6,unit=0`.
- ATA PIO dùng primary `0x1F0/0x3F6`, LBA28, polling.

**Điểm cần nhớ:** driver ATA này là QEMU-specific. Máy thật thường dùng AHCI/NVMe, không phải IDE legacy.

### 4.2 AArch64 — kernel trực tiếp

```text
QEMU -kernel vifast-kernel-aarch64
    → kernel ELF
    → platform AArch64
```

- Không UEFI.
- GICv2.
- VirtIO-MMIO block window `0x0a00_0000..0x0a00_4000`, stride `0x200`.
- Thiết bị tìm thấy trong lần chạy trước: base `0x0a003e00`, magic `0x74726976`, version 2, device 2.
- Block driver polling, một request tại một thời điểm, queue tĩnh.

### 4.3 RISC-V 64 — OpenSBI

```text
QEMU -bios default (OpenSBI)
    → kernel ELF ở S-mode
```

- Không UEFI.
- OpenSBI cung cấp SBI console/timer/IPI/HSM.
- `-smp 2`; hart 1 được start bằng SBI HSM.
- OpenSBI PMP Region07 cho S/U `R,W,X` toàn bộ; **không có PMP chặn**.
- 8 slot VirtIO-MMIO cách nhau `0x1000`; QEMU đặt `virtio-blk-device` ở slot cuối
  `0x10008000`. Driver scan `0x10001000..0x10009000`, yêu cầu version 2/device 2.
- Legacy chỉ log `BLOCK SKIP (virtio legacy)`; không dùng RAM disk.

---

## 5. Các subsystem hiện có

### 5.1 Object, capability

Các loại object trong ABI:

```text
Untyped, Tcb, Endpoint, Notification, CNode,
Frame, VSpace, PageTable, SchedContext, Iommu
```

Thực tế đã dùng/kiểm thử mạnh:

- Untyped
- TCB
- Endpoint
- Notification
- Frame
- VSpace
- PageTable
- Iommu (mô hình phần mềm)

Các phần **chưa hoàn chỉnh hoặc là placeholder**:

- `CNode`: chưa có đường retype/implement đầy đủ.
- `SchedContext`: chưa có retype/implement đầy đủ.
- `IrqGet`/`IrqSetNotification`/`IrqAck`: chưa có đường thực thi hoàn chỉnh.

Đừng giả định các object này đã có semantics đầy đủ chỉ vì enum/ABI có tên.

Đặc điểm:

- Capability có quyền monotonic.
- Cap copy giữ parent chain.
- Revoke Untyped đi qua PageTableOps.
- W^X được ép theo Frame, kể cả alias.
- Object table cố định 128 object; TCB quota 16.
- Chưa có free/destroy object/frame ngoài đường revoke Untyped.

### 5.2 Bộ nhớ, VSpace, ASID/PCID

- Frame allocator từ Untyped.
- Page table 3 kiến trúc.
- `PageTableOps` portable.
- `VSpace` có range VA allocator first-fit, tái sử dụng hole.
- Kiểm tra user pointer qua page table walk.
- Frame map/unmap có kiểm tra quyền và PA.
- ASID 12-bit gán theo VSpace.
- x86 PCID chỉ bật nếu CPUID có; QEMU TCG thường không có nên fallback CR3.
- TLB shootdown:
  - RISC-V/AArch64: full user TLB shootdown qua IPI.
  - Chưa có per-VA shootdown.
  - I-cache mới sync local hart; chưa remote.

Layout user:

```text
CODE_VA              = 0x0000_0010_0000_0000   (64 GiB)
ROOT ELF entry       ≈ 0x0000_0010_0020_0000
STACK_BASE           ≈ 0x0000_0010_0030_5000
USER_VA_START        = CODE_VA
USER_VA_END          = CODE_VA + 0x10_0000_0000  (128 GiB)
```

### 5.3 Scheduler và SMP

Đã có:

- Scheduler priority.
- Timer tick.
- Preemption demo.
- `PREEMPT OK`.
- RISC-V: SBI HSM start hart 1.
- AArch64: PSCI CPU_ON CPU1.
- Secondary hart nhận IPI và ACK.
- Full TLB shootdown user và per-VA targeted shootdown.
- Output `SMP OK (... ipi=1)`.

Chưa có:

- x86 AP bootstrap.
- Per-hart runqueue.
- Hart phụ chạy task người dùng.
- Scheduler hart phụ đã có trên cả 3 kiến trúc.
- Timer per-hart và per-VA TLB shootdown đã có; xem mục 9/13 summary.

**SMP hiện tại chỉ chứng minh IPI/TLB maintenance, không phải SMP scheduler thật.**

### 5.4 IPC và VFS

IPC:

- Send, Call, Recv, Reply, ReplyRecv.
- Notification signal/wait/poll.
- Chuyển capability notification.
- `call_message`/`reply_recv_message` ở user wrapper.
- Fault endpoint tối giản; reply để tiếp tục chưa demo.

VFS server:

- Chạy user-space, không nằm trong kernel.
- Protocol label:
  - `OPEN = 0x2001`
  - `READ = 0x2002`
  - `WRITE = 0x2003`
  - `CLOSE = 0x2004`
  - `STAT = 0x2005`
- Giới hạn:
  - 4 file
  - 64 byte/file
  - tên tối đa 8 byte
  - I/O tối đa 24 byte/lần
- File hiện có: `/motd`, `/scratch`.
- `/scratch` ghi xuống sector 0 với:
  - magic `VF01`
  - độ dài `u32` little-endian
  - dữ liệu tối đa 64 byte
- Cả 3 kiến trúc in `FS DISK OK`; persistence do `test-persist.py` kiểm tra
  (`FS PERSIST OK` và `FS PERSIST DATA <nonce>`).

Đây **chưa phải filesystem thật**:
- Không có superblock nhiều block.
- Không có inode.
- Không có directory.
- Không có permission.
- Không có journal/crash recovery.
- Chưa có fsync semantics đầy đủ.

### 5.5 Block device

Giao diện platform:

```rust
fn init_block();
fn block_read(lba: u64, out: &mut [u8; 512]) -> bool;
fn block_write(lba: u64, input: &[u8; 512]) -> bool;
```

Syscall:

```text
BlockRead  = 43
BlockWrite = 44
```

AArch64:

- `vifast-virtio` driver.
- VirtIO-MMIO legacy/modern version >= 2.
- Split virtqueue 8 descriptor.
- Polling, không IRQ.
- Queue tĩnh, một request/lần.

x86_64:

- ATA PIO.
- Primary channel `0x1F0/0x3F6`.
- LBA28.
- Polling BSY/DRQ.
- Flush cache sau write.
- QEMU bus `ide.6,unit=0`.

RISC-V:

- VirtIO-MMIO block backend dùng chung `vifast-virtio`.
- Scan `0x10001000..0x10009000`, stride `0x1000`; device slot `0x10008000`.
- Polling, queue tĩnh; DMA range kiểm tra nằm trong `kernel_accessible_range()`.
- Legacy version 1 chỉ log `BLOCK SKIP (virtio legacy)`.

### 5.6 IOMMU

Đây là **mô hình phần mềm**, không phải IOMMU phần cứng.

Đã có:

- `IommuDomain`.
- Tối đa 32 DMA mapping/domain.
- IOVA window bắt đầu `0x1_0000_0000`.
- Window 1 GiB (`+0x4000_0000`).
- Mapping page-aligned.
- Chỉ `READ`/`WRITE`; từ chối `EXEC`.
- `IoMap = 27`, `IoUnmap = 28`.
- Destroy Frame sẽ xóa mapping DMA liên quan.
- Demo `IOMMU OK`.

Chưa có:

- VT-d.
- SMMUv3.
- Device assignment thật.
- Invalidation queue.
- Interrupt remapping.
- IOMMU page table phần cứng.

---

## 6. Hạn chế, nợ kỹ thuật và rủi ro

| Mức | Vấn đề | Hệ quả | Ghi chú |
|---|---|---|---|
| — | SMP đa hart đã chạy cả 3 | Scheduler/worker/IPI/preempt đã có | Duy trì 50/50 |
| — | x86 AP boot đã xong | AP scheduler + worker | 50/50 |
| — | FS v3 đã có | Journal, directory, permission, indirect, 7 phase | 69 KiB/64 inode |
| — | Per-VA TLB đã có | Mask harts_loaded + TLB_VA | AArch broadcast |
| — | Per-VA TLB + remote I-cache | Targeted IPI, AArch broadcast | Đã xong |
| P1 | IOMMU chỉ mô phỏng | Không cách ly DMA thật | VT-d/SMMU |
| P1 | `static mut` ở platform/server | SMP sẽ có data race | Cần khóa/per-CPU |
| — | Double-fetch audit | Mọi syscall copy user đúng một lần | Đã audit vòng 19 |
| P1 | Object table cố định | Hết object là `OutOfMemory` | 128 object; TCB 16 |
| P1 | QEMU-specific driver | Không chạy được máy thật | ATA legacy, VirtIO-MMIO |
| P2 | x86 identity map 0..1 GiB | RAM/UEFI vượt 1 GiB có thể hỏng | Cần memory map |
| P2 | RISC kernel accessible 4 MiB | Headroom 392.624 B; guard 192 KiB | Theo dõi kích thước |
| P2 | Tài liệu có chỗ cũ | Agent dễ làm sai | Summary có mục stale |
| P2 | CNode/SchedContext/Irq placeholder | ABI trông đầy đủ nhưng chưa chạy | Implement hoặc đánh dấu rõ |
| P2 | Chưa có CI | Dễ commit đỏ | Thêm CI chạy build + QEMU |
| P2 | Chưa formal verification | Không chứng minh tính đúng | Fuzz hiện mới hạn chế |

### Rủi ro lớn nhất

1. **SMP + `static mut`**: scheduler đa hart đã có BKL nhưng vẫn cần audit data race.
2. **FS v3 chưa có barrier phần cứng**: journal giả định sector ghi nguyên tử; đĩa thật cần flush.
3. **BKL/IPI**: không được để hart phụ lấy BKL khi hart 0 đang chờ TLB ACK.
4. **QEMU-specific**: pass QEMU không đảm bảo pass máy thật.
5. **Harness dễ bị "làm yếu"**: không được sửa test cho dễ pass.

---

## 7. Cần cải thiện — roadmap ưu tiên

### P0 — Parity và độ tin cậy baseline

#### P0.1 RISC-V block + FS bền vững — DONE (`23d8dfb`)

- VirtIO-MMIO scan `0x10001000..0x10009000`, stride `0x1000`.
- QEMU đặt device ở slot `0x10008000`; `force-legacy=false` cho version 2/device 2.
- OpenSBI Region07 cho S/U RWX toàn bộ; PMP không phải nguyên nhân cũ.
- Block/FS/persist PASS trên RISC-V; harness bật `require_block/require_disk`.

#### P0.2 Test persistence hermetic — DONE (`096faf8`)

- `scripts/test-persist.py`: disk trắng, nonce phase 2, negative magic.
- Áp dụng cả RISC-V/AArch64/x86_64 từ `23d8dfb`.
- `test-qemu.py` dùng disk temp mới mỗi lần chạy.

#### P0.3 Full SMP

**x86_64:**

- Parse ACPI RSDP/MADT.
- Lấy LAPIC base.
- INIT-SIPI-SIPI cho AP.
- Trampoline thật dưới 1 MiB hoặc page được map.
- GDT/IDT/stack per-CPU.
- AP vào scheduler.
- Thay `SMP SKIP` bằng `SMP OK (... ipi=1)`.
- Harness x86 bật `require_smp=True`.

**RISC-V/AArch64:**

- Hart phụ vào scheduler, không chỉ maintenance loop.
- Per-hart current TCB.
- Khóa/BKL hoặc thiết kế lock phù hợp.
- Timer cho từng hart.
- IPI cho reschedule và TLB.

**Acceptance:**

- Có bằng chứng task chạy trên >1 hart (log/counter mới).
- Không deadlock, không data race.
- QEMU 3 lần liên tiếp.
- Giữ nguyên `SMP OK` và `ipi=1`.

### P1 — Filesystem, TLB, IOMMU

#### P1.1 FS v3 (đã xong phần lõi)

Đề xuất format tối giản nhưng đúng:

- Superblock ở LBA 0 hoặc 1:
  - magic `VF03` (mount VF02)
  - version
  - block size
  - tổng block
  - inode count
  - free bitmap
  - root inode
  - checksum
- Inode table.
- Directory entry.
- Data block.
- Bitmap cấp phát.
- Journal nhỏ hoặc copy-on-write metadata.
- Cache block trong server.

Protocol mở rộng:

```text
OPEN, READ, WRITE, CLOSE, STAT
MKDIR, READDIR, UNLINK, RENAME, TRUNCATE, FSYNC
```

Giữ `/motd`, `/scratch`, các string harness cũ. Thêm test:
- tạo file
- ghi/đọc
- thư mục
- xóa
- rename
- stat
- reboot recovery
- kill giữa ghi (mô phỏng) không hỏng superblock

#### P1.2 Per-VA TLB shootdown + remote I-cache (đã xong vòng 18)

- IPI payload gồm ASID + VA range + loại invalidation.
- Hart nhận invalidate đúng range.
- ACK có timeout.
- I-cache: RISC `fence.i` remote; AArch `dc cvau` + `ic ivau` + barriers. x86 I/D cache coherent, chỉ cần lệnh serializing trên hart chạy code mới; không dùng `wbinvd`/clflush.
- Test map/unmap từ hart A, chạy code mới trên hart B.

#### P1.3 Hardware IOMMU

- x86: ACPI DMAR, VT-d, domain ID, context entry, IOMMU page table, invalidation.
- AArch64: SMMUv3/IORT nếu QEMU hỗ trợ.
- RISC-V: IOMMU spec nếu target hỗ trợ.
- Giữ API `IoMap`/`IoUnmap`, thêm backend.
- Test: DMA hợp lệ chạy; DMA ngoài mapping bị chặn; Frame destroy thu hồi mapping.

### P2 — User experience

#### P2.1 VirtIO network/input

- Tách transport VirtIO dùng chung (MMIO + PCI).
- Net: RX/TX queue, ARP/IPv4/UDP tối thiểu hoặc dùng `smoltcp`.
- Input: virtio-keyboard/tablet; input server; event queue.
- AArch MMIO trước, x86 PCI sau.

#### P2.2 Compositor/ViFastKey

- Framebuffer:
  - x86: UEFI GOP.
  - AArch/RISC: ramfb/virtio-gpu nếu có.
- Compositor user-space:
  - surface, damage, z-order, input focus.
- ViFastKey:
  - keymap, shortcut, text input.
- MVP: cửa sổ đơn giản + bàn phím + chuột.
- Không đưa logic GUI vào kernel.

### P3 — Hardening/cleanup

- Implement hoặc xóa rõ CNode/SchedContext/Irq.
- Free/destroy object/frame ngoài revoke.
- Tách pool TCB/VSpace nếu cần.
- Giảm `static mut`, thêm lock/per-CPU.
- X86 memory map thay identity 0..1 GiB.
- CI chạy build + QEMU.
- Cập nhật docs cũ, stale comments.
- Cân nhắc fuzz rộng hơn, Miri cho phần host.

---

## 8. Hợp đồng harness — không được phá

`scripts/test-qemu.py` là "hợp đồng" của dự án. **Không được làm yếu để pass.**

### Assertion bắt buộc

- `SELFTEST PASS`
- `booting on`
- `BOOT UNTYPED`
- `PREEMPT OK`
- `ROOT SWITCH n` với `n > 1`
- `PERCPU CLOBBER OK` đúng 1 lần; không có `PERCPU CLOBBER BAD`
- RISC-V/AArch64 `SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)` đúng 1 lần; AArch64 thêm `SMP TIMER OK`
- RISC-V/AArch64 `SMP WORKERS OK (a=50, b=50)` đúng 1 lần
- RISC-V/AArch64 `SMP SCHED OK (hart0=N, hart1=M, resched_sent=S, resched_wake=W, idle_entries=I)` đúng 1 lần, N > 1, M >= 100, W > 0, S <= I
- RISC-V/AArch64 `SMP IDLE OK (hart1_wakeups=K, idle_entries=I, wake_timer=Wt, wake_sgi=Ws)` đúng 1 lần; AArch64 thêm `SMP TIMER OK`
- `Isolation`
- `Server`
- `Client`
- `Mapped`
- `Cap OK`
- `ViFastOS ramfs`
- `ViFastOS write`
- `IPC BUF OK`
- `Reply OK`
- `VFS SERVER OK` đúng 1 lần
- `VFS OK` đúng 1 lần
- `VFS STAT OK` đúng 1 lần
- `IOMMU OK` đúng 1 lần
- không có `VFS BAD`
- đúng **4** dòng `USER FAULT`
- có fault tại:
  - `0x80000000`
  - `0x1000002000`
  - `0x1000301000`
- `ROOT RUST` đúng 1 lần
- `BOOTINFO OK` đúng 1 lần
- `RUST MODULES OK` đúng 1 lần
- `CLIENT VSPACE OK` đúng 1 lần
- `FAULT EP OK` đúng 1 lần
- `ROOT READY` đúng 1 lần
- không có `ROOT PANIC`
- `Zero OK`
- x86: `KERNEL DATA NX OK`
- RISC/AArch: `SMP OK` và `ipi=1`
- RISC/AArch/x86: `BLOCK OK`, `FS DISK OK`

### Exit code

- RISC-V/AArch64: QEMU exit code `0`.
- x86_64: QEMU exit code `3` do `isa-debug-exit` (kernel ghi 1).

### Điều cấm

- Không xóa/giảm fault demo.
- Không đổi địa chỉ fault để dễ pass.
- Không đổi `ROOT SWITCH` thành so sánh `>=1`.
- Không bỏ kiểm tra `SMP OK`/`ipi=1`.
- Không đánh dấu RISC-V có disk bằng cách luôn trả `true` từ RAM.
- Không commit khi QEMU đỏ.
- Không thêm `unsafe` vào `vifast-kernel`/`vifast-runtime`.
- Không làm hỏng soft-float user target.
- Không vượt giới hạn RISC-V 4 MiB.

Nếu cần thêm tính năng, **thêm assertion mới**; không thay assertion cũ bằng điều kiện yếu hơn.

---

## 9. Quy trình làm việc cho agent

### Mỗi vòng

1. `git status` — không ghi đè việc đang làm dở.
2. Đọc báo cáo này + `ViFastOs-architecture-summary.txt` + `docs/overnight-log.md`.
3. Chạy baseline:
   ```bash
   ./scripts/build.sh
   python3 scripts/test-qemu.py
   python3 scripts/test-persist.py
   ```
4. Chọn **một** hạng mục nhỏ.
5. Viết test trước nếu có thể.
6. Sửa code.
7. Chạy host test + QEMU 3 lần.
8. Cập nhật:
   - `docs/overnight-log.md`
   - `ViFastOs-architecture-summary.txt`
   - SHA-256 trong log nếu summary đổi
9. Commit một commit sạch.
10. Báo lại: đã làm gì, test nào chạy, còn kẹt gì.

### Definition of Done

Một thay đổi chỉ được coi là xong khi:

- Code build không warning.
- Host test liên quan PASS.
- QEMU 10/10 cả 3 kiến trúc.
- `python3 scripts/test-persist.py` PASS cả 3 kiến trúc.
- Không làm yếu harness.
- Có docs + log.
- Có commit.
- Nêu rõ giới hạn mới nếu có.

### Quy ước commit

```text
feat: ...
fix: ...
test: ...
docs: ...
refactor: ...
```

---

## 10. Bản đồ repo

| Đường dẫn | Vai trò |
|---|---|
| `crates/vifast-kernel` | Portable kernel: object/cap/IPC/scheduler/VSpace/ramfs/IOMMU model |
| `crates/vifast-runtime` | Boot flow, Platform trait, syscall dispatch, timer, ELF loading |
| `crates/vifast-abi` | BootInfo, syscall IDs, object types, VFS protocol, user layout |
| `crates/vifast-user` | Syscall wrappers cho user programs |
| `crates/vifast-elf` | ELF64 parser |
| `crates/vifast-pte` | PTE formats, generic page-table walk, user VA range |
| `crates/vifast-arch` | Trait/primitive chung + `bkl.rs`, `CpuState` |
| `crates/vifast-arch-riscv64` | RISC-V trap/MMU/timer/SMP |
| `crates/vifast-arch-aarch64` | AArch64 trap/MMU/timer/SMP |
| `crates/vifast-arch-x86_64` | x86_64 trap/MMU/timer/SMP |
| `crates/vifast-platform-riscv64` | OpenSBI, UART, page table, platform |
| `crates/vifast-platform-aarch64` | GICv2, PSCI, VirtIO block |
| `crates/vifast-platform-x86_64` | UEFI, GDT/TSS/IDT, ATA, paging |
| `crates/vifast-virtio` | VirtIO-MMIO block driver AArch64 + RISC-V |
| `crates/vifast-root` | Root task: BootInfo, nạp module, fault handler |
| `crates/vifast-server` | User-space VFS server |
| `crates/vifast-client` | VFS client demo |
| `crates/vifast-kernel-*` | Binary kernel cho từng target |
| `scripts/build.sh` | Build và test toàn bộ |
| `scripts/test-qemu.py` | Harness QEMU 3 kiến trúc |
| `scripts/test-persist.py` | Hermetic persistence 3 kiến trúc |
| `scripts/check-root-elf.sh` | Kiểm tra root ELF/link |
| `docs/overnight-log.md` | Nhật ký thay đổi |
| `ViFastOs-architecture-summary.txt` | Tài liệu kiến trúc chi tiết |

---

## 11. Phụ lục A — Output QEMU tham khảo

### RISC-V

```text
SMP OK (harts=2, ipi=1)
ROOT ELF entry=0x1000200000 segments=2 stack=0x1000305000
DATA UNTYPED pages=7/64 objects=7/48
IOMMU OK
USER FAULT ... addr=0x80000000
USER FAULT ... addr=0x0
USER FAULT ... addr=0x1000002000
USER FAULT ... addr=0x1000301000
BLOCK OK
FS DISK OK
VFS OK
PREEMPT OK (ticks=20, switches=539932)
ROOT SWITCH 35
DATA UNTYPED pages=51/64 objects=48/56
```

### AArch64

```text
SMP OK (harts=2, psci=0, ipi=1)
ROOT ELF entry=0x1000200000 segments=2 stack=0x1000305000
DATA UNTYPED pages=7/64 objects=7/48
IOMMU OK
USER FAULT ... addr=0x80000000
USER FAULT ... addr=0x80000000
USER FAULT ... addr=0x1000002000
USER FAULT ... addr=0x1000301000
BLOCK OK
FS DISK OK
VFS OK
PREEMPT OK (ticks=20, switches=1147894)
ROOT SWITCH 35
DATA UNTYPED pages=52/64 objects=48/56
```

### x86_64

```text
ROOT ELF entry=0x1000200000 segments=3 stack=0x1000305000
DATA UNTYPED pages=9/64 objects=9/48
IOMMU OK
USER FAULT ... addr=0x80000000
USER FAULT ... addr=0x0
USER FAULT ... addr=0x1000002000
USER FAULT ... addr=0x1000301000
BLOCK OK
FS DISK OK
VFS OK
PREEMPT OK (ticks=20, switches=408653)
ROOT SWITCH 35
DATA UNTYPED pages=41/64 objects=25/48
```

---

## 12. Phụ lục B — Hằng số và giới hạn quan trọng

| Hằng số | Giá trị |
|---|---|
| User code base | `0x0000_0010_0000_0000` (64 GiB) |
| User VA end | `0x0000_0020_0000_0000` (128 GiB) |
| Root ELF base | `0x0000_0010_0020_0000` |
| Stack base | `0x0000_0010_0030_5000` |
| Page size | 4096 |
| ASID | 12-bit |
| Max objects | 128 |
| Max TCB | 16 |
| Data Untyped | 64 page / 48 object |
| IOMMU mappings/domain | 32 |
| IOMMU IOVA base | `0x1_0000_0000` |
| IOMMU window | 1 GiB |
| VFS files | 4 |
| VFS file size | 64 byte |
| VFS name | 8 byte |
| VFS I/O | 24 byte |
| Block sector | 512 byte |
| AArch virtio window | `0x0a00_0000..0x0a00_4000`, stride `0x200` |
| RISC virtio device | `0x1000_8000`, version 2/device 2 |
| RISC kernel limit | `0x8040_0000` (4 MiB) |
| RISC headroom | 454.192 byte |
| Root ELF RISC-V | 12.880 byte |
| Root ELF AArch64 | 12.704 byte |
| Root ELF x86_64 | 16.744 byte |
| Test disks | temp dir riêng mỗi lần chạy, 4 MiB |

---

## 13. Phụ lục C — Lịch sử commit gần đây

```text
23d8dfb feat: riscv virtio-mmio block backend
4293406 refactor: add big kernel lock and per-cpu state groundwork
096faf8 test: hermetic two-boot persistence check
1e5d113 docs: sync architecture summary with current code
1b3dc12 docs: record x86 block and persistent fs
fe569c1 feat: add x86 ata block backend for persistent vfs
c1cfe2e docs: record persistent vfs on virtio block
1d4f0ea feat: persist vfs scratch on block storage
f216966 feat: add aarch64 virtio block backend
cf89446 docs: record smp maintenance ipi
ed06f82 feat: add smp maintenance ipi and tlb shootdown
aba4816 docs: sync iommu qemu counters
687962c feat: add software iommu domains and dma maps
85fe2cf feat: add user-space vfs server over ipc
395a589 feat: demo notification capability transfer over call
562727d feat: add first-fit vspace range allocator
6963f58 refactor: supply rust modules through bootinfo
826efed feat: tag vspaces with asid and pcid
fffa7d6 feat: add vfs stat request
```

---

## 14. Phụ lục D — Tài liệu tham khảo

- `ViFastOs-architecture-summary.txt` — tổng hợp kiến trúc chi tiết.
- `docs/overnight-log.md` — nhật ký từng đêm/từng mốc.
- `docs/02-kien-truc.md` — kiến trúc.
- `docs/06-lo-trinh.md` — lộ trình cũ.
- `docs/10-multi-arch.md` — ghi chú đa kiến trúc.
- `README.md` — tổng quan và lệnh.

Khi summary thay đổi, cập nhật SHA-256 trong `docs/overnight-log.md`.

---

## 15. Prompt copy-paste cho agent tiếp nhận

```text
Bạn là kỹ sư tiếp nhận dự án ViFastOS tại /Users/mps/dvchd/ViFastOs.

Hãy đọc trước:
- docs/ViFastOs-agent-handoff.md (báo cáo này)
- ViFastOs-architecture-summary.txt
- docs/overnight-log.md
- scripts/test-qemu.py
- crates/vifast-kernel/src/lib.rs
- crates/vifast-runtime/src/lib.rs

Mục tiêu: tiếp tục hoàn thiện các phần còn lại của ViFastOS, mỗi lần làm xong
một phần thì chạy đầy đủ test và commit.

Ưu tiên:
1. Full SMP: x86 AP, scheduler đa hart, BKL audit.
2. FS v3: indirect, permission, crash-safe — đã xong.
3. Per-VA TLB + remote I-cache — đã xong.
4. Hardware IOMMU.
5. VirtIO network/input.
6. Compositor/ViFastKey.

Ràng buộc bắt buộc:
- Kernel/runtime giữ no_std và forbid(unsafe_code).
- User code giữ soft-float đúng target.
- Không làm yếu scripts/test-qemu.py.
- Giữ đúng 4 USER FAULT, đúng các địa chỉ 0x80000000,
  0x1000002000, 0x1000301000.
- Giữ ROOT SWITCH n > 1.
- Giữ các string/assertion hiện có; thêm assertion mới nếu cần.
- Không commit khi test đỏ.
- Mỗi feature phải có docs + docs/overnight-log.md.
- Chạy QEMU 10/10 cả 3 kiến trúc trước khi commit.

Bắt đầu bằng:
  git status
  ./scripts/build.sh
  python3 scripts/test-qemu.py
  python3 scripts/test-persist.py

Sau đó chọn một hạng mục nhỏ, làm, test, cập nhật docs, commit.
Nếu bị kẹt ở SMP/BKL hoặc persistence, báo rõ đã thử gì và log liên quan.
```

---

## 16. Kết luận

ViFastOS đang ở trạng thái **nguyên mẫu microkernel đa kiến trúc có baseline xanh**, với các cơ chế lõi đã chạy thật trên QEMU:

- Boot 3 kiến trúc.
- User mode + ELF + VSpace.
- Capability + IPC.
- VFS user-space.
- Block device + FS v3 bền vững trên cả 3 kiến trúc.
- SMP maintenance IPI/TLB per-VA + capability delete/suspend + preempt 50/50.
- IOMMU mô hình phần mềm.

Các phần còn thiếu lớn nhất là:

- Hardware IOMMU.
- Per-hart runqueue/khóa mịn/x2APIC.
- Block cache/double indirect + barrier.
- Network/input/GUI.

Đây là nền tảng tốt để tiếp tục, nhưng **chưa phải hệ điều hành hoàn chỉnh** và chưa nên dùng cho mục đích production hoặc bảo mật thật.
