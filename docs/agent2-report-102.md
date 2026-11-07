# ViFastOs — Báo cáo vòng 102 (mục tiêu mới: từ demo sang OS dùng được)

**Ngày:** 2026-11-07 (tối). **HEAD:** `fcfe5a9`. Baseline gate xanh
(kernel 156, QEMU 3/3×3, persist 7×3, harness 508, soak 30/30).

## 1. Lộ trình 12 mốc (lõi trước, vòng ngoài sau)

### Lõi (ưu tiên 1–7)

1. **Process/lifecycle hoàn chỉnh** — Exit/Wait/Reap, destroy-cả-nhóm,
   fault-endpoint Reply-resume tổng quát; hết kẹt BlockedReply; `leaked=0`.
2. **Memory allocator tổng quát** — free-page + page-table-page toàn cục;
   churn/fuzz 100k bước `leaked=0`; nới object table/TCB quota có kiểm chứng.
3. **FS bền ngoài QEMU** — nâng giới hạn (4MiB/64 inode/tên 28B/txn 16 block),
   crash-test torn-commit/power-loss replay đúng, barrier HW thật.
4. **Driver block/net IRQ-driven** — bỏ polling-fallback; 3 arch
   `BLOCK OK/FS DISK OK + VIRTIO NET READY` ổn định repeat/soak, không retry
   RX descriptor.
5. **IPC/sched mở rộng** — CNode đa cấp, Reply-mang-cap, hàng đợi lớn hơn,
   SMP ≥4 hart + soak dài, đo BKL contention để quyết khóa mịn.
6. **Pager/mount tổng quát + shared-mem** — demand-load từ file VFS thật,
   mmap/munmap, mount đa điểm, shared-frame dùng chung.
7. **Bảo mật cơ bản** — HW-IOMMU, NX toàn kernel, FP/SIMD không rò rỉ,
   phân quyền FS/badge qua harness âm, fuzz syscall xanh.

### Vòng ngoài (8–12)

8. **Shell + job control** — `&/jobs/fg`, Ctrl-C, lịch sử, dòng ≥256B.
9. **Userspace/libc tối thiểu + name/mount/exec** — bỏ slot VFS cứng,
   `run /hello` nạp ELF từ FS.
10. **Net stack thành dịch vụ** — socket IPC, ping/UDP+DNS/TCP-echo qua
    user-net QEMU.
11. **GPU/input service + compositor MVP** — blit ≥2 client, focus, text field.
12. **ViFastKey + tài liệu vận hành** — Telex ra tiếng Việt, `docs/van-hanh.md`.

## 2. Quy tắc vòng

Mỗi mốc: một/nhiều commit nhỏ, gate xanh mới merge ý tưởng vào main
(quick gate mỗi vòng, full gate + soak khi xong mốc). Đo được = tiêu chí
trong mục 1.
