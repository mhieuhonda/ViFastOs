# 03.5. Bộ nhớ ảo: Sv39, TLB, ASID, shootdown, W^X

Mã nguồn: `vifast-kernel/src/vm.rs`; MMU giả lập trong `vifast-sim/src/machine.rs`; Python `vm.py`.

## 1. Sv39 (RISC-V)

- Địa chỉ ảo 39 bit, user dùng nửa thấp (`< 2^38`).
- 3 cấp page table, mỗi bảng 4 KB = 512 PTE × 8 byte.
- `va` chia thành: VPN[2] (bit 30..38), VPN[1] (21..29), VPN[0] (12..20), offset (0..11).
- PTE: `PPN << 10 | flags`, flags: V(1) R(2) W(4) X(8) U(16).
  - V=1 và R=W=X=0: trỏ tới bảng cấp dưới.
  - V=1 và có R/W/X: lá.

```
VSpace.paddr (bảng cấp 2) ─VPN[2]─► PageTable cấp 1 ─VPN[1]─► PageTable cấp 0 ─VPN[0]─► Frame
```

Page table là cấu trúc thật nằm trong RAM (`PhysMem` ở simulator). Kernel đọc/ghi PTE qua `Platform::phys_read/phys_write`. MMU giả lập đi bảng bằng đúng hàm `vm::walk` khi TLB miss. Test `sv39_page_tables_live_in_physical_memory`.

## 2. Ai cấp page table?

Kernel **không tự cấp**. User Retype `PageTable` rồi `PageTableMap`: kernel gắn nó vào cấp trống đầu tiên trên đường tới `vaddr` (cấp 2 trước, rồi cấp 1). `FrameMap` khi thiếu bảng trung gian trả `MISSING_PAGE_TABLE`. Pager và boot đều xử lý bằng vòng lặp: thiếu thì retype thêm bảng rồi thử lại.

Lý do: nếu kernel tự cấp page table thì cần heap kernel, và một task có thể làm kernel hết bộ nhớ bằng cách map rải rác. Để user trả bằng untyped của mình thì chi phí tính đúng cho người gây ra.

## 3. Quyền và W^X

`leaf_flags(rights)`:
- R → PTE_R; W → PTE_W | PTE_R (RISC-V cấm W mà không R); X → PTE_X.
- Không có R lẫn X: `INVALID_ARG`.
- **W và X cùng lúc: `ILLEGAL_OPERATION`** (chính sách W^X). Một trang không bao giờ vừa ghi vừa thực thi được, chặn lớp tấn công tiêm mã. JIT phải map hai lần (một bản W, một bản X ở địa chỉ khác) hoặc đổi quyền theo pha.

Test `wx_and_exec_permission`: RWX bị từ chối, RX được, Fetch trên trang RW gây fault.

## 4. TLB và ASID

Mỗi hart có TLB riêng (64 mục, FIFO ở simulator). Mục TLB gắn **ASID** của VSpace, nên đổi tiến trình không cần xoá TLB. ASID cấp khi Retype VSpace, trả lại khi VSpace bị huỷ (test `vspace_destroy_frees_asid`). Hết ASID: `OUT_OF_MEMORY` khi Retype VSpace.

## 5. TLB shootdown

Khi gỡ một ánh xạ, PTE trong RAM bị xoá nhưng **các hart khác có thể vẫn giữ bản dịch cũ trong TLB**. Nếu không xoá, thread trên hart khác tiếp tục đọc/ghi trang đã bị thu hồi (có thể đã cấp cho tiến trình khác): lỗ hổng nghiêm trọng.

`shootdown(asid, vpn)`: xoá mục TLB tương ứng trên mọi hart; với hart khác hart hiện tại thì gửi IPI (SBI remote fence trên phần cứng).

Test `tlb_shootdown_across_harts`: hai thread chung VSpace trên hai hart. Có shootdown: reader fault ngay sau unmap. Tắt shootdown (`tlb_shootdown_enabled = false`, chỉ để minh hoạ): reader vẫn đọc được sau khi unmap.

**Chú ý trên phần cứng thật:** shootdown phải **đồng bộ**: hart gọi unmap chỉ được trả về (và chỉ được tái sử dụng frame) sau khi mọi hart khác đã xác nhận xoá TLB. Simulator xoá tức thì nên không thấy được vấn đề này.

## 6. Unmap và huỷ VSpace

- `FrameUnmap`: xoá PTE (chỉ khi PTE vẫn trỏ đúng frame này), gỡ khỏi bản đồ ngược, shootdown trang đó.
- Xoá cap frame đang map: tự unmap.
- Huỷ PageTable đang map: xoá PTE trỏ tới nó, shootdown toàn ASID.
- Huỷ VSpace: gỡ cờ `mapped` ở mọi cap (qua bản đồ ngược `frames`, `tables`), shootdown toàn ASID, trả ASID.

## 7. Chia sẻ bộ nhớ

Copy cap frame, map bản copy vào VSpace khác (có thể quyền khác, ví dụ producer RW, consumer R). Revoke cap gốc xoá mọi bản copy, kéo theo unmap mọi nơi. Đây là cơ chế cho dữ liệu lớn giữa các tiến trình (ảnh camera, buffer âm thanh, framebuffer).

## 8. Hướng mở rộng
- Superpage (2 MB, 1 GB) để giảm TLB miss.
- Bit A/D (accessed/dirty) cho pager làm swap hoặc copy-on-write.
- PCID/ASID tái sử dụng có quản lý thế hệ (ASID generation) như Linux khi ASID ít.
- Chuyển sang Sv48 nếu cần không gian địa chỉ lớn.
