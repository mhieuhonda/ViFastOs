# 11.6. `vifast-runtime` và `Platform` trait

Nguồn: `crates/vifast-runtime/src/lib.rs`.

## 1. Mục tiêu

`vifast-runtime` chứa logic dùng chung cho mọi port:

- boot sequence;
- tạo object/cap cho user demo;
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
type Context: Copy + 'static;
const USER_CODE_VA: usize;
const USER_DATA_VA: usize;
const DIRECT_VSPACE: bool = false;
fn user_code_base() -> usize;
fn stack_a_top() -> usize;
fn stack_b_top() -> usize;
```

`user_code_base()` cần thiết cho x86 UEFI vì code page nằm tại địa chỉ động
do UEFI cấp, không phải một hằng VA cố định.

### Kernel handoff

```rust
fn set_kernel(kernel: *mut Kernel);
fn kernel() -> &'static mut Kernel;
```

Trap handler lấy kernel qua đây. Việc dereference pointer nằm trong platform,
không nằm trong runtime.

### Bộ nhớ và user

```rust
fn code_page() -> &'static mut [u8];
fn data_page() -> &'static mut [u8];
fn build_page_tables();
fn vspace_root() -> u64;
fn map_user_page(va, pa, rights) -> KResult<()>;
fn unmap_user_page(va) -> KResult<()>;
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
8. Tạo 2 TCB, endpoint, notification, VSpace, Untyped.
9. Cài cap:
   - A: endpoint READ;
   - B: endpoint WRITE, VSpace WRITE, Untyped WRITE, notification
     READ|WRITE|GRANT.
10. Copy user image vào `A::code_page()`.
11. `A::build_page_tables()`.
12. `A::prepare_user()`.
13. Tạo context A/B.
14. `A::set_kernel()`.
15. Enqueue A/B, `pick()`.
16. `A::start_timer()`.
17. `A::enter_user(context A)`.

## 4. Syscall dispatch

`runtime::syscall::dispatch::<A>(context, number, args)`:

- lấy `Kernel`;
- lấy current TCB;
- gọi `Kernel::user_syscall`;
- xử lý `UserAction`:
  - `Return`/`Message`: ghi register;
  - `MapFrame`/`UnmapFrame`: gọi platform MMU, rollback shadow nếu lỗi;
  - `Open`/`Read`/`Write`: copy user pointer qua platform, gọi ramfs;
  - `Debug`: copy buffer rồi `putc`;
  - `Blocked`: chuyển context;
  - `Exit`: thoát.

Mọi `unsafe` copy user nằm sau `Platform::read_user`/`write_user`.

## 5. Timer

`runtime::timer::on_timer::<A>`:

- lưu context;
- tăng tick;
- sau 20 tick in `PREEMPT OK` và exit;
- reload/ack timer;
- yield + pick;
- nạp context + resume message.

## 6. Quy tắc mở rộng

Thêm board mới **không** sửa runtime nếu board cung cấp đủ `Platform`.
Nếu cần một hành vi mới, ưu tiên thêm method có default hoặc trait nhỏ hơn
là thêm `cfg(target_arch)` vào runtime.
