# 11.9. Kiểm thử và tiêu chí PASS

## 1. Build

```bash
./scripts/build.sh
```

Bao gồm:

- host test `vifast-kernel`, `vifast-pte`, `vifast-elf`, `vifast-runtime`,
  `vifast-user`;
- build/validate root ELF cho 3 target
  (`riscv64imac-unknown-none-elf`, `aarch64-unknown-none-softfloat`,
  `x86_64-unknown-none`) bằng `vifast-elf` và kiểm tra FP/SIMD;
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
# build.sh cũng build root ELF cho 3 target và chạy vifast-elf-validate
```

`.cargo/config.toml` đặt `RUST_MIN_STACK=8 MiB` vì `Kernel` debug hiện ~600 KiB.

Hiện có 76 test trong `vifast-kernel` + 11 test trong `vifast-pte` +
17 test trong `vifast-elf`:

- kiểm tra kích thước `Kernel`/`Object`/`Tcb`;
- self-test đầy đủ;
- BootInfo layout cố định 32 byte và offset từng field;
- ReplyRecv: no caller chỉ Recv; reply trước rồi nhận caller kế; stale epoch
  trả NoCaller;
- Fault endpoint: handler waiting nhận message, không waiting thì queue, revoke
  cap endpoint xoá link; QEMU in FAULT EP OK;
- QEMU yêu cầu đúng 4 USER FAULT, gồm 0x80000000, 0x1000002000 và root Rust
  stack 0x1000301000; client dùng VSpace/Untyped con riêng, `Zero OK` vẫn qua;
- capability rights monotonic;
- revoke descendant;
- VSpace W^X + overlap + unmap;
- W^X theo Frame: RW ở một VA rồi RX cùng Frame ở VA/VSpace khác bị từ chối,
  snapshot page table không đổi; unmap hoặc revoke VSpace giải phóng bộ đếm;
- mapping bắt buộc có READ (mask rỗng/W-only/X-only trả `InvalidArg`);
- PA uniqueness: Frame-Frame và PageTable-VSpace trùng dải bị từ chối; bố cục
  Frame/PageTable/VSpace rời nhau được chấp nhận;
- map EXEC gọi `sync_icache`, map RW không gọi;
- `mapped_w/mapped_x` tràn u16 trả `OutOfMemory`, không quay vòng;
- frame alignment;
- syscall từ chối cap sai type/đã revoke và W^X;
- chuyển cap yêu cầu slot đích rõ ràng, không fallback;
- user pointer validation cho DebugWrite/Open/Read/Write;
- Untyped byte accounting + OutOfMemory;
- revoke Untyped destroy Frame + xoá mapping cũ;
- revoke Untyped detach TCB đang BlockedRecv;
- max_objects limit; Untyped con: range/quota, sibling PA uniqueness, frame
  nằm trong dải con, revoke cha/con, nested child, cap không root, reparent TCB;
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
- build_task_space_elf map PT_LOAD R/RX/RW, BSS zero, segment mid-page, stack
  4 trang + guard, mở rộng VSpace có sẵn và rollback khi thiếu trang;
- PteFakeOps đếm leaf chính xác và bắt mapping thừa;
- FrameUnmap đòi Frame cap đúng PA; VSpace cap đơn lẻ hoặc sai frame bị từ chối;
- FrameMap chỉ cho quyền mapping ⊆ quyền Frame cap (READ|EXEC không map RW);
- Frame code dùng chung root/server map RX hai lần, `mapped_x=2`, không WRITE;
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

`vifast-pte` có 11 test cho encode/decode, table_span, KERNEL_SHARED_SLOTS, dải user thật của 3
kiến trúc, biên index và leaf L3 AArch64 RO+X / RW+NX / EL0 / leaf_is_user.

`vifast-elf` không tự định nghĩa dải user; caller truyền machine và user range.
Có 17 host test: ELF hợp lệ, magic/class, phdr table/phoff overflow,
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
- đúng 3 `USER FAULT`, có `addr=0x80000000` và `addr=0x1000002000`;
- `ROOT RUST` và `ROOT READY` mỗi chuỗi đúng 1 lần, trước `Server`/`Client`;
- không có `ROOT PANIC`;
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
