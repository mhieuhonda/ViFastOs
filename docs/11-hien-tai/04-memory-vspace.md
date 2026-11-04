# 11.4. Bộ nhớ ảo, Frame/VSpace/PageTable và W^X

Nguồn: `crates/vifast-kernel/src/object.rs`,
`crates/vifast-kernel/src/lib.rs`,
`crates/vifast-runtime/src/lib.rs`,
`crates/vifast-platform-*/src/platform.rs`.

## 1. Mô hình chung

Kernel portable đi **generic page-table walk** qua `PageTableOps`. Page table
thật là nguồn sự thật duy nhất; `VSpace.mappings` chỉ còn là reverse map cho
revoke/validation.

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

- root handle;
- page size;
- range `[base, end)`;
- `next_va` cho cấp phát tuần tự;
- `code_base/code_len`, `stack_base/stack_len`;
- `aux_pa` cho PDPT/L1 riêng của VSpace retype trên x86/AArch64;
- tối đa 32 mapping (reverse map).

`validate_user_range()` kiểm tra buffer user theo code/stack/mapping trước
`DebugWrite`, `Open`, `Read`, `Write`. Đây là hàng rào bảo mật chính; platform
copy chỉ là lớp thứ hai. Đây vẫn là mô hình tạm thời: khi bỏ code/stack window
ở 3b, hàm này sẽ walk leaf thật qua `vifast-pte` và chỉ dựa vào page table.

`Kernel::map_frame` kiểm tra:

- rights chỉ gồm `READ|WRITE|EXEC`;
- không đồng thời `WRITE` và `EXEC` (W^X);
- PA căn page;
- Frame đủ một page;
- VA căn page;
- VA nằm trong range;
- không overlap mapping cũ.

## 3. RISC-V64

File: `crates/vifast-platform-riscv64/src/platform.rs`.

- Sv39, root tĩnh `ROOT`.
- Kernel identity map `0x8000_0000..0x803f_ffff`.
- UART `0x1000_0000` map 2 MiB.
- User range `0x0000_0010_0000_0000..0x0000_0020_0000_0000` (64..128 GiB);
  boot window code `+0`, stack `+0x1000..+0x8fff`, data `+0xa000` là tạm thời.
- Code: `R|X|U` (không W).
- Stack: `R|W|U` (không X).
- Data mapping qua `FrameMap`: `R|W|U` hoặc `R|X|U` theo quyền.
- `sfence.vma` sau map/unmap.

Trap entry dùng kernel trap stack qua `sscratch`; `sscratch=0` khi ở kernel
để phát hiện trap từ S-mode. `SUM=0`; copy user bật `SUM` tạm thời.

## 4. AArch64

File: `crates/vifast-platform-aarch64/src/platform.rs`.

- 4-level page table tĩnh.
- GICv2 `0x0800_0000`, PL011 `0x0900_0000` map trong L1[0]; RAM kernel
  `0x4000_0000..` map trong L1[1]. `KERNEL_SHARED_SLOTS = [0, 1]`.
- User range `0x0000_0010_0000_0000..0x0000_0020_0000_0000` (64..128 GiB);
  boot window code `+0`, stack `+0x1000..+0x8fff`, data `+0xa000` là tạm thời.
- Code: EL0 read-only + executable (`AP_USER_RO`, UXN=0).
- Stack: EL0 read-write + NX (`AP_USER_RW`, UXN=1).
- Data: EL0 read-write + NX.
- Map/unmap dùng `tlbi vae1is`, `dsb ish`, `isb`.
- PAN nếu CPU hỗ trợ; read/write user tạm clear PAN.

Lưu ý encoding `AP[2:1]`:
- `AP_USER_RW = 0b01` (bit 6);
- `AP_USER_RO = 0b11` (bit 6 + bit 7) để EL0 đọc được.

## 5. x86_64 UEFI

File: `crates/vifast-platform-x86_64/src/platform.rs`.

- Tự dựng PML4/PDPT/PD/PT 4 KiB, không sửa page table UEFI.
- Identity map 0..1 GiB cho kernel/UEFI image, stack, GDT/TSS/IDT; PDPT[0]
  là shared slot, PML4[0]/PDPT là private mỗi VSpace.
- User range `0x0000_0010_0000_0000..0x0000_0020_0000_0000` (64..128 GiB);
  boot window code `+0`, stack `+0x1000..+0x8fff`, data `+0xa000` là tạm thời.
- Code: P|US, read+execute, NX clear.
- Stack/data: P|W|US|NX.
- `FrameMap` map đúng VA được yêu cầu; `FrameUnmap` clear PTE + `invlpg`.
- `VSpace.direct` đã bị xoá; x86/AArch64 dùng chung root entry 0 nhưng tách
  bảng user; PageTableMap không thay entry 0.
- SMEP/SMAP nếu CPUID hỗ trợ; user copy dùng STAC/CLAC.
- CR3 được chuyển sang PML4 riêng ngay trước `iretq`.
- Hạn chế còn lại: identity map 0..1 GiB; chưa có AP; data kernel chưa NX.
## 6. TLB

Hiện tại:

- RISC-V: `sfence.vma` một hart.
- AArch64: `tlbi vae1is` một hart.
- x86: `invlpg`.

Chưa có IPI shootdown đa hart. Khi thêm SMP scheduler, cần:

1. Theo dõi hart nào đang dùng VSpace.
2. Gửi IPI tới các hart đó.
3. Chờ acknowledge trước khi trả frame/page table.

## 7. Page table allocator

`PageTable` object đã có metadata `{root, mapped: {vspace, va_base, level}}` và PageTableMap.
Retype VSpace đã cấp root + aux page, zero và gọi `install_kernel`; runtime
root vẫn dùng bảng tĩnh cho tới khi switch root ở Bước 4. Chưa có:

- allocator page table page tổng quát;
- range VA allocator nhiều vùng;
- unmap toàn bộ VSpace;
- ASID/PCID;
- thu hồi frame.

Hiện tại server/client/root dùng chung một VSpace và cùng stack window. Vì vậy
capability đã tách nhưng bộ nhớ chưa tách: client có thể ghi vào stack của
server. Đây là lý do chính phải làm VSpace riêng khi thêm ELF loader.

## 8. W^X hiện tại

| Port | Code | Stack/Data |
|---|---|---|
| RISC-V | RX + U | RW + NX |
| AArch64 | RO + X + U | RW + NX |
| x86_64 | RX + U | RW + NX data/stack |

Kernel từ chối mọi mapping user yêu cầu đồng thời `WRITE` và `EXEC`.
