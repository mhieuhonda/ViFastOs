# 11.4. Bộ nhớ ảo, Frame/VSpace/PageTable và W^X

Nguồn: `crates/vifast-kernel/src/object.rs`,
`crates/vifast-kernel/src/lib.rs`,
`crates/vifast-runtime/src/lib.rs`,
`crates/vifast-platform-*/src/platform.rs`.

## 1. Mô hình chung

Kernel portable đi **generic page-table walk** qua `PageTableOps`. Page table
thật là nguồn sự thật duy nhất; `VSpace.mappings` chỉ còn là reverse map cho
revoke.

Trình tự map:

```text
FrameMap syscall
  → Kernel::map_frame() kiểm tra W^X/range/reverse map
  → Kernel map_page_table() walk generic
  → PageTableOps::write_pte + flush_va
  → Runtime trả VA cho user
```

Thiếu bảng trung gian trả `MissingPageTable`; kernel không tự cấp phát bảng.
Unmap đi cùng walk và dừng ở leaf cấp cao.

`Kernel::build_task_space(untyped, image, layout)` là đường mẫu cho Bước 4/7:
retype VSpace, gắn PageTable còn thiếu, copy ảnh vào Frame, map code RX và
`stack_pages` trang stack RW; `data: Option<TaskData>` chỉ map khi được yêu cầu.
Nếu lỗi thì rollback object và bump/objects của Untyped.

Root dùng builder với `stack_pages=0`, rồi runtime map rời 4 stack page
`+0x5000/+0x6000/+0x7000/+0x8000` cho fault1/2/3 và client. Root Rust ELF dùng
stack 4 trang + guard tại `ROOT_ELF_STACK_BASE`. Trang `+0x2000` dành riêng cho
server và **không được map trong root VSpace**.

`Kernel::build_task_space_shared_code(untyped, code_frame, layout)` map cùng
Frame code root vào VSpace server, chỉ nhận một trang code ở bring-up hiện tại.

`Kernel::build_task_space_elf(untyped, vspace, image, bytes, stack)` map từng
PT_LOAD theo trang: R → R+NX, RX → RX, RW → RW+NX; copy đúng `filesz` tại offset
`vaddr % 4096`, phần BSS còn lại đã zero; stack có 4 trang và guard không map.
Có thể thêm vào VSpace có sẵn. Lỗi giữa chừng rollback object, bump/objects và
các PTE mới.

`PageTableMap(page_table_cap, vspace_cap, va)`:

- kernel walk từ root tới entry invalid đầu tiên trên đường tới `va`;
- tự chọn level của bảng mới, user không truyền level;
- kiểm tra toàn bộ `table_span(level)` nằm trong `user_va_range`, không chỉ `va`;
- ghi đúng một PTE cha, flush, lưu `{vspace, va_base, level}` vào PageTable;
- gắn lần hai trả `AlreadyMapped`.

Revoke Untyped đi theo thứ tự Frame → PageTable sâu nhất → PageTable cha →
VSpace, để Frame còn unmap được leaf trước khi bảng cha bị gỡ.

## 2. VSpace

`VSpace` lưu:

- root handle (root retyped từ boot Untyped; bảng tĩnh chỉ còn kernel slots);
- page size;
- range `[base, end)`;
- `next_va` cho cấp phát tuần tự;
- `aux_pa` cho PDPT/L1 riêng của VSpace retype trên x86/AArch64;
- tối đa 32 mapping (reverse map).

Root VSpace 6.4 dùng chung cho root Rust/client/fault thread; server có VSpace riêng.
`validate_user_range()` walk page table thật cho từng trang trước
`DebugWrite`, `Open`, `Read`, `Write`: leaf phải non-kernel, có R, và W nếu
cần ghi. Code/stack window fields đã bị xoá; reverse map chỉ còn cho revoke.

`Kernel::map_frame` kiểm tra:

- rights chỉ gồm `READ|WRITE|EXEC`;
- không đồng thời `WRITE` và `EXEC` trên một leaf, và W^X được kiểm tra theo
  **Frame**: Frame đang có leaf W thì không nhận leaf X ở bất kỳ VA/VSpace nào,
  ngược lại cũng vậy;
- PA căn page;
- Frame đủ một page;
- VA căn page;
- VA nằm trong range;
- không overlap mapping cũ.

## 3. RISC-V64

File: `crates/vifast-platform-riscv64/src/platform.rs`.

- Sv39; `ROOT` tĩnh chỉ còn kernel slots làm nguồn cho `install_kernel`.
- Kernel identity map `0x8000_0000..0x803f_ffff`.
- UART `0x1000_0000` map 2 MiB.
- Kernel accessible chỉ khoảng 4 MiB (`0x8000_0000..0x8040_0000`); kernel image
  và metadata/data Untyped phải nằm trọn trong dải này.
- Root VSpace: code user.S +0 RX, stack rời +0x5000/+0x6000/+0x7000/+0x8000
  cho fault/client; root ELF và stack Rust tại ROOT_ELF_BASE. Data +0xa000
  để trống cho client FrameMap.
- Server VSpace: code dùng chung Frame root, stack +0x2000 RW; root VSpace
  không map trang này.
- Code: `R|X|U` (không W).
- Stack: `R|W|U` (không X).
- Data mapping qua `FrameMap`: `R|W|U` hoặc `R|X|U` theo quyền.
- `sfence.vma` sau map/unmap.

Trap entry dùng kernel trap stack qua `sscratch`; `sscratch=0` khi ở kernel
để phát hiện trap từ S-mode. `SUM=0`; copy user bật `SUM` tạm thời.

## 4. AArch64

File: `crates/vifast-platform-aarch64/src/platform.rs`.

- 4-level bảng kernel-only tĩnh; root VSpace/PageTable user retyped từ boot Untyped.
- GICv2 `0x0800_0000`, PL011 `0x0900_0000` map trong L1[0]; RAM kernel
  `0x4000_0000..` map trong L1[1]. `KERNEL_SHARED_SLOTS = [0, 1]`.
- Root VSpace: code user.S +0 EL0 RO+X, stack fault/client
  +0x5000..+0x8000 RW+NX; root ELF + stack Rust ở ROOT_ELF_BASE; data +0xa000
  để trống cho client FrameMap.
- Server VSpace: code dùng chung Frame root, stack +0x2000 RW+NX.
- Code: EL0 read-only + executable (`AP_USER_RO`, UXN=0).
- Stack: EL0 read-write + NX (`AP_USER_RW`, UXN=1).
- Data: EL0 read-write + NX.
- Map/unmap dùng `tlbi vae1is`, `dsb ish`, `isb`; `switch_root` dùng
  `tlbi vmalle1is`, `dsb ish`, `isb` (full flush).
- PAN nếu CPU hỗ trợ; read/write user tạm clear PAN.

Lưu ý encoding `AP[2:1]`:
- `AP_USER_RW = 0b01` (bit 6);
- `AP_USER_RO = 0b11` (bit 6 + bit 7) để EL0 đọc được.

## 5. x86_64 UEFI

File: `crates/vifast-platform-x86_64/src/platform.rs`.

- Tự dựng PML4/PDPT/PD identity 4 KiB kernel-only, không sửa page table UEFI.
- Identity map 0..1 GiB cho kernel/UEFI image, stack, GDT/TSS/IDT; PDPT[0]
  là shared slot, PML4[0]/PDPT là private mỗi VSpace.
- Root VSpace: code user.S +0 P|US RX, stack fault/client +0x5000..+0x8000
  RW|NX; root ELF + stack Rust ở ROOT_ELF_BASE; data +0xa000 để trống cho
  client FrameMap.
- Server VSpace: code dùng chung Frame root, stack +0x2000 RW|NX|US.
- Code: P|US, read+execute, NX clear.
- Stack/data: P|W|US|NX.
- `FrameMap` map đúng VA được yêu cầu; `FrameUnmap` clear PTE + `invlpg` và
  đòi Frame cap có PA khớp mapping, không chỉ VSpace cap.
- `VSpace.direct` đã bị xoá; x86/AArch64 dùng chung root entry 0 nhưng tách
  bảng user; PageTableMap không thay entry 0.
- SMEP/SMAP nếu CPUID hỗ trợ; user copy dùng STAC/CLAC.
- CR3 được chuyển sang PML4 riêng ngay trước `iretq`.
- Hạn chế còn lại: identity map 0..1 GiB; chưa có AP; data kernel chưa NX.
## 6. TLB

Hiện tại:

- RISC-V: `sfence.vma` một hart.
- AArch64: map/unmap `tlbi vae1is`; đổi root `tlbi vmalle1is`; cả hai kèm
  `dsb ish`/`isb`.
- x86: `invlpg`.

Chưa có IPI shootdown đa hart. Khi thêm SMP scheduler, cần:

1. Theo dõi hart nào đang dùng VSpace.
2. Gửi IPI tới các hart đó.
3. Chờ acknowledge trước khi trả frame/page table.

## 7. Page table allocator

`PageTable` object đã có metadata `{root, mapped: {vspace, va_base, level}}` và PageTableMap.
Retype VSpace cấp root + aux page, zero và gọi `install_kernel`. 4.3 dựng root
VSpace, 4.5 dựng server VSpace từ boot Untyped. Runtime đọc root từ TCB mỗi lần
restore, switch CR3/TTBR0/satp và full flush nếu khác. `kernel_accessible_range()`
assert metadata/data nằm trong identity map. Boot check walk leaf thật qua
`read_pte`/`decode_at` cho toàn dải kernel accessible và các symbol trap/kernel/MMIO:
leaf phải non-user, trap entry có X, stack/storage có W. Chưa có:

- allocator page table page tổng quát;
- range VA allocator nhiều vùng;
- unmap toàn bộ VSpace;
- ASID/PCID;
- thu hồi frame.

Root VSpace và server VSpace đã tách, nên client không còn ghi được vào stack
server. Client vẫn dùng root VSpace nên có thể ghi vào stack root/fault; Bước 7
mới tách hoàn toàn client.

## 8. W^X hiện tại

| Port | Code | Stack/Data |
|---|---|---|
| RISC-V | RX + U | RW + NX |
| AArch64 | RO + X + U | RW + NX |
| x86_64 | RX + U | RW + NX data/stack |

Kernel từ chối mọi mapping user yêu cầu đồng thời `WRITE` và `EXEC`.
Ngoài ra mỗi `Frame` giữ `mapped_w`/`mapped_x`; `FrameMap` từ chối W nếu Frame
đã có leaf X và từ chối X nếu Frame đã có leaf W. Vì vậy hai alias của cùng
một Frame không thể tạo ra một trang vừa ghi vừa chạy được. `FrameUnmap`,
revoke VSpace/PageTable và destroy Frame đều giảm bộ đếm này; host test và
`check_invariants()` đối chiếu bộ đếm với reverse map. Mapping cũng bắt buộc
có `READ` trên mọi kiến trúc.
