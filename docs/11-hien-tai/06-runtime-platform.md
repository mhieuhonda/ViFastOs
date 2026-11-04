# 11.6. `vifast-runtime` và `Platform` trait

Nguồn: `crates/vifast-runtime/src/lib.rs`.

## 1. Mục tiêu

`vifast-runtime` chứa logic dùng chung cho mọi port:

- boot sequence;
- Console trait (không phụ thuộc vifast-arch);
- tạo duy nhất root TCB và giao VSpace/Untyped;
- Kernel được đặt trong static storage của platform, không nằm trên boot stack
  (hiện khoảng 584 KiB với 128 object/64 cap/32 mapping);
- copy user image;
- syscall action dispatch;
- quản lý context switch khi block;
- timer tick/preemption;
- log chuẩn.

Nó **không** chứa `asm!`, MMIO, trap frame hay page-table format.

## 2. `Platform` trait

Các nhóm phương thức:

### Định danh và user layout

```rust
pub trait Platform: Arch + vifast_kernel::console::Console {
type Context: Copy + 'static;
const USER_CODE_VA: usize;
const USER_DATA_VA: usize;
fn user_code_base() -> usize;
```

User layout lấy từ `vifast-abi::user_layout`; cả 3 port dùng chung code VA
64 GiB. `user_code_base()` trả VA cố định, không còn PA động của UEFI.

### Kernel handoff

```rust
fn init_kernel() -> &'static mut Kernel;
fn set_kernel(kernel: *mut Kernel);
fn kernel() -> &'static mut Kernel;
```

Trap handler lấy kernel qua đây. Việc dereference pointer nằm trong platform,
không nằm trong runtime.

### Bộ nhớ và user

```rust
fn data_page() -> &'static mut [u8];
fn metadata_page() -> &'static mut [u8];
fn boot_page() -> &'static mut [u8];
fn page_table_ops() -> &'static dyn PageTableOps;
fn vspace_root() -> u64;
fn switch_root(root_pa: u64) -> bool;
fn check_kernel_mappings(root_pa: u64) -> bool;
fn read_user(ptr, out) -> KResult<()>;
fn write_user(ptr, input) -> KResult<()>;
```

### Context và nhập user mode

```rust
fn context_for_user(entry, stack) -> Self::Context;
fn contexts() -> &'static mut [Self::Context];
fn enter_user(context: &Self::Context) -> !;
fn write_result(...);
fn write_message(...);
```

### Timer, SMP, chuẩn bị

```rust
fn prepare_user() {}
fn start_timer();
fn reload_timer();
fn ack_timer();
fn start_smp();
```

`prepare_user()` dành cho x86: cài GDT/TSS/IDT **sau** khi không còn gọi UEFI
MP Services, tránh làm hỏng môi trường firmware.

## 3. Boot sequence

`runtime::boot::<A>()`:

1. `A::init()` — console/clock.
2. `A::irq_disable()`.
3. `A::init_platform(info)`.
4. In banner.
5. `Kernel::new()`, chạy self-test.
6. In `SELFTEST PASS`.
7. `A::start_smp()`.
8. Tạo boot Untyped 128 KiB/32 object, metadata Untyped, data Untyped.
9. Cài metadata/data Untyped slot 2/3.
10. `build_task_space` dựng root VSpace: code user.S RX, không stack/data;
    map rời 4 stack page fault/client; cài VSpace slot 1 và TCB root.
11. Parse root ELF trong cửa sổ 1 MiB; `build_task_space_elf(Some(root))`
    thêm root ELF + stack 4 trang + guard.
12. Dựng server VSpace từ boot Untyped, code dùng chung Frame root, stack
    +0x2000; cài server VSpace slot 12.
13. Cài Frame cap code user.S READ|EXEC tại các slot code; retype BootInfo
    frame, ghi BootInfo, map READ-only tại `ROOT_BOOTINFO_VA`.
14. `check_kernel_mappings(root_pa/server_pa)`; log BOOT/DATA Untyped.
15. `A::prepare_user()`.
16. Tạo context root từ entry ELF + `ROOT_ELF_STACK_TOP`; set arg0 =
    `ROOT_BOOTINFO_VA`.
17. `A::set_kernel()`.
17. Enqueue root, `pick()`, `ensure_current_root()` -> `ROOT SWITCH 1`.
18. `A::start_timer()`.
19. `A::enter_user(context root)`.

Root task Rust in `ROOT RUST`, tự Retype/CapCopy/TcbConfigure/TcbResume
server/client/fault thread, in `ROOT READY`, rồi block bằng `Wait` trên
notification riêng. Runtime không còn tham chiếu "Server"/"Client".

## 4. Syscall dispatch

`runtime::syscall::dispatch::<A>(context, number, args)`:

- lấy `Kernel`;
- lấy current TCB;
- gọi `Kernel::user_syscall`;
- xử lý `UserAction`:
  - `Return`/`Message`: ghi register;
  - `ConfigureTcb`: cài context entry/stack (không còn window validation);
  - `PageTableMap`/`FrameMap`/`FrameUnmap`: kernel walk generic qua
    PageTableOps, read/write PTE, flush, kiểm tra table span; mapping EXEC
    gọi `sync_icache` trước khi trả;
  - `Open`/`Read`/`Write`: copy user pointer qua platform, gọi ramfs;
  - `Debug`: copy buffer rồi `putc`;
  - `Blocked`: chuyển context;
  - `Exit`: thoát.

Mọi `unsafe` copy user nằm sau `Platform::read_user`/`write_user`.
Runtime assert metadata/data/boot Untyped nằm trong `kernel_accessible_range()`
và gọi `check_kernel_mappings` trên cả root/server VSpace. Root được đọc từ
`tcb.vspace` mỗi lần restore; `switch_root` ghi CR3/TTBR0/satp + full flush khi
khác. `read_user`/`write_user` assert root đang nạp khớp TCB hiện tại. Counter
`root_switches` in `ROOT SWITCH n` khi test-exit.

## 5. Timer

`runtime::timer::on_timer::<A>`:

- lưu context;
- tăng tick;
- in `PREEMPT OK` và exit **chỉ khi feature `test-exit` được bật** (binary test);
- user fault → `fault::suspend`, TCB `Faulted`, chọn thread khác;
- reload/ack timer;
- yield + pick;
- nạp context + resume message.

## 6. Quy tắc mở rộng

Thêm board mới **không** sửa runtime nếu board cung cấp đủ `Platform`.
Nếu cần một hành vi mới, ưu tiên thêm method có default hoặc trait nhỏ hơn
là thêm `cfg(target_arch)` vào runtime.
