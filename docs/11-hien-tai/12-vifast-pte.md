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
```

`decode_at` cần cho AArch64 vì page descriptor L3 và table descriptor dùng
cùng bit thấp `0b11`.

## 3. Quy tắc

- `W^X`: leaf không được vừa write vừa exec.
- RISC-V: W không hợp lệ nếu thiếu R; encoder tự thêm R khi W.
- x86: leaf có U/S, RW, NX; table có P|W|U.
- AArch64: page L3 vs table; block L1/L2; AP[2:1], UXN/PXN.

## 4. Test host

- roundtrip leaf/table;
- quyền RX/RW/NX;
- x86 large leaf 2 MiB;
- AArch64 table/page/block;
- index biên 64 GiB, 1 GiB−1, 256 GiB−1.
