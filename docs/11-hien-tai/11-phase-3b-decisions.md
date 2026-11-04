# 11.11. Quyết định thiết kế cho Phase 3b (VSpace riêng)

Tài liệu này chốt hướng trước khi triển khai; code hiện tại vẫn dùng một VSpace
chung cho root/server/client.

## 1. Kernel mapping

- RISC-V và x86_64: mỗi VSpace copy các entry cấp cao nhất của kernel identity
  map khi tạo; đánh dấu Global để không bị flush khi đổi task.
- AArch64: chuyển kernel sang **TTBR1**; user dùng TTBR0. Không copy kernel map
  theo từng VSpace.
- Không đưa kernel mapping vào shadow `VSpace.mappings`.

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
