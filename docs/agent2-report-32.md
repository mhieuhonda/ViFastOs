# ViFastOs — Báo cáo vòng 32 (budget dùng chung, mở mục tiêu dài hạn)

**Ngày:** 2026-11-06
**HEAD đầu vòng:** `7958e8e`
**Mốc:** Pha 1 mục tiêu dài hạn — thống nhất guard static storage.

## 1. Thay đổi

- Thêm `vifast_kernel::KERNEL_STATIC_BUDGET = 716_800` vào
  `crates/vifast-kernel/src/object.rs` (export qua `pub use object::*`).
- `crates/vifast-runtime/src/lib.rs` dùng const dùng chung thay vì const cục bộ.

Không đổi hành vi boot; chỉ dọn nợ kỹ thuật vòng 30 để mọi platform và host
test cùng tham chiếu một ngưỡng.

## 2. Số liệu cổng

- Host: kernel **134**, abi 4, arch 7, pte 11, elf 17, acpi 8, fs 22 → **203**.
- QEMU chuẩn **3/3 ×3**; persistence 7×3 PASS; harness **462**; warning 0.
- `cargo fmt --check` PASS; `check-docs.py` PASS (sau `gen-docs.py`).

## 3. Mục tiêu dài hạn M1→M3

1. **Pha 1 (đang làm):** an toàn static + giảm footprint (budget chung, slot
   48 xong, assert linker 3 arch còn lại).
2. **Pha 2:** pager v2 tổng quát + demand-load (`docs/design/pager-v2.md`).
3. **Pha 3:** VirtIO net + stack tối thiểu (ARP/IPv4/ICMP/UDP).
4. **Pha 4:** display/input MVP (`docs/design/display-input-v1.md`).
