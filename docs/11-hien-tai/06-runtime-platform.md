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
fn stack_a_top() -> usize;
fn stack_b_top() -> usize;
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
fn set_active_root(root_pa: u64);
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
10. `build_task_space` dựng root VSpace từ boot Untyped: code RX, 8 trang stack,
    không data; cài VSpace slot 1 và TCB root.
11. `check_kernel_mappings(root_pa)`; log used/free pages/objects của boot Untyped.
12. `A::set_active_root(root_pa)`, `A::prepare_user()`.
13. Tạo context duy nhất cho root.
14. `A::set_kernel()`.
15. Enqueue root, `pick()`.
16. `A::start_timer()`.
17. `A::enter_user(context root)`.

Root task tự Retype/CapCopy/TcbConfigure/TcbResume server/client/fault thread,
sau đó block bằng `Wait` trên notification riêng. Runtime không còn tham chiếu
"Server"/"Client".

## 4. Syscall dispatch

`runtime::syscall::dispatch::<A>(context, number, args)`:

- lấy `Kernel`;
- lấy current TCB;
- gọi `Kernel::user_syscall`;
- xử lý `UserAction`:
  - `Return`/`Message`: ghi register;
  - `ConfigureTcb`: cài context entry/stack (không còn window validation);
  - `PageTableMap`/`FrameMap`/`FrameUnmap`: kernel walk generic qua
    PageTableOps, read/write PTE, flush, kiểm tra table span;
  - `Open`/`Read`/`Write`: copy user pointer qua platform, gọi ramfs;
  - `Debug`: copy buffer rồi `putc`;
  - `Blocked`: chuyển context;
  - `Exit`: thoát.

Mọi `unsafe` copy user nằm sau `Platform::read_user`/`write_user`.
Runtime assert metadata/data/boot Untyped nằm trong `kernel_accessible_range()`
và gọi `check_kernel_mappings(root_pa)` trên root VSpace mới trước khi set
active root. Check này walk leaf thật và chạy lại trên root mới trước lần
switch đầu tiên.

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
