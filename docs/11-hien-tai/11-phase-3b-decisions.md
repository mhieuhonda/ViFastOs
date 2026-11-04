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

- Runtime nhúng ảnh từng task thành boot module page-aligned.
- Mỗi ảnh có Frame cap; danh sách nằm trong BootInfo frame map read-only.
- Root map Frame code RX vào VSpace từng task; không copy code.
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
  BootInfo sẽ thay hard-code ở Bước 6.
- FrameUnmap cần VSpace WRITE + Frame cap khớp PA; chỉ VSpace cap không thể
  unmap mapping của frame không sở hữu.
- Object có `creator: CapRef { addr, id }` để revoke Untyped copy không phá
  object của cap gốc; `delete_cap` từ chối khi còn object/cap con.
- Kernel mapping check dùng walk leaf thật (non-user/X/W), không chỉ kiểm tra slot.
- Khi TCB bị hủy, object creator/cap con được reparent lên cap cha còn sống;
  delete Untyped cap trả IllegalOperation khi còn object/cap con.
- x86 assert LoadedImage base/end nằm trong identity 0..1 GiB.
- CapCopy Untyped vẫn chia sẻ vùng nhớ, chưa có Untyped con; Untyped con để Bước 6,
  khi root phát tài nguyên cho task. 4.5 server không retype.
- Platform chỉ encode/decode PTE; walk page table làm generic trong portable
  kernel.

## 7. Revoke VSpace

- Unmap toàn bộ mapping.
- Trả page-table page về Untyped.
- TCB đang dùng VSpace đó chuyển `Faulted` (đã có).

## 8. Thứ tự triển khai

1. Hoàn tất kiểm tra fault origin/self-revoke/zero frame.
2. Chuyển AArch64 sang TTBR1.
3. VSpace động + code/stack mapping + PageTableMap.
4. Rust root task + BootInfo + từng task một VSpace.
5. Test cách ly stack client/server.
6. ELF loader trong root task; IPC buffer bằng Frame cap.
