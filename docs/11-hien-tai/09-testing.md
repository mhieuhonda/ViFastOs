# 11.9. Kiểm thử và tiêu chí PASS

## 1. Build

```bash
./scripts/build.sh
```

Bao gồm:

- host test `vifast-kernel`;
- build RISC-V64 bare metal;
- build AArch64 bare metal;
- build x86_64 UEFI.

## 2. Host test

```bash
cargo test -p vifast-kernel
cargo test -p vifast-runtime
cargo test -p vifast-user
```

Hiện có 12 test trong `vifast-kernel`:

- kiểm tra kích thước `Kernel`/`Object`/`Tcb`;
- self-test đầy đủ;
- capability rights monotonic;
- revoke descendant;
- VSpace W^X + overlap + unmap;
- frame alignment;
- syscall từ chối cap sai type/đã revoke và W^X;
- chuyển cap yêu cầu slot đích rõ ràng, không fallback;
- user pointer validation cho DebugWrite/Open/Read/Write;
- Untyped accounting metadata + OutOfMemory;
- revoke Untyped reset accounting + xoá cap con;
- TcbConfigure/TcbResume.

`vifast-runtime` hiện chưa có unit test riêng; nó được kiểm tra qua QEMU.

## 3. QEMU test

```bash
python3 scripts/test-qemu.py
```

Script chạy:

- RISC-V64: `-machine virt -smp 2`, OpenSBI;
- AArch64: `-machine virt,gic-version=2 -cpu cortex-a57 -smp 2`;
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
