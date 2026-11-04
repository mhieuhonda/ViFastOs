# 11.4. Bộ nhớ ảo, Frame/VSpace/PageTable và W^X

Nguồn: `crates/vifast-kernel/src/object.rs`,
`crates/vifast-kernel/src/lib.rs`,
`crates/vifast-runtime/src/lib.rs`,
`crates/vifast-platform-*/src/platform.rs`.

## 1. Mô hình chung

Kernel portable chỉ giữ **shadow mapping**. Nó không biết địa chỉ thanh ghi
page table, không đụng MMIO và không thực hiện `sfence.vma`/`tlbi`/`invlpg`.

Trình tự map:

```text
FrameMap syscall
  → Kernel::map_frame() kiểm tra shadow
  → UserAction::MapFrame
  → Platform::map_user_page()
  → Platform flush TLB
  → Runtime trả VA cho user
```

Trình tự unmap tương tự với `FrameUnmap`.

## 2. VSpace

`VSpace` lưu:

- root handle;
- page size;
- range `[base, end)`;
- `next_va` cho cấp phát tuần tự;
- cờ `direct` cho x86 identity mapping;
- tối đa 8 mapping.

`Kernel::map_frame` kiểm tra:

- rights chỉ gồm `READ|WRITE|EXEC`;
- không đồng thời `WRITE` và `EXEC` (W^X);
- PA căn page;
- Frame đủ một page;
- VA căn page;
- VA nằm trong range (trừ direct);
- không overlap mapping cũ.

## 3. RISC-V64

File: `crates/vifast-platform-riscv64/src/platform.rs`.

- Sv39, root tĩnh `ROOT`.
- Kernel identity map `0x8000_0000..0x803f_ffff`.
- UART `0x1000_0000` map 2 MiB.
- User code `0x4000_0000`, stack `0x4000_1000..0x4000_8fff`,
  data `0x4000_a000`.
- Code: `R|X|U` (không W).
- Stack: `R|W|U` (không X).
- Data mapping qua `FrameMap`: `R|W|U` hoặc `R|X|U` theo quyền.
- `sfence.vma` sau map/unmap.

Hạn chế: trap entry hiện dùng user stack, chưa có `sscratch` kernel trap stack.

## 4. AArch64

File: `crates/vifast-platform-aarch64/src/platform.rs`.

- 4-level page table tĩnh.
- GICv2 `0x0800_0000`, PL011 `0x0900_0000` map device block.
- User code `0x2000_0000`, stack `0x2000_1000..0x2000_8fff`,
  data `0x2000_a000`.
- Code: EL0 read-only + executable (`AP_USER_RO`, UXN=0).
- Stack: EL0 read-write + NX (`AP_USER_RW`, UXN=1).
- Data: EL0 read-write + NX.
- Map/unmap dùng `tlbi vae1is`, `dsb ish`, `isb`.

Lưu ý encoding `AP[2:1]`:
- `AP_USER_RW = 0b01` (bit 6);
- `AP_USER_RO = 0b11` (bit 6 + bit 7) để EL0 đọc được.

## 5. x86_64 UEFI

File: `crates/vifast-platform-x86_64/src/platform.rs`.

- Dùng page table UEFI; `vspace_root()` đọc CR3.
- `VSpace.direct = true`: VA trả về = PA của `USER_FRAME`.
- `make_user_page` bật U/S, RW, NX cho data; code executable nhưng vẫn RW.
- Khi sửa page table phải tạm clear `CR0.WP`.
- `FrameUnmap` hiện chỉ `invlpg`, không thu hồi U/S trên leaf 2 MiB để tránh
  mất luôn code/stack trong cùng 2 MiB.
- Vì vậy x86 chỉ là bring-up, chưa đạt W^X/isolation production.

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

`PageTable` object đã có metadata nhưng page table root vẫn là **bảng tĩnh**
trong platform. Chưa có:

- allocator page table page tổng quát;
- range VA allocator nhiều vùng;
- unmap toàn bộ VSpace;
- ASID/PCID;
- thu hồi frame.

## 8. W^X hiện tại

| Port | Code | Stack/Data |
|---|---|---|
| RISC-V | RX + U | RW + NX |
| AArch64 | RO + X + U | RW + NX |
| x86_64 | RW + X (UEFI large page) | RW + NX data |

Kernel từ chối mọi mapping user yêu cầu đồng thời `WRITE` và `EXEC`.
