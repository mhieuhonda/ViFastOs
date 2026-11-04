# 03.6. Ngắt, driver ở user space, MMIO, DMA, IOMMU

Mã nguồn: `vifast-kernel/src/irq.rs`, `vm.rs` (IOMMU); `vifast-sim/src/hw.rs` (thiết bị); Python `irq.py`, `hw.py`.

## 1. Driver là tiến trình

Một driver chỉ là tiến trình user có thêm capability:

| Cap | Cho phép |
|---|---|
| Frame thiết bị (map vào VSpace) | Đọc/ghi thanh ghi MMIO trực tiếp, không qua kernel |
| IrqHandler | Nhận ngắt của đúng một đường IRQ, ack |
| Notification (bound) | Nơi ngắt được giao tới |
| IoSpace | Cho thiết bị DMA vào các trang driver chỉ định |
| Endpoint | Phục vụ request từ client |

Driver lỗi chỉ làm chết tiến trình driver. Supervisor restart nó. Kernel không chứa mã của bất kỳ thiết bị cụ thể nào (ngoài bộ điều khiển ngắt và timer trong `vifast-core`).

## 2. Đường đi của ngắt

```
thiết bị ─► PLIC ─► trap S-mode ─► vifast-core: claim IRQ
            ─► kernel.deliver_irq(irq):
                 chưa ai đăng ký → spurious_irq++
                 đang che (masked) hoặc chưa gắn notification → pending = true
                 ngược lại → masked = true, signal notification
driver: nhận tín hiệu, đọc thanh ghi, xử lý, IrqAck
            ─► masked = false; nếu pending → giao ngay
```

- Ngắt bị **che từ lúc giao tới lúc ack**: driver không bị ngập ngắt khi đang xử lý.
- Ngắt tới lúc đang che **gộp lại** thành một pending (test `irq_masked_until_ack`: 3 ngắt, driver thấy 2 lần).
- Trên phần cứng: "che" tương ứng với không `complete` ở PLIC cho tới khi ack (hoặc tắt enable).

**Chú ý:** driver phải đọc hết trạng thái thiết bị mỗi lần được đánh thức, vì nhiều sự kiện có thể đã gộp vào một tín hiệu.

## 3. MMIO

Frame thiết bị được tạo lúc boot (`Frame { device: true }`, `paddr = mmio_base`), không thuộc RAM, không bị xoá trắng, không IoMap được. Map vào driver với quyền RW (không bao giờ X). Truy cập MMIO là lệnh load/store bình thường; MMU dịch sang địa chỉ vật lý thiết bị, bus chuyển tới thiết bị.

Thiết bị mẫu `DmaSensor`: 4 thanh ghi 8 byte (DMA_ADDR, DMA_LEN, CTRL, STATUS). Ghi CTRL = 1 thì sau 3 tick thiết bị DMA chuỗi `SENSOR-n` vào DMA_ADDR rồi phát ngắt.

## 4. Vì sao bắt buộc phải có IOMMU

MMU chỉ kiểm soát **CPU**. Thiết bị DMA truy cập bộ nhớ trực tiếp, không đi qua MMU. Driver ở user space bị MMU cách ly hoàn hảo nhưng vẫn có thể **ra lệnh cho thiết bị** ghi vào bất kỳ địa chỉ vật lý nào, kể cả vùng kernel.

Test `without_iommu_dma_corrupts_kernel`: driver độc hại đặt DMA_ADDR = đầu RAM (ảnh kernel), canary của kernel bị ghi đè. Test `iommu_blocks_malicious_dma`: có IOMMU, yêu cầu bị chặn, STATUS = ERROR, kernel nguyên vẹn.

Kết luận: **microkernel với driver ở user space chỉ thực sự cô lập khi có IOMMU** (Intel VT-d, AMD-Vi, ARM SMMU, RISC-V IOMMU). Thiếu IOMMU thì driver dùng DMA phải được xếp vào TCB (tin tưởng hoàn toàn).

## 5. IoSpace

Mỗi thiết bị có một IoSpace (bảng dịch IOMMU): trang IOVA → trang vật lý + quyền.

- `IoMap(frame, iospace, iova, rights)`: cho thiết bị truy cập frame tại IOVA. Frame thiết bị bị từ chối. Quyền = quyền yêu cầu ∩ quyền cap frame.
- `IoUnmap(frame)`, xoá cap frame, huỷ IoSpace: gỡ ánh xạ.
- `dma_plan`: dịch từng trang của một yêu cầu DMA; một trang không hợp lệ thì chặn **toàn bộ** yêu cầu (không ghi một phần).

**Chú ý trên phần cứng:** sau IoUnmap phải invalidate IOTLB của IOMMU và đợi xác nhận trước khi tái sử dụng frame, tương tự TLB shootdown. Frame đang được thiết bị DMA không được thu hồi khi thiết bị còn đang chạy; driver phải dừng thiết bị trước (hoặc supervisor reset thiết bị khi restart driver).

## 6. Khởi động lại driver

Driver chết giữa chừng có thể để thiết bị ở trạng thái dở dang (đang DMA, ngắt đang che). Quy trình khuyến nghị cho supervisor/driver manager:
1. Reset thiết bị (qua thanh ghi reset hoặc nguồn).
2. Thu hồi IoMap cũ.
3. Restart driver; driver khởi tạo thiết bị từ đầu.
4. Client đang chờ nhận `CANCELLED`, tự gửi lại.

## 7. Hướng mở rộng
- IOMMU bằng page table thật trong RAM (RISC-V IOMMU spec), thay `BTreeMap`.
- Driver PCIe: cấu hình space qua cap, MSI/MSI-X thành notification.
- Thư viện `vifast-driver` dùng chung: wrapper an toàn cho MMIO (`volatile`), vòng DMA, mô hình async (driver dùng executor nhỏ kiểu Embassy trên notification).
