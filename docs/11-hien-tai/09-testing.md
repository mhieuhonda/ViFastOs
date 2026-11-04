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
cargo test -p vifast-elf
cargo test -p vifast-runtime
cargo test -p vifast-user
```

`.cargo/config.toml` đặt `RUST_MIN_STACK=8 MiB` vì `Kernel` debug hiện ~600 KiB.

Hiện có 49 test trong `vifast-kernel` + 10 test trong `vifast-pte` +
14 test trong `vifast-elf`:

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
- TcbConfigure không còn validation entry/stack;
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
- validate_user_range walk leaf user/R/W, cross-page/high-leaf/overflow;
- PteFakeOps tách test RISC-V/x86_64/AArch64, dùng encoder/decoder thật cho
  map/unmap/PageTableMap/validate/build;
- build không data rồi FrameMap tại `DATA_VA` thành công trên cả 3 format;
- build_task_space_shared_code map cùng Frame code root/server;
- FrameUnmap đòi Frame cap đúng PA; VSpace cap đơn lẻ hoặc sai frame bị từ chối;
- boot Untyped 128 KiB/32 object và build_task_space host test.
- PageTableMap không thay entry 0 dùng chung;
- revoke Untyped unmap Frame trước, rồi PageTable sâu → nông, không còn PTE
  trỏ tới trang đã free;
- page table và VSpace thuộc hai Untyped khác nhau: revoke một bên không để
  reverse map/metadata treo;
- kernel accessible range và boot-time trap-path slot check trên cả 3 arch.

Boot assert:
- x86 LoadedImage base/end trong 0..1 GiB;
- RISC-V/AArch64 kernel image end + metadata/data/boot Untyped trong kernel accessible range;
- walk leaf thật cho kernel mapping: non-user, X trap entry, W stack/storage.

`vifast-pte` có 10 test cho encode/decode, table_span, KERNEL_SHARED_SLOTS, dải user thật của 3
kiến trúc, biên index và leaf L3 AArch64 RO+X / RW+NX / EL0 / leaf_is_user.

`vifast-elf` không tự định nghĩa dải user; caller truyền machine và user range.
Có 14 host test: ELF hợp lệ, magic/class, phdr table/phoff overflow,
file range, segment/memory offset overflow, congruence vaddr-offset, ET_DYN,
wrong machine, PT_INTERP/PT_DYNAMIC/PT_TLS, W+X, overlap, entry sai và
không có PT_LOAD.

`vifast-runtime` hiện chưa có unit test riêng; nó được kiểm tra qua QEMU.
QEMU kiểm tra thêm `ROOT SWITCH n`: 4.4 chỉ cần n ≥ 1, 4.5 cần n > 1 do
server/client đổi VSpace khi IPC.

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
- `BOOT UNTYPED` (log used/free pages và objects);
- `PREEMPT OK`;
- `Server`;
- `Client`;
- `Mapped`;
- `Cap OK`;
- `ViFastOS ramfs`;
- `ViFastOS write`;
- `Reply OK`;
- `USER FAULT` ít nhất 3 lần, gồm fault3 ghi stack server;
- `Isolation`;
- `ROOT SWITCH n` với n > 1;
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
