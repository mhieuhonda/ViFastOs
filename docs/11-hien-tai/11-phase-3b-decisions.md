# 11.11. Quyết định thiết kế cho Phase 3b (VSpace riêng)

Tài liệu này chốt hướng trước khi triển khai; code hiện tại vẫn dùng một VSpace
chung cho root/server/client.

## 1. Kernel mapping

- RISC-V: copy entry gốc của kernel identity map vào root mới; đánh dấu Global.
- x86_64 và AArch64: **không copy entry cấp cao nhất** vì kernel và user cùng
  entry gốc. Mỗi VSpace có PML4/L0 riêng, entry 0..1 GiB trỏ tới PD/L2 kernel
  dùng chung; entry user trỏ tới bảng riêng của task.
- Retype VSpace trên x86/AArch64 cần thêm page cho bảng gốc/trung gian.
- Không đưa kernel mapping vào shadow `VSpace.mappings`.
- TTBR1 và high-half kernel là giai đoạn riêng sau 3b.

## 2. Chuyển address space

- `Context`/TCB lưu `root` của VSpace.
- Runtime ghi `satp` (RISC-V), `TTBR0` (AArch64) hoặc `CR3` (x86) khi switch.
- Chưa có ASID/PCID: flush TLB phần user khi đổi root; kernel Global giữ lại.

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

- Expose `PageTableMap`.
- Retype VSpace/PageTable từ Untyped.
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
