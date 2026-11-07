# 11.15. Software IOMMU, W^X chéo CPU/DMA và audit vòng 99–101

Nguồn: `crates/vifast-kernel/src/lib.rs` (`iommu_map`/`iommu_unmap`,
`map_frame`/`remap_frame`, `copy_frame_bytes`, `IoMap`/`IoUnmap`,
`destroy_object`, `can_destroy`), `docs/audits/2026-11-07-round99-soi-chay-K32-K75.md`.

## 1. Mô hình IOMMU software

- Mỗi domain có reverse map `[Option<IommuMapping>; 32]`, IOVA window
  `IOMMU_IOVA_BASE .. +0x4000_0000`, chỉ `READ`/`WRITE` (không `EXEC`).
- `insert`/`remove` xong đều `flush_iotlb()` (hiện là no-op hook, K12 —
  phần cứng sau này điền thật).

## 2. W^X chéo CPU/DMA (vòng 101, K55)

`mapped_w`/`mapped_x` đếm trên Frame object, không phải trên từng mapping.
Quy tắc khép kín sau vòng 101:

- `iommu_map(WRITE)` cộng `mapped_w` (DMA-WRITE cũng là WRITE) và từ chối
  khi `mapped_x != 0`;
- `map_frame`/`remap_frame` EXEC từ chối khi `mapped_w != 0` (kể cả phần do
  DMA cộng);
- `iommu_unmap` / `unmap_iommu_frame_everywhere` / destroy domain trừ lại
  `mapped_w` đúng frame.

Trước đó `iommu_map(WRITE)` + `map_frame(EXEC)` cùng PA đều lọt (1 PA vừa
CPU-X vừa DMA-W). Invariant checker so sánh `mapped_w`/`mapped_x` với
reverse map CPU nhưng mù DMA-W — khoảng trống đã biết, chưa lấp.

## 3. Gate quyền IoMap/IoUnmap (vòng 101, K73/K74)

- `IoMap`: vé Frame phải **chứa** quyền DMA yêu cầu (R-only + DMA-WRITE →
  `NoRights`). Trước đó chỉ cần `Frame:READ` là đủ.
- `IoUnmap`: caller xuất trình vé Iommu-WRITE **và** vé Frame khớp
  `(pa, object)` của mapping tại IOVA (gương `FrameUnmap`). Trước đó task B
  có domain-WRITE gỡ được DMA của task A. Đổi ABI:
  `iommu_unmap(domain, iova, frame_slot)`.

## 4. Destroy domain còn mapping sống (vòng 101, K56)

- `can_destroy` trả false khi domain còn mapping → `revoke` từ Untyped bị
  chặn (trước đó revoke giết domain sống, PA tái cấp trong khi IOTLB cũ
  còn trỏ).
- `destroy_object` nhánh Iommu: gỡ mọi mapping, trừ `mapped_w` từng frame,
  flush IOTLB (phòng thủ sâu cho đường gọi trực tiếp).

## 5. Danh mục audit K32–K75

File soi chay: `docs/audits/2026-11-07-round99-soi-chay-K32-K75.md`
(44 phát hiện, 4 subagent xác minh độc lập họ B/C/D/E + agent chính họ A).

| Kết luận | Các K | Ghi chú |
|---|---|---|
| Đúng, đã vá | 37 | K32–K44, K47–K59, K61–K69, K72–K75 |
| Sai (false positive, có chứng minh) | K45, K46, K60 | K45: bit đã tiêu thụ qua message; K46: poll/wait không rearm là thiết kế; K60: mask ghost chỉ flush thừa |
| Mismatch vệ sinh | K72 | runtime đã clamp đúng, chỉ cần phát `len` đã kẹp cho đồng nhất |

Bài học lớn nhất: siết `resume` cấm `BlockedFault` gãy pager flow (mục 3
file 14). Mọi siết syscall phải chạy QEMU pager trước khi kết luận.
