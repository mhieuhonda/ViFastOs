# Inbox cho agent0

## [vòng 26] AArch64 crash khi thêm bộ đếm pool vào kernel
- Bối cảnh: thêm field HWM vào `Kernel`/static `AtomicU16` và ghi trong
  `alloc_owned` làm AArch64/x86_64 báo `KERNEL EXCEPTION vector=4` ngay sau
  `BOOTINFO OK` (RISC-V vẫn xanh). Revert toàn bộ giữ baseline.
- Các phương án: (a) tính HWM hoàn toàn ngoài kernel (runtime sample tại các
  mốc), (b) chỉ đo current counts, (c) debug riêng AArch64 với static tối thiểu.
- Đề xuất agent2: (c) trước, vì đây có thể là lỗi nền về vị trí static/AT.
- Đang chặn: M1.1 (POOL HWM) | Không chặn M0.2/M1.2
- **Cập nhật vòng 28:** đã thử biến thể thứ 4 — sampling **read-only** từ runtime
  (`Kernel::pool_counts()` + atomic trong runtime, không ghi gì vào kernel)
  trên nền M1.2c. AArch64 vẫn lỗi (`USER FAULT tcb=None`, `PANIC hart=64`) sau
  `BOOTINFO OK`. Đã revert. HWM BLOCKED 2 phiên liên tiếp → theo mục 2.5 không
  thử lần 3; chuyển sang M1.3 (tách TCB pool) và sẽ đo HWM sau bằng số slot
  pool đang dùng nếu runtime sampling còn lỗi.
- Nhánh đang đi trong lúc chờ: M0.2 (xong) + M1.2.
- Đã xử lý phần ROOT ERR 16: nguyên nhân là **runtime vẫn index CONTEXTS theo
  `tcb.index`** sau khi mảng đổi sang MAX_TCBS; ConfigureTcb dùng get_mut(OOB)
  nên trả IllegalOperation. Sửa runtime dùng `context_slot` (commit M1.2c).
  HWM AArch vẫn chưa thử lại sau sửa này.

## [vòng 26] Ngân sách pager v2
- Bối cảnh: data 368 KiB + `MAX_OBJECTS` 163 không khả thi với headroom.
- Đề xuất: tách pool object (M1) để giảm `Object` từ 4.752 xuống ≤256 B.
- Đang chặn: M2 pager v2 | Nhánh: M1 → M2.

## [vòng 28] Tách TCB pool lần đầu — AArch64 KERNEL EXCEPTION
- Bối cảnh: đã tách `Kind::Tcb(TcbId)` + `Kernel.tcbs: [Option<Tcb>; 22]`,
  host test 134/134 PASS, Object 4.752→4.152, Kernel 710.248→706.576.
  Nhưng QEMU: RISC-V 3/3, AArch64 0/3 (`KERNEL EXCEPTION vector=4`,
  `ELR=0x40092fa0` địa chỉ vật lý, `FAR=0xab5b8`), x86 2/3 (1 timeout).
- Đã revert để giữ baseline xanh; đây là lần thử 1/3.
- Giả thuyết: lỗi nền AArch64 với thay đổi layout bằng `.bss`/object table
  (VBAR/MMU/stack?), không phải logic pool (host test xanh).
- Đang chặn: M1.3 | Nhánh tạm: M1.4 docs? hoặc debug AArch tối thiểu.

## [vòng 29] CNode pool thử — cùng lỗi AArch64 layout (stash riêng)
- Bối cảnh: tách `Kind::CNode(CNodeId)` + `Kernel.cnodes[8]` (stash@{0}).
  Host 135/135, RISC-V QEMU 3/3, nhưng **AArch64 0/3**
  (`KERNEL EXCEPTION vector=4 cur-spx-sync ELR=0x4009201c FAR=0xb4410`).
- Phát hiện: tách CNode riêng **không giảm Object** (CNode 4.096 vẫn nằm
  trong pool) mà phình Kernel 710.248 → 743.016 (+32.768). Đã stash riêng,
  chỉ commit layout probe (`LAYOUT objs=0 root_cspace=698544 current=705552`).
- QEMU 3/3×3 + persist 7×3 + harness 462 xanh với layout probe.
- Đề xuất tiếp: giảm `MAX_CAPS_TASK` 64→48, hoặc tách `Tcb.cspace/fds` trước.
