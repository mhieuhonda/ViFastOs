# 11.9. Kiểm thử và tiêu chí PASS

## 1. Build

```bash
./scripts/build.sh
```

Bao gồm:

- host test `vifast-kernel`, `vifast-runtime`, `vifast-user`;
- build RISC-V64 bare metal;
- build AArch64 bare metal;
- build x86_64 UEFI.

## 2. Host test

```bash
cargo test -p vifast-kernel
cargo test -p vifast-pte
cargo test -p vifast-runtime
cargo test -p vifast-user
```

`.cargo/config.toml` đặt `RUST_MIN_STACK=8 MiB` vì `Kernel` debug hiện ~600 KiB.

Hiện có 36 test trong `vifast-kernel` + 9 test trong `vifast-pte`:

- kiểm tra kích thước `Kernel`/`Object`/`Tcb`;
- self-test đầy đủ;
- capability rights monotonic;
- revoke descendant;
- VSpace W^X + overlap + unmap;
- frame alignment;
- syscall từ chối cap sai type/đã revoke và W^X;
- chuyển cap yêu cầu slot đích rõ ràng, không fallback;
- user pointer validation cho DebugWrite/Open/Read/Write;
- Untyped byte accounting + OutOfMemory;
- revoke Untyped destroy Frame + xoá mapping cũ;
- revoke Untyped detach TCB đang BlockedRecv;
- max_objects limit;
- cap copy giữ parent chain;
- TcbConfigure entry/stack validation (sẽ bỏ ở 3b khi code/stack là mapping thường);
- TcbResume từ chối TCB chưa configure;
- Yield thật qua switch_blocked;
- checked_add/overflow + buffer vắt qua hai page khác quyền;
- self-revoke Untyped bị từ chối khi TCB hiện tại là con;
- retype Frame và PageTable đều gọi zero page qua PageTableOps;
- generic page-table walk xoá leaf qua PageTableOps giả lập;
- TCB pool quota đầy trả OutOfMemory;
- walk safety: user range, thiếu bảng, leaf cấp cao, PTE valid, snapshot không đổi;
- PageTableMap gán level 1/2/3, FrameMap qua bảng mới, từ chối bảng vượt range;
- retype VSpace cấp root + aux page, hai VSpace độc lập;
- revoke bản copy Untyped không chạm object của cap gốc;
- root revoke thu hồi cả object của cap con; delete cap bị từ chối khi còn con;
- assert Untyped nằm trong kernel_accessible_range();
- walk leaf thật cho kernel mapping: non-user, X cho trap entry, W cho stack/storage.
- PageTableMap không thay entry 0 dùng chung;
- revoke Untyped unmap Frame trước, rồi PageTable sâu → nông, không còn PTE
  trỏ tới trang đã free;
- page table và VSpace thuộc hai Untyped khác nhau: revoke một bên không để
  reverse map/metadata treo;
- kernel accessible range và boot-time trap-path slot check trên cả 3 arch.

`vifast-pte` có 9 test cho encode/decode, table_span, KERNEL_SHARED_SLOTS, dải user thật của 3
kiến trúc và các biên index.

`vifast-runtime` hiện chưa có unit test riêng; nó được kiểm tra qua QEMU.

## 3. QEMU test

```bash
python3 scripts/test-qemu.py
```

Script chạy:

- RISC-V64: `-machine virt -smp 2`, OpenSBI;
- AArch64: `-machine virt,gic-version=2 -cpu max -smp 2`;
- x86_64: OVMF, `-smp 2`.

Điều kiện PASS:

- `SELFTEST PASS`;
- `booting on`;
- `PREEMPT OK`;
- `Server`;
- `Client`;
- `Mapped`;
- `Cap OK`;
- `ViFastOS ramfs`;
- `ViFastOS write`;
- `Reply OK`;
- `USER FAULT` ít nhất 2 lần;
- `Zero OK`;
- RISC-V/AArch64 phải có `SMP OK`;
- x86_64 chấp nhận `SMP SKIP` ở giai đoạn hiện tại.

## 4. Kết quả hiện tại

```text
riscv64  PASS
aarch64  PASS
x86_64   PASS
```

Đã chạy lặp 3 lần. Exit code QEMU:

- RISC-V: 0;
- AArch64: 0;
- x86_64: 3 do `isa-debug-exit`.

## 5. Debug

- Serial được ghi ra file trong `test-qemu.py`.
- x86 UEFI dùng OVMF; nếu không tìm thấy firmware, test skip/fail.
- Nếu QEMU x86 treo trước banner, kiểm tra OVMF vars và đường dẫn ESP.
- Nếu RISC-V/AArch64 `SMP SKIP`, kiểm tra `-smp 2` và HSM/PSCI.

## 6. Chưa có

- test stress SMP;
- fuzz syscall;
- Kani/Verus;
- test trên phần cứng thật;
- test filesystem persistence;
- test network/driver.
