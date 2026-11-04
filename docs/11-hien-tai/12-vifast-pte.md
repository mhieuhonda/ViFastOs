# 11.12. `vifast-pte` — encode/decode PTE an toàn

Nguồn: `crates/vifast-pte/src/lib.rs`.

## 1. Mục tiêu

Tách toàn bộ phần tính bit descriptor khỏi platform để test trên host:

- Sv39;
- x86_64 4-level;
- AArch64 4 KiB granule.

Crate là `no_std`, `forbid(unsafe_code)`. Platform chỉ còn `read_pte`,
`write_pte`, `flush_*` và `zero_frame`.

## 2. API

Mỗi format cung cấp:

```text
LEVELS
index(va, level)
encode_leaf(pa, rights)
encode_table(pa)
decode(pte)
decode_at(pte, level)
table_span(level)
USER_VA_START / USER_VA_END
```

`decode_at` cần cho AArch64 vì page descriptor L3 và table descriptor dùng
cùng bit thấp `0b11`.

## 3. Quy tắc

- `W^X`: leaf không được vừa write vừa exec.
- RISC-V: W không hợp lệ nếu thiếu R; encoder tự thêm R khi W.
- x86: leaf có U/S, RW, NX; table có P|W|U.
- AArch64: page L3 vs table; block L1/L2; AP[2:1], UXN/PXN.
- Dải user 64..128 GiB: nằm ngoài kernel identity, RISC-V vẫn trong nửa thấp
  Sv39; x86/AArch64 dùng entry 0 dùng chung nhưng không thay entry này.
- `table_span(level)` là kích thước VA mà một bảng ở level đó phủ; dùng để
  kiểm tra toàn dải trước khi PageTableMap ghi PTE.

## 4. Test host

- roundtrip leaf/table;
- quyền RX/RW/NX;
- x86 large leaf 2 MiB;
- AArch64 table/page/block;
- index biên 64 GiB, 1 GiB−1, 256 GiB−1;
- dải user không giao kernel identity;
- table_span level 1/2/3 và biên range.
