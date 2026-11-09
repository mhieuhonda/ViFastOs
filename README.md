<p align="center">
  <img src="resources/brand/vifastos-lockup-cream.png" alt="ViFastOs · microkernel kiểu seL4 viết bằng Rust" width="620">
</p>

# ViFastOs — microkernel viết bằng Rust, chạy 3 loại CPU

> **Đọc nhanh cho người mới (kể cả sinh viên năm nhất):**
> ViFastOs là một “bác bảo vệ + quản lý tòa nhà” cho máy tính.
> Mọi chương trình muốn `xin RAM, đọc file, nói chuyện với nhau` đều phải xin phép bác này.
> Hiện tòa nhà đã **bật điện được trên cả 3 máy ảo QEMU**
> (RISC-V, ARM, x86_64), chương trình đã ở được phòng riêng,
> gọi điện cho nhau được, lưu file được, gõ lệnh được.
> Nhưng đây vẫn là **“nhà mẫu demo”**, chưa phải nhà để ở thật:
> chưa mạng, chưa màn hình đẹp, driver mới chỉ chạy trên QEMU.
>
> Trạng thái trong file này cập nhật tới `HEAD 61f0f2e (r274, 10/11/2026)`.
> README cũ ghi `51 syscall` là đã cũ — nay là **58 syscall (0…57), 58/58 đã làm thật**.

## Dự án này là gì?

* **Microkernel kiểu seL4:** kernel (lõi) làm rất ít việc —
  chỉ giữ `chìa khóa (capability)`, `xếp hàng chạy (scheduler)`,
  `chia phòng (bộ nhớ ảo)`, `chuyển thư (IPC)`.
  Còn lại như `lưu file, driver, shell` cho ra ngoài làm,
  hỏng cái này không sập cả hệ thống.
* **Một lõi, ba nhà:** cùng một `vifast-kernel` portable,
  nhưng mỗi kiến trúc build một binary riêng.
  Không có binary duy nhất chạy cả 3 CPU.
* **Viết bằng Rust, `no_std`:** hầu hết crate kernel không dùng heap,
  bảng object đặt sẵn trong bộ nhớ tĩnh.

## Đã làm được gì? (tính tới r274)

### 1. Bật máy được trên 3 máy ảo

| Kiến trúc | Boot | Console sớm | Vào user mode | Timer + giành CPU |
|---|---|---|---|---|
| RISC-V64 | OpenSBI + QEMU `virt` | 16550 | `sret` → U-mode, `ecall` | SBI timer |
| AArch64 | QEMU `virt` | PL011 | `eret` → EL0, `svc #0` | GICv2 + generic timer |
| x86_64 | UEFI/OVMF | COM1 | `iretq` → ring3, `int 0x80` | PIC/PIT + LAPIC |

Boot xong kernel tự kiểm tra (`SELFTEST PASS`) rồi mới cho chương trình user chạy.

### 2. Chương trình user đã ở phòng riêng, có chìa khóa

* Nạp được chương trình Rust thật từ file ELF:
  `root / server / client / fault / shell / pager / /bin/vf`.
  Mỗi người một `VSpace` riêng, hàng rào `W^X` (vừa ghi vừa chạy là cấm),
  đánh số `ASID/PCID`, cấp phát địa chỉ kiểu first-fit.
* **Capability = chìa khóa:** muốn làm gì phải có chìa đúng quyền.
  Đã có copy chìa nhưng chỉ được giảm quyền, `revoke` thu hồi,
  chuyển chìa qua IPC phải có quyền `GRANT`, badge giữ nguyên không cho đổi danh tính.
* **IPC = gọi điện + bấm chuông:**
  `Send/Recv`, `Call/Reply/ReplyRecv`, `Notification signal/wait/poll`.
* **Chia đất (Untyped/retype):** cho một cục đất sét (RAM thô),
  muốn thành `Frame, PageTable, VSpace, CNode, Endpoint, Notification, Tcb,
  SchedContext, Iommu` thì nặn ra, có sổ sách.
  `ObjectDestroy (50)` phá từng object và trả đất về Untyped
  (watermark + free list, có bộ đếm `leaked` khi list đầy).
* Giới hạn hiện tại (đọc từ code, không phải số cũ):
  `MAX_OBJECTS = 168`, `MAX_TCBS = 24`, `MAX_HARTS = 4`,
  `root 128 cap / task 16 cap`, `CNode phẳng 16 slot`,
  `64 mapping`, `8 hàng đợi`, `8 fd`, `Kernel ≤ 566_272 byte`.

### 3. Đã chia CPU cho 2 nhân thật

* Cả 3 arch đều bật được nhân thứ 2
  (RISC-V HSM, ARM PSCI, x86 ACPI MADT + LAPIC + GDT/TSS theo từng hart).
* Mỗi hart một hàng đợi riêng + đi ăn trộm việc của nhau (work stealing),
  timer 100 Hz giành CPU (preempt), khóa lớn BKL ticket,
  IPI bảo trì, TLB shootdown đúng địa chỉ.
* Có `Sleep` hẹn giờ, `SchedContext` chia ngân sách `budget/period`,
  `TcbRestart`, `TcbBindNotification`, `TcbWait (51)` chờ TCB exit,
  `TcbInfo (52)` điểm danh, `MemInfo (53)`, `TimerInfo (54)`,
  `ModuleRelease (55)`, `TcbSetArgs (56)` truyền `argc/argv`,
  `TcbGrantCap (57)` ủy quyền cap cho tiến trình con.

Tổng: **58 syscall từ `Send = 0` tới `TcbGrantCap = 57`, 58/58 đã có handler thật,
0 placeholder.** Xem bảng thật trong [crates/vifast-abi/src/lib.rs](crates/vifast-abi/src/lib.rs).

### 4. Đã lưu file được thật, không phải RAM tạm

* Có **VFS server ở user-space + FS v3 có journal (nhật ký)**.
  Mất điện giữa chừng vẫn khôi phục được, test persistence 7 phase đều PASS.
* Làm được: hard link, symlink, file 2 lớp gián tiếp,
  `O_APPEND/O_EXCL/O_DIRECTORY`, `STAT_PATH`, `STATFS`,
  block cache write-back cho data, `fsync/BlockFlush` thật.
* Giới hạn demo (đọc từ [crates/vifast-fs/src/lib.rs](crates/vifast-fs/src/lib.rs)):
  ảnh `8192 block × 512 byte = 4 MiB`, `64 inode`,
  tên `28 byte`, đường dẫn `128 byte`,
  file logic tới `8.4 MiB` nhưng bị chặn bởi ảnh 4 MiB.
* Driver disk mới cho QEMU: VirtIO-MMIO (RISC-V/ARM) và ATA PIO (x86), polling.

### 5. Đã có shell gõ được + chương trình con thật

UART chạy bằng ngắt (IRQ + notification), shell chờ ngắt chứ không hỏi liên tục.
Vào shell thấy `SHELL READY` rồi `shell> `.

Lệnh có trạng thái nên để trong shell (builtin):
`help, time, echo, ls, cat, mkdir, rm, mv, ln, stat, sync, truncate,
loop, run, runfs, fsck, cd, pwd, kill, history, !!`.

Lệnh không trạng thái thì tách ra chương trình ngoài
(một ảnh multicall `/bin/vf` + 7 hard link, chọn món theo `argv[0]`):
`uname, uptime, free, ps, jobs, args, df`.
Gõ `df` thì shell tự tìm `/bin/df` và chạy.

Điểm mới quan trọng (r264 + r268):
trước đây con sinh ra tay trắng nên `df` phải nằm trong shell.
Nay root `mint` endpoint VFS với `badge 1` (danh tính shell, không phải root)
vào `slot 1` của con qua `TcbGrantCap`, con gọi `STATFS` và in `blocks`.
Đây là **applet đầu tiên cần capability**, chứng minh đường ủy quyền cap
cho `exec` đã chạy end-to-end.

Thêm nữa: demand paging có pager server (`PAGER OK / PAGER2 OK / PAGER3 OK /
PAGER EVICT OK`), lỗi truy cập sai chỉ suspend thread đó (`USER FAULT`),
không panic cả kernel. Timer IRQ / UART IRQ đã có object `IRQ + notification/ack`.

## Có gì MỚI so với README cũ?

README cũ dừng ở khoảng `51 syscall`, `147 object / 18 TCB`,
`pager 2 client`, `435 case âm`. Từ r249 tới r274 đã thêm:

* **r249:** `ps/jobs` qua `TcbInfo`, `cd/pwd` builtin, `uname/ps` là hard link.
* **r254:** cổng interactive thật — đọc một luồng serial, gõ từng lệnh chờ đúng marker.
  `help / cd+pwd / uname / ps / history / !!` PASS trên RISC-V + ARM.
* **r259:** bất biến scheduler ở ranh giới về user
  (RISC-V ~167k check, x86 ~95k check, 0 vi phạm), gate thêm bước `boundary`.
* **r260:** đóng hết audit còn lại — `43 CLOSED, 3 UNREACHABLE, 0 OPEN`.
* **r261:** x86 `GDT/TSS theo từng hart`, bỏ code gắn cứng chỉ chạy `hart 1`.
* **r262:** `MAX_IO 256 → 128` vì `256` làm x86 hỏng boot `12/49 (~24%)` lỗi `FS V2 BAD`.
  Về `128` thì `32/32` xanh. Đây là thu hẹp cửa sổ race, chưa phải sửa gốc.
* **r263:** báo cáo G1 đầy đủ + khối `SHA-256` tự kiểm.
  Lần đầu `gate full 10/10` xanh và `soak 100 × 3 arch = 298/300 (99.3%)`,
  không còn lần nào hỏng `FS V2 OK`.
* **r264:** thêm `TcbGrantCap = 57` + 6 host test khóa từng ràng buộc.
* **r268:** `df` thành applet, con nhận cap VFS từ root (badge 1).
* **r271 → r273 → r274 (drama T32/T33):**
  r271 cho hủy được `TCB Exited` dù còn là `current` của hart idle.
  Soak 100 treo sau `shell>` (thiếu `PREEMPT OK`) nên r273 hoàn nguyên.
  Soak 40 trên bản đã hoàn nguyên vẫn treo đúng kiểu 1 lần,
  chứng minh treo **có sẵn ~0.3–1% trên x86_64**, không phải do T32.
  Nên r274 **áp lại bản vá** và mở **T33** để bắt bệnh treo `PREEMPT OK`.

Nguồn thật mới nhất không phải file này mà là
[docs/reviews/progress-g1.md](docs/reviews/progress-g1.md) và
[ViFastOs-architecture-summary.txt](ViFastOs-architecture-summary.txt).
[docs/00-hien-trang.md](docs/00-hien-trang.md) mới tới 06/11/2026 nên cũng đã cũ.

## Kiến trúc

```
                    ┌──────────────────────────────────────┐
                    │   user mode: ring3 / EL0 / U-mode      │
                    ├──────────────────────────────────────┤
                    │  vifast-kernel (portable, no_std)    │
                    │  object · capability · IPC · sched  │
                    │  generic walk · FS ABI · checks      │
                    ├──────────────────────────────────────┤
                    │  vifast-runtime (generic boot/syscall)│
                    ├──────────────────────────────────────┤
                    │  vifast-platform-* (BSP integration) │
                    │  trap · MMU · timer · IRQ · SMP      │
                    ├──────────────────────────────────────┤
                    │  vifast-arch-* (hardware primitives) │
                    │  UART · clock · IRQ mask · shutdown  │
                    ├──────────┬──────────┬────────────────┤
                    │ riscv64  │ aarch64  │ x86_64 (UEFI)  │
                    │ OpenSBI  │ QEMU virt│ OVMF           │
                    └──────────┴──────────┴────────────────┘
```

| Crate | Vai trò | `no_std` | `unsafe` |
|---|---|---|---|
| `vifast-abi` | Số syscall, mã lỗi, quyền, message, layout user | có | cấm |
| `vifast-arch` | Trait `Arch`, `BootInfo`, hợp đồng phần cứng | có | cấm |
| `vifast-kernel` | Object/capability/IPC/scheduler/VSpace portable | có | cấm |
| `vifast-runtime` | Boot, syscall dispatch, timer/preemption dùng chung | có | cấm |
| `vifast-pte` | Encode/decode PTE Sv39/x86/AArch64 | có | cấm |
| `vifast-elf` | Parser ELF64, kiểm W^X/range/overlap | có | cấm |
| `vifast-acpi` | Parser ACPI/MADT cho x86 SMP | có | cấm |
| `vifast-fs` | FS v3 có journal (64 inode, ảnh 4 MiB) | có | cấm |
| `vifast-user` | Wrapper syscall cho user 3 arch | có | có (asm syscall) |
| `vifast-arch-riscv64` | UART 16550, `rdtime`, SBI | có | có (MMIO/asm) |
| `vifast-arch-aarch64` | PL011, `cntpct`, PSCI | có | có (MMIO/asm) |
| `vifast-arch-x86_64` | COM1, `rdtsc`, debug-exit | có | có (port I/O) |
| `vifast-platform-riscv64` | Sv39, trap, SBI timer, HSM SMP | có | có |
| `vifast-platform-aarch64` | MMU 4 cấp, GICv2, timer, PSCI SMP | có | có |
| `vifast-platform-x86_64` | GDT/TSS/IDT theo hart, PML4, PIC/PIT, ring3 | có | có |
| `vifast-kernel-riscv64` | Entry + linker + ảnh user | có | có |
| `vifast-kernel-aarch64` | Entry + linker + ảnh user | có | có |
| `vifast-kernel-x86_64-uefi` | Entry UEFI + ảnh user | có | có |
| `vifast-root` | Task gốc ELF, dựng VFS/pager/exec/shell | có | có (start asm) |
| `vifast-server` | VFS server user-space | có | có |
| `vifast-client` | Shell + demo client | có | có |
| `vifast-bin` | Ảnh multicall `/bin/vf` (7 applet) | có | có |
| `vifast-virtio` | VirtIO block/console | có | có |
| `vifast-fsutil` | Tool host tạo ảnh đĩa (`mkfs/put/link`) | không | có (std) |

Nguyên tắc: `vifast-kernel` **không** biết địa chỉ UART, thanh ghi,
page size thật hay IRQ controller. Chi tiết đó nằm ở
`vifast-arch-*` và `vifast-platform-*`.
Binary `vifast-kernel-*` chỉ còn entry, linker và ảnh user.

## Chạy thử

Yêu cầu:

* Rust stable **1.99.1** + target:
  ```bash
  rustup target add x86_64-unknown-none aarch64-unknown-none riscv64gc-unknown-none-elf
  ```
* QEMU: `qemu-system-riscv64`, `qemu-system-aarch64`, `qemu-system-x86_64`.
* x86_64 cần OVMF (`edk2-x86_64-code.fd` + `edk2-i386-vars.fd`).
  Script tự tìm đường phổ biến trên macOS/Linux, hoặc đặt `OVMF_CODE`.

Build + test nhanh:

```bash
./scripts/build.sh
./scripts/test-qemu.sh
```

Cổng đầy đủ (build + host test + QEMU 3 lần/arch + persistence + harness + docs):

```bash
./scripts/gate.sh
./scripts/check-all.sh
```

Chạy riêng từng arch:

```bash
# RISC-V (OpenSBI + QEMU virt)
cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release
qemu-system-riscv64 -machine virt -m 128M -nographic -bios default \
  -kernel target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64

# AArch64 (QEMU virt)
cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release
qemu-system-aarch64 -machine virt,gic-version=2 -cpu max -m 128M -nographic \
  -kernel target/aarch64-unknown-none/release/vifast-kernel-aarch64

# x86_64 (UEFI/OVMF; xem scripts/test-qemu.py để có lệnh đầy đủ)
cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release
```

Kết quả mong đợi (tên marker có thể thêm theo vòng, nhưng lõi vẫn vậy):

```
ViFastOs booting on riscv64
...
SELFTEST PASS
...
VFS OK
FS LARGE OK
PAGER OK / PAGER2 OK / PAGER3 OK
SHELL READY
shell>
DESTROY OK
SLEEP OK
PREEMPT OK
```

Host test:

```bash
cargo test -p vifast-kernel
cargo test -p vifast-pte
cargo test -p vifast-elf
python3 scripts/test-harness-spec.py   # 632 case âm: PASS
```

## Độ ổn định hiện tại (trung thực)

* Gate full tại r263–r274: **10/10 bước PASS**
  (build, kernel-test, fuzz, QEMU, boundary, persist, docs, harness, fmt).
* Soak dài:
  `100 vòng × 3 arch = 298/300 (99.3%)` tại r262,
  `30 × 3 = 90/90` tại r265,
  `30 × 3 = 90/90` tại r269 sau khi `df` thành applet.
  Gộp 3 đợt cuối: **chỉ 2 lỗi trên 480 lần boot**, không còn hỏng `FS V2 OK`.
* Bệnh còn mở **T33**: treo sau `shell>`, thiếu `PREEMPT OK`,
  chỉ gặp trên x86_64 tỉ lệ `~0.3–1%`.
  Đã chứng minh không phải do bản vá T32.
  Hướng nghi: hart phụ ngủ trong `wait_for_work` mà IPI không đánh thức,
  hoặc đường `Sleep` trả về sai.

## Những gì **chưa** có

Để tránh hiểu nhầm “boot được” = “OS hoàn chỉnh”:

* Userland vẫn là demo (root/server/client/shell/`/bin/vf`),
  chưa có libc, shell hệ thống đầy đủ, dynamic linking hay ứng dụng thật.
* CNode mới bảng phẳng `16 slot`, chưa có guard, nhiều cấp.
* Chưa có hardware IOMMU (VT-d/SMMU); `IoMap` mới là mô hình phần mềm.
* Chưa có network, GPU/framebuffer, compositor hay ViFastKey.
  Shell chưa có job control thật (`jobs` mới đếm TCB).
* FS giới hạn ảnh 4 MiB, chưa có mount/pager tổng quát hay barrier phần cứng thật.
  Driver disk mới cho QEMU, chưa kiểm chứng máy thật.
* SMP mới test 2 hart (`MAX_HARTS = 4`); hart phụ vẫn dùng platform idle loop ở vài đường.
  Chưa có khóa mịn/x2APIC.
* Chưa có CI (GitHub không dùng), formal verification/Kani/Verus;
  mới có invariant checker, fuzz và 632 case kiểm âm.
* Tài liệu `01`–`07` và `legacy/`, `python-model/` là tham chiếu lịch sử,
  nhiều chỗ chưa được port sang bản bare-metal này.

Trạng thái chi tiết: [docs/00-hien-trang.md](docs/00-hien-trang.md) (06/11),
tiến độ mới nhất: [docs/reviews/progress-g1.md](docs/reviews/progress-g1.md),
tài liệu thành phần: [docs/11-hien-tai/00-index.md](docs/11-hien-tai/00-index.md).

## Hướng phát triển

1. ~~Trap + user mode tối giản~~ — đã có.
2. ~~Timer interrupt + preemptive scheduler một hart~~ — đã có.
3. ~~MMU cơ bản~~ — đã có VSpace/PageTable/Frame, W^X, flush TLB.
   Còn allocator page-table tổng quát.
4. ~~SMP bootstrap~~ — đã có 2 hart cả 3 arch + per-hart runqueue + TLB shootdown.
   Còn sửa gốc race `MAX_IO`, mở rộng `-smp 4`, khóa mịn.
5. ~~Untyped/retype cơ bản~~ — đã có accounting + `ObjectDestroy` thu hồi.
   Còn allocator vật lý toàn cục.
6. ~~Root task + user ELF~~ — đã có `/bin/vf` + `TcbGrantCap` cho exec.
   Còn chính sách badge đầy đủ và `exec` từ file bất kỳ mượt hơn.
7. **Driver framework** user-space: VirtIO console/block đã có, còn net.
8. **FS + pager + supervisor + compositor** — FS demo đã có, còn mount/pager tổng quát.
9. **Verification**: mở rộng invariant checker, fuzz, Kani/Verus; bắt T33.

## Ghi chú thiết kế

* Một kernel **nguồn** cho nhiều arch, nhưng **mỗi arch một binary**.
* `vifast-kernel` là `#![forbid(unsafe_code)]`; `unsafe` chỉ nằm ở arch/boot/user-asm.
* Bảng object cố định để chứng minh lõi không cần heap;
  đây là reference implementation, chưa phải allocator cuối.
* RISC-V dùng trap stack qua `sscratch`; AArch64 dùng `SP_EL1`,
  x86_64 dùng `TSS.RSP0`. Stack per-thread vẫn là việc tiếp theo.
* x86_64 tự dựng PML4 và page table 4 KiB cho user; kernel/driver vẫn chạy
  identity map 0..1 GiB.
* x86_64 dùng UEFI/OVMF vì QEMU `-kernel` không nhận ELF64 multiboot.
* QEMU x86 dùng `isa-debug-exit` (ghi port `0x501`, thoát mã 3);
  RISC-V dùng SBI SRST; AArch64 dùng PSCI SYSTEM_OFF.

## Trang web (os.vifast.org)

Trang giới thiệu + **đăng ký nhận tin sớm** nằm ngay trong repo này, ở
`deploy/vifastos_web/` (FastAPI + SQLite, không ORM, không analytics).

```bash
cd deploy/vifastos_web
docker compose up -d --build       # đứng sau Traefik của Coolify (network `coolify`)
```

Biến môi trường: `ADMIN_PASSWORD` (bảo vệ `/admin`), `STATS_SALT`, `DATA_DIR`,
`ALLOWED_HOSTS`. Đăng ký lưu ở `data/vifastos.sqlite3`; xem/xuất CSV tại
`/admin` và `/admin/export.csv`.

## Giấy phép

Mã nguồn phát hành theo **giấy phép kép MIT HOẶC Apache-2.0** — người dùng chọn một trong hai:

* [`LICENSE-MIT`](LICENSE-MIT) — bản ngắn gọn, tương thích GPLv2.
* [`LICENSE-APACHE`](LICENSE-APACHE) — có điều khoản cấp bằng sáng chế (§3).

Biểu thức SPDX: `MIT OR Apache-2.0` (khai báo ở `[workspace.package]` trong `Cargo.toml`).

Logo, wordmark và tài nguyên thương hiệu **không** thuộc phạm vi hai giấy phép trên — xem
[`TRADEMARKS.md`](TRADEMARKS.md).
