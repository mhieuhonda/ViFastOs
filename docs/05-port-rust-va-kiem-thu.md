# 05. Bản port Rust: cấu trúc, build, kiểm thử

## 1. Cấu trúc workspace

```
rust/
├── Cargo.toml                    workspace, Rust 1.85+, edition 2021
├── manifests/                    space.toml, desktop.toml (nhúng vào binary)
└── crates/
    ├── vifast-abi/src/lib.rs     ABI no_std
    ├── vifast-kernel/src/
    │   ├── lib.rs                no_std, forbid(unsafe_code)
    │   ├── platform.rs           trait Platform
    │   ├── object.rs             ObjRef, Cap, Slot, object, ObjectTable
    │   ├── kernel.rs             struct Kernel, trap, dispatch, fault
    │   ├── cspace.rs             phân giải cptr, CDT, revoke, zombie, retype
    │   ├── ipc.rs                endpoint, call/reply, notification
    │   ├── sched.rs              RunQueue, Hart, timer, MCS
    │   ├── vm.rs                 Sv39, map/unmap, shootdown, IOMMU
    │   ├── irq.rs                ngắt
    │   └── tcb.rs                vòng đời thread
    └── vifast-sim/
        ├── src/
        │   ├── lib.rs, main.rs
        │   ├── hw.rs             PhysMem, Tlb, Hw: Platform, DmaSensor
        │   ├── user.rs           Sys, Instr, Program (async), Images
        │   ├── machine.rs        CPU nhiều hart, MMU, vòng tick
        │   ├── boot.rs           manifest TOML
        │   └── images.rs         chương trình mẫu
        └── tests/kernel.rs       27 test port từ Python
```

## 2. Đối chiếu Python và Rust

| Python | Rust | Ghi chú |
|---|---|---|
| `ktypes.py`, `abi.py` | `vifast-abi` | Op dataclass thành `enum Syscall` |
| `objects.py` | `object.rs` | Mixin thành nhiều khối `impl Kernel` |
| `kernel.py` | `kernel.rs` + `vifast-sim/machine.rs` | Vòng lặp CPU tách ra simulator |
| `cspace.py`, `ipc.py`, `vm.py`, `irq.py`, `tcb.py`, `sched.py` | cùng tên `.rs` | Logic 1-1 |
| `hw.py` | `vifast-sim/hw.rs` | |
| `boot.py` | `vifast-sim/boot.rs` | |
| `images.py` | `vifast-sim/images.rs` | Generator thành `async fn` |
| `tests/test_kernel.py` | `vifast-sim/tests/kernel.rs` | Cùng 27 kịch bản |
| `KernelError` | `Err(KError)` | |
| `BLOCKED`, `PREEMPTED` | `Ret::Blocked`, `Ret::Preempted` | |
| `t.program`, `t.retry_op` | `ThreadCtx` trong simulator | Kernel chỉ giữ `context_epoch` và `resume` |
| Load/Store trong kernel | MMU trong `machine.rs` | Đúng với phần cứng hơn: không phải syscall |

Khác biệt nhỏ về chuỗi kết quả trong test: Python in `(42,)`, Rust in `[42]`; Python in `b'\x00...'`, Rust in `[0, 0, ...]`.

## 3. Chương trình user trong simulator

```rust
pub async fn hello(s: Sys) -> Result<(), String> {
    let console = s.env("console")?;              // thiếu thì Err = panic
    let r = s.call(console, Message::new(PRINT, &pack_text("xin chào"))).await;
    if !r.is_ok() { s.print(format!("lỗi: {}", r.err.name())).await; }
    Ok(())
}
images.register("hello", &["console", "ram_base"], |s| program(hello(s)));
```

Cơ chế: mỗi `.await` trên một lệnh đặt lệnh vào kênh và trả `Pending`. Simulator lấy lệnh, thực thi (syscall vào kernel, hoặc Load/Store qua MMU), đặt kết quả, poll tiếp. Trả `Err(text)` tương đương panic: kernel nhận `USER_EXCEPTION`.

Tham số `&["console", "ram_base"]` là tên của a0, a1 khi tiến trình được tạo lúc chạy bằng `TcbConfigure { args: [...] }`.

## 4. Build và chạy

```bash
cd rust
cargo build                        # build cả workspace
cargo test -p vifast-sim           # 27 test
cargo run -p vifast-sim -- space   # demo space, in log
cargo run -p vifast-sim -- desktop
cargo clippy --all-targets
cargo build -p vifast-kernel --target riscv64gc-unknown-none-elf   # kiểm tra no_std thật
```

Lệnh cuối cần `rustup target add riscv64gc-unknown-none-elf`. Nó chứng minh `vifast-abi` và `vifast-kernel` không vô tình phụ thuộc `std`.

## 5. Khi biên dịch lần đầu

Bản Rust chưa được biên dịch (xem `04-diem-chu-y.md` mục 8). Quy trình khuyến nghị:

1. `cargo build -p vifast-abi`: crate nhỏ nhất, sửa trước.
2. `cargo build -p vifast-kernel`: lỗi hay gặp:
   - **E0502/E0499 (mượn)**: lấy giá trị Copy ra biến cục bộ trước, kết thúc khối mượn, rồi mới gọi phương thức `&mut self`. Toàn bộ code đã viết theo mẫu này, nếu sót thì sửa theo đúng mẫu.
   - **Type annotations needed** trong closure: thêm kiểu tham số, ví dụ `|pa: u64|`.
   - **Unused import**: chỉ là cảnh báo, xoá đi.
3. `cargo build -p vifast-sim`, rồi `cargo test -p vifast-sim`.
4. Nếu test lệch: so với Python bằng cách chạy cùng kịch bản trong `python-model` với `verbose=True` và so log từng tick. Hai bản được thiết kế để cho **cùng chuỗi sự kiện**.

Gửi lại output của `cargo build` nếu cần hỗ trợ sửa.

## 6. Thêm một test mới

1. Viết kịch bản trong Python trước (`python-model/tests/test_kernel.py`), chạy cho qua.
2. Port sang `rust/crates/vifast-sim/tests/kernel.rs` với cùng manifest (TOML) và cùng chương trình (async fn).
3. Nếu là bất biến bảo mật, thêm vào bảng S1..S12 trong `04-diem-chu-y.md`.

## 7. Hướng phát triển code

- Thêm `Kernel::check_invariants()` (chỉ build test) kiểm tra: refs khớp số cap, CDT nhất quán, cap `mapped` khớp PTE, mọi thread READY nằm đúng một runqueue. Gọi sau mỗi syscall trong simulator.
- Fuzz: sinh chuỗi syscall ngẫu nhiên (crate `arbitrary` + `cargo fuzz`), chạy với `check_invariants`. Kernel không được panic với bất kỳ đầu vào nào.
- Kiểm chứng: dùng Kani cho các hàm thuần (`bits_at`, `leaf_flags`, `RunQueue`), Verus cho lõi `unsafe` của `vifast-core`.
