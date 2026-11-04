# 11.11. Quyết định thiết kế cho Phase 3b (VSpace riêng)

Tài liệu này chốt hướng trước khi triển khai. 4.3 root đã chạy trên VSpace
retype; 4.5 server có VSpace riêng, client vẫn dùng root VSpace.

## 1. Kernel mapping

- RISC-V: copy entry gốc của kernel identity map vào root mới; đánh dấu Global.
- x86_64 và AArch64: PML4/L0 và PDPT/L1 là **riêng** mỗi VSpace; entry 0
  của root trỏ bảng private. Chỉ PDPT[0] (x86) / L1[0..=1] (AArch64) trỏ
  PD/L2 kernel dùng chung; entry user 64..127 trỏ bảng riêng của task.
- Retype VSpace trên x86/AArch64 cần thêm page cho bảng gốc/trung gian.
- Không đưa kernel mapping vào reverse map `VSpace.mappings`.
- TTBR1 và high-half kernel là giai đoạn riêng sau 3b.

## 2. Chuyển address space

- `Context` **không** cache root. Runtime đọc root từ `tcb.vspace` mỗi lần restore;
  TcbConfigure đổi VSpace hoặc revoke làm Faulted không để lại cache treo.
- `Kernel::current_root(tcb)` trả `Option<u64>`; None thì không vào user mà
  fault-suspend TCB.
- Runtime ghi `satp` (RISC-V), `TTBR0` (AArch64) hoặc `CR3` (x86) + full flush
  khi root khác. Có counter `root_switches`, log `ROOT SWITCH n` khi test-exit.
- `read_user`/`write_user` assert root đang nạp khớp current TCB.
- Chưa có ASID/PCID; full flush hiện tại là đủ cho một hart.

## 3. Code/stack

- Bỏ code/stack window cố định.
- Code/stack là mapping bình thường trong VSpace.
- `validate_user_range` chỉ dựa trên mapping + quyền.
- `TcbConfigure` không cần kiểm tra entry/stack; entry sai sẽ gây user fault
  và bị suspend (fault endpoint sẽ báo sau).

## 4. Boot modules và ảnh task

- Root task và về sau server/client là ELF boot module page-aligned; dùng
  `vifast-elf` parse PT_LOAD, không dùng ảnh phẳng.
- Mỗi module có Frame cap; BootInfo chứa danh sách module và frame cap read-only.
- Map từng PT_LOAD RX/RW theo quyền; không map W+X; không copy code.
- Root stack 4 trang + một trang guard; server/client/fault tạm giữ user.S ở Bước 6.
- 4.5 dùng `build_task_space_shared_code`: root và server map cùng Frame code.
- Root VSpace map stack root/client/fault1/2/3; stack server +0x2000 chỉ nằm
  trong VSpace server. Fault3 ghi +0x2000 phải USER FAULT.

## 5. Frame map nhiều VSpace

- Frame phải theo dõi **tất cả mapping** (hoặc mỗi mapping là một cap riêng).
- Revoke phải unmap mọi nơi; đây là điều kiện cho IPC buffer ở 3c.

## 6. Page table

- Retype VSpace/PageTable phải zero page (không chỉ Frame).
- `PageTableOps` là đường duy nhất kernel gọi read/write PTE, unmap, zero,
  install_kernel và flush; không dùng `RevokeReport` hay trả danh sách VA về
  runtime.
- Các thao tác ghi PTE là infallible; kiểm tra nằm ở preflight.
- BootInfo nằm trong `vifast-abi`, có `version`/`size` và slot ranges; root
  không hard-code slot 1/2/3.

- `PageTableMap` đã expose: kernel chọn level, kiểm tra toàn bộ table span,
  lưu vị trí gắn và trả `AlreadyMapped` khi gắn lại.
- Retype PageTable và VSpace từ Untyped đã có; 4.3 runtime root chuyển sang
  VSpace mới, 4.5 thêm VSpace server. Cap VSpace server tạm ở root slot 12;
  BootInfo sẽ thay hard-code ở Bước 7.
- FrameUnmap cần VSpace WRITE + Frame cap khớp PA; chỉ VSpace cap không thể
  unmap mapping của frame không sở hữu.
- Object có `creator: CapRef { addr, id }` để revoke Untyped copy không phá
  object của cap gốc; `delete_cap` từ chối khi còn object/cap con.
- Kernel mapping check dùng walk leaf thật (non-user/X/W), không chỉ kiểm tra slot.
- Khi TCB bị hủy, object creator/cap con được reparent lên cap cha còn sống;
  delete Untyped cap trả IllegalOperation khi còn object/cap con.
- x86 assert LoadedImage base/end nằm trong identity 0..1 GiB.
- CapCopy Untyped vẫn chia sẻ vùng nhớ, chưa có Untyped con; Untyped con để Bước 7,
  khi root phát tài nguyên cho task. 4.5 server không retype.
- Platform chỉ encode/decode PTE; walk page table làm generic trong portable
  kernel.

## 7. Revoke VSpace

- Unmap toàn bộ mapping.
- Trả page-table page về Untyped.
- TCB đang dùng VSpace đó chuyển `Faulted` (đã có).

## 8. Root ELF và target

- `ROOT_ELF_BASE = CODE_VA + 0x20_0000` (2 MiB kế tiếp); user.S vẫn ở 64 GiB.
- Runtime parse root ELF với cửa sổ hẹp
  `[ROOT_ELF_BASE, ROOT_ELF_STACK_GUARD)` (1 MiB), không dùng toàn dải
  64..128 GiB, để linker sai không thể đè code user.S/stack.
- Root stack 4 trang tại `ROOT_ELF_STACK_BASE`, guard page ngay dưới không map.
- Vùng mới cần thêm PageTable cấp cuối; vẫn dùng chung PD/L2 với vùng 64 GiB.
- Code user soft-float: RISC-V `riscv64imac-unknown-none-elf`, AArch64
  `aarch64-unknown-none-softfloat`, x86_64 `x86_64-unknown-none` relocation
  static. Kernel chưa lưu FP/SIMD context.
- Linker: `-z max-page-size=4096`, `-z separate-code`, ALIGN(4096) giữa segment.
- Code model: RISC-V medium; x86_64 pic + `-no-pie` để vẫn ET_EXEC; AArch64
  static. `_start` asm căn stack; RISC-V nạp gp; linker định nghĩa
  `__global_pointer$`.
- build.sh disassemble root ELF và từ chối FP/SIMD; kiểm tra e_type ET_EXEC.
- vifast-elf nhận `machine`, user range từ caller và từ chối
  PT_INTERP/PT_DYNAMIC/PT_TLS; bỏ qua PT_NOTE/PT_GNU_*.
- Overlap segment tính theo trang đã round-down/round-up; PT_LOAD phải có R;
  p_offset và p_vaddr cùng dư trang. PT_LOAD chỉ R map R+NX, RX map RX,
  RW map RW+NX.

## 9. Thứ tự triển khai

1. Hoàn tất kiểm tra fault origin/self-revoke/zero frame.
2. VSpace động + code/stack mapping + PageTableMap (đã xong 4.3).
3. Root/server VSpace riêng + stack isolation (đã xong 4.5).
4. vifast-elf parser + host test (đã xong).
5. Bước 6: build_task_space_elf + host test, jump table user.S, vifast-root
   ELF/link 3 target (đã xong); nối root ELF vào runtime 6.4 là việc tiếp theo;
   server/client/fault giữ user.S.
6. Bước 7: BootInfo, Untyped con, client VSpace riêng; bỏ slot hard-code.
7. 3c: server/client Rust ELF, IPC buffer, ReplyRecv, fault endpoint.
8. AArch64 TTBR1 là giai đoạn riêng sau 3b.
