# 11.11. Quyết định thiết kế cho Phase 3b (VSpace riêng)

Tài liệu này chốt hướng trước khi triển khai. 4.3 root đã chạy trên VSpace
retype; 4.5 server có VSpace riêng; 7.5 client có VSpace riêng và Untyped con.

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
- Root VSpace map stack fault1/2/3 + root Rust; stack client/fault4 chỉ nằm
  trong VSpace client, stack server +0x2000 chỉ nằm
  trong VSpace server. Fault3 ghi +0x2000 phải USER FAULT.

## 5. Frame map nhiều VSpace

- Đã chọn reverse map nhiều mapping: mỗi `Mapping` lưu Frame object chính xác;
  không dùng phương án mỗi mapping một Frame cap.
- Frame giữ `mapped_w`/`mapped_x` là số leaf W/X trên toàn bộ các VSpace.
  `FrameMap` từ chối W khi Frame đã có X và từ chối X khi Frame đã có W; như
  vậy hai alias của cùng Frame không thể lách W^X.
- Mapping bắt buộc có READ; mask rights rỗng/W-only/X-only trả `InvalidArg`.
- Revoke phải unmap mọi nơi; đây là điều kiện cho IPC buffer ở 3c.
- Revoke VSpace/PageTable, `FrameUnmap` và destroy Frame đều giảm bộ đếm.
  `check_invariants()` đối chiếu `mapped_w`/`mapped_x` với reverse map.
- Bộ đếm dùng `checked_add`; tràn u16 trả `OutOfMemory`, không quay vòng.
- PA uniqueness là invariant trong `check_invariants()`: Frame, PageTable,
  VSpace root/aux không chồng dải; Untyped chỉ lồng theo owner chain và con
  nằm trọn trong cha. Đây là tiền đề bắt buộc của W^X theo Frame.
- Map RW → unmap → map RX là hành vi cố ý (mô hình seL4): tại mỗi thời điểm
  Frame chỉ có một loại quyền. Nó không phải lỗ hổng.

## 5b. Untyped con

- API syscall: `Retype(a0 parent, a1 Untyped, a2 dest, a3 size bytes,
  a4 child_max_objects)`. `size` phải dương, bội số trang; vùng được căn trang
  và lấy từ `bump` của cha.
- Điều kiện quota của cha:
  `objects + reserved + child_max_objects + 1 <= max_objects`. Con chiếm một
  object của cha; `child_max_objects` được giữ trong `reserved`.
- Con có `base`, `size`, `max_objects`, `bump=0`, `objects=0`, `reserved=0`.
  Không zero vùng của con; Frame/PageTable tự zero khi retype.
- Cap con có `parent = CapRef` của cap cha và luôn `root=false`, kể cả sau
  CapCopy.
- Revoke cha thu hồi toàn bộ object trong cây con theo thứ tự Frame →
  PageTable sâu nhất → VSpace → Untyped con; sau đó `reserved` được tính lại và
  bump cha chỉ reset khi `objects == 0`.
- Revoke cap con chỉ reset nội dung con, không đụng tới cha; object trực tiếp
  của cha vẫn sống.
- Delete cap cha bị từ chối khi còn object/cap con (quy tắc delete hiện có).
- Con lồng con được phép; invariant 39 kiểm tra containment và PA uniqueness
  sau mỗi bước.

## 5c. BootInfo

- `BootInfo` nằm trong `vifast-abi`, layout `#[repr(C)]` 32 byte, magic
  `VFBOOT01`, version/size được kiểm tra.
- Runtime cài root/meta/data/server, code Frame caps READ|EXEC, rồi retype một
  Frame BootInfo và map READ-only tại `ROOT_BOOTINFO_VA` — trên root stack,
  ngoài cửa sổ parse ELF.
- Root nhận địa chỉ BootInfo trong arg0 (`a0`/`x0`/`rdi`) và cấp object mới
  tuần tự từ `free_slot_start`; code caps nằm trong `user_code_first/count`.
- Root in `BOOTINFO OK`; harness yêu cầu đúng một chuỗi.

## 5d. VSpace client

- 7.4 dựng VSpace client từ **data Untyped của root**, không dùng Untyped con.
  VSpace/PageTable/stack là tài nguyên của root; client chưa được chuyển sang.
- Root retype VSpace, lặp `PageTableMap` + `FrameMap` qua helper
  `map_with_tables()` cho tới khi hết `MissingPageTable`. Code lấy từ Frame cap
  READ|EXEC trong BootInfo, stack RW.
- Code, client stack và DATA_VA cùng một vùng 2 MiB; bảng cấp cuối được tạo
  bởi code/stack nên client FrameMap tại +0xa000 không gặp MissingPageTable.
- Bước 7.4 chỉ log `CLIENT VSPACE OK`; TcbConfigure chuyển sang VSpace mới ở
  7.5. Data Untyped được log lúc test-exit để kiểm tra quota.

## 5e. Client tách hẳn (7.5)

- Client nhận VSpace riêng ở slot 2 và Untyped con 64 KiB/8 object ở slot 3;
  không còn cap tới root VSpace hay data Untyped của root.
- Root VSpace chỉ còn stack fault1/2/3; client stack và fault4 stack nằm trong
  VSpace client. TcbConfigure client dùng VSpace client.
- Fault4 là thread user.S thứ năm trong VSpace client, ghi vào
  `ROOT_ELF_STACK_BASE` (không map trong VSpace client) để sinh fault thứ tư.
- `Zero OK` vẫn chạy: client CapRevoke/Untyped con/retype Frame zero trong
  VSpace client.
- Harness yêu cầu đúng 4 USER FAULT, gồm địa chỉ root stack; đây là mốc kết
  thúc 3b.

## 5f. ReplyRecv (8.2)

- Syscall 4: `a0 endpoint cap READ`, `a1 recv slot`, `a2 label`, `a3..a5` data.
- Không có caller: chỉ thực hiện Recv.
- Có caller: Reply caller hiện tại trước, sau đó Recv. Nếu caller stale/invalid
  thì trả NoCaller và không queue receiver.
- Server user.S dùng vòng ReplyRecv + Signal; initial call không caller nên chỉ
  Recv, các vòng sau reply rồi nhận caller kế.
- Host test: no caller, reply rồi nhận caller kế, stale epoch.

## 5g. Fault endpoint (8.3)

- TCB có `fault_ep: Option<CapRef>`; đặt qua `TcbConfigure` a5 là slot cap
  Endpoint trong cspace của TCB được cấu hình (0 = clear).
- User fault vẫn log `USER FAULT`. Nếu có fault endpoint, kernel gửi message
  label `LABEL_FAULT`, data `[addr, cause, 0, 0]`; fault TCB chuyển
  `BlockedFault`. Nếu handler chưa Recv, message được queue và TCB vẫn block.
- Handler Recv nhận message; Reply để tiếp tục là quyết định của handler.
- Root task được nâng priority trong runtime để nhận fault4 kịp thời; root in
  `FAULT EP OK addr=...`.
- x86 `enter_user` khôi phục đầy đủ register từ TrapFrame trước iretq, cần cho
  việc resume handler sau page fault.
- Fault1/2/3 không cấu hình endpoint nên giữ hành vi suspend cũ.

## 5h. IPC buffer (8.1)

- TCB có `ipc_buffer: Option<{frame, va}>`; `TcbSetIpcBuffer(tcb, frame, va)`
  yêu cầu Frame map RW đúng VA trong VSpace của TCB.
- Khi sender và receiver đều có buffer, kernel copy `IPC_BUFFER_WORDS=32` word
  trực tiếp qua PA (`copy_frame_to_frame`), không qua VA user.
- Unmap/revoke Frame/VSpace xoá liên kết buffer của TCB qua `note_frame_unmapping`.
- Host test: reject chưa map/RO, copy word, unmap xoá link. Demo IPC dài trong
  user.S chưa nối; API đã sẵn cho 3c.

## 6. Page table

- Retype VSpace/PageTable phải zero page (không chỉ Frame).
- `PageTableOps` là đường duy nhất kernel gọi read/write PTE, unmap, zero,
  install_kernel, flush và sync_icache; không dùng `RevokeReport` hay trả danh
  sách VA về runtime.
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
- FrameMap kiểm tra quyền mapping ⊆ quyền Frame cap. Frame cap code user.S là
  READ|EXEC, không WRITE, nên không thể map code dùng chung thành RW.
- 7.1b: W^X tính theo Frame, không theo từng mapping; đây là lớp bảo vệ bắt
  buộc trước khi root nhận Frame code dùng chung ở 7.3/7.4.
- 7.1c: map EXEC gọi `sync_icache(pa, size)`: RISC-V `fence.i`; AArch64
  `dc cvau` + `dsb ish` + `ic ivau` + `dsb ish` + `isb`; x86 no-op. Khi có
  SMP, RISC-V cần remote fence qua SBI/IPI, AArch64 cần shootdown sang PE khác.
- Object có `creator: CapRef { addr, id }` để revoke Untyped copy không phá
  object của cap gốc; `delete_cap` từ chối khi còn object/cap con.
- Kernel mapping check dùng walk leaf thật (non-user/X/W), không chỉ kiểm tra slot.
- Khi TCB bị hủy, object creator/cap con được reparent lên cap cha còn sống;
  delete Untyped cap trả IllegalOperation khi còn object/cap con.
- x86 assert LoadedImage base/end nằm trong identity 0..1 GiB.
- Untyped con đã có từ 7.2; từ 7.5 client nhận Untyped con, root không còn
  CapCopy data Untyped cho client. 4.5 server không retype.
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
   ELF/link 3 target, nối root ELF vào runtime 6.4 (đã xong); server/client/fault
   giữ user.S.
6. 7.1/7.1b/7.1c: FrameMap ⊆ Frame cap, W^X theo Frame + READ, PA uniqueness,
   sync_icache khi map EXEC và counter checked_add (đã xong).
7. Bước 7.2–7.5: Untyped con (7.2), BootInfo (7.3), VSpace client (7.4),
   tách client/fault4 (7.5 — hoàn tất 3b).
8. 8.1 IPC buffer API/core đã xong; 3c còn server/client Rust ELF và demo buffer dài.
9. AArch64 TTBR1 là giai đoạn riêng sau 3b.
