# ViFastOs kernel model (Python)

Đặc tả **chạy được** của microkernel ViFastOs kiểu seL4: capability, untyped memory, IPC đồng bộ,
driver và supervisor ở user space, chạy trên một máy RISC-V giả lập nhiều hart (Sv39, TLB, IOMMU, DMA).
Mục đích: chốt logic và ngữ nghĩa trước, sau đó port sang Rust và dùng bộ test này làm chuẩn so sánh.

```bash
python run_demo.py space      # 2 hart, cấu hình tĩnh, driver IMU qua ngắt, payload bị giới hạn CPU và restart
python run_demo.py desktop    # tạo tiến trình lúc chạy, pager, driver camera DMA qua IOMMU
python tests/test_kernel.py -v   # 27 test
```

Yêu cầu Python 3.11+ (dùng `tomllib`), không cần thư viện ngoài.

## Cấu trúc

| File | Nội dung | Rust |
|---|---|---|
| `ktypes.py`, `abi.py` | Hằng số, mã lỗi, quyền, loại object, syscall, số hiệu syscall | `vifast-abi` (no_std) |
| `objects.py` | Kernel object, Cap, Slot, TrapFrame, bảng object có generation | `vifast-kernel::object` |
| `cspace.py` | Phân giải cptr nhiều cấp có guard, CDT, delete/revoke có preemption, zombie CNode, retype | `vifast-kernel::cap` |
| `ipc.py` | Endpoint, Call/Reply, Notification, chuyển cap | `vifast-kernel::ipc` |
| `vm.py` | Page table Sv39 trong bộ nhớ vật lý, TLB + ASID, shootdown, W^X, IOMMU | logic: `vifast-kernel::vm`; ghi PTE, `sfence.vma`: `vifast-core` |
| `sched.py` | Hàng đợi theo hart, timer, ngân sách CPU (MCS) | `vifast-kernel::sched` |
| `irq.py` | Ngắt thành notification, mask/ack | logic: `vifast-kernel::irq`; PLIC: `vifast-core` |
| `tcb.py` | Cấu hình, resume, suspend, restart, affinity, huỷ | `vifast-kernel::thread` |
| `kernel.py` | Vòng lặp máy nhiều hart, trap, khoá kernel lớn, dispatch, fault | `vifast-kernel::syscall` + `vifast-core::trap` |
| `hw.py` | **Phần cứng giả lập**: RAM vật lý, bus MMIO, TLB, thiết bị DMA | không port (phần cứng thật/QEMU) |
| `boot.py` | Dựng hệ thống từ manifest, profile desktop/space | công cụ build image hoặc root task |
| `images.py` | Chương trình user mẫu | các binary dùng `vifast-user` |

## Những gì được mô hình hoá

| Phần | Cách làm | Test |
|---|---|---|
| Capability, CDT, revoke | Quyền chỉ giảm, badge một lần, revoke thu hồi cả cap đã chuyển qua IPC | `test_cap_transfer_and_revoke` |
| CSpace nhiều cấp | Phân giải cptr 64 bit qua guard + radix từng cấp như seL4 | `test_multilevel_cspace_with_guard` |
| Untyped, không heap kernel | Object có địa chỉ vật lý thật, xoá trắng khi retype | `test_untyped_exhaustion_and_reclaim`, `test_retype_zeroes_memory` |
| Thời gian syscall có giới hạn | Revoke xoá 8 cap mỗi lượt rồi restart syscall | `test_revoke_is_preemptible` |
| Zombie CNode | Dọn 16 slot/tick, phá được vòng CNode tự tham chiếu | `test_zombie_cnode_cycle_is_reclaimed` |
| Sv39 | Page table 3 cấp nằm trong RAM giả lập, user tự cấp PageTable | `test_sv39_page_tables_live_in_physical_memory` |
| Quyền thực thi, W^X | Fetch cần X, cấm map trang vừa W vừa X | `test_wx_and_exec_permission` |
| TLB, ASID, shootdown | TLB từng hart gắn ASID; unmap gửi IPI xoá TLB hart khác. Tắt shootdown thì thấy đọc được dữ liệu cũ | `test_tlb_shootdown_across_harts`, `test_vspace_destroy_frees_asid` |
| SMP | Nhiều hart, hàng đợi riêng, affinity, IPI | `test_two_harts_run_in_parallel` |
| Khoá kernel lớn (BKL) | Mỗi tick chỉ một hart được vào kernel; lệnh máy thường không cần khoá | `test_big_kernel_lock_serializes_syscalls` |
| Timer, preemption | mtimecmp từng hart, hết timeslice thì xoay vòng | `test_timer_round_robin` |
| MCS (budget/period) | Thread hết budget bị throttle đến chu kỳ sau | `test_budget_gives_temporal_isolation` |
| Trap frame, ABI | a7 = số syscall, a0 = mã lỗi, a1 = giá trị, pc += 4 | `test_trap_frame_abi` |
| MMIO | Thanh ghi thiết bị map vào user, đọc/ghi đi thẳng tới thiết bị | `test_desktop_profile` |
| DMA + IOMMU | Có IOMMU: DMA chỉ tới trang đã IoMap. Không có: driver độc hại ghi đè được kernel | `test_iommu_blocks_malicious_dma`, `test_without_iommu_dma_corrupts_kernel` |
| Fault, supervisor, pager | Fault gửi như Call; supervisor restart có giới hạn; pager cấp trang và page table | `test_server_crash_cancels_waiting_client`, `test_desktop_profile` |
| Ngắt | Che sau khi giao, pending gộp lại, mở khi ack | `test_irq_masked_until_ack` |
| Profile tĩnh | Space xoá untyped, IRQ control, sched control sau boot | `test_space_is_static` |

## Quan sát rút ra từ mô phỏng

- **BKL là nút thắt thật:** khi cả hai hart cùng làm syscall liên tục, chỉ một hart vào được kernel mỗi tick.
  Ở bản demo space, nếu IMU phát ngắt mỗi 4 tick thì driver IMU gần như chiếm trọn hart 0 vì
  phải tranh khoá với hart 1. Đó là lý do seL4 giữ đường syscall cực ngắn, và về sau nên thay BKL bằng khoá mịn hơn.
- **Shootdown là bắt buộc:** tắt nó thì hart khác vẫn đọc được trang đã unmap qua TLB cũ.
- **IOMMU là bắt buộc nếu driver ở user space:** MMU không chặn được DMA.

## Bảng chuyển Python sang Rust

| Python | Rust |
|---|---|
| `ObjRef(index, gen)` + `ObjectTable` | Mảng object cố định + generation `u32` |
| `raise KernelError` / `fail()` | `return Err(KError::X)`, lan truyền bằng `?` |
| `BLOCKED`, `PREEMPTED` | `enum Outcome { Done(SysResult), Blocked, Preempted }` |
| Mixin | Nhiều khối `impl Kernel` trong các module |
| `deque` hàng đợi | Danh sách liên kết intrusive trong TCB |
| `PhysMem.read_u64/write_u64` trên PTE | `unsafe` ghi PTE trong `vifast-core`, bọc bằng API an toàn |
| `Tlb.flush` | `sfence.vma` + IPI qua SBI |
| Generator + `yield` | TrapFrame + `ecall` + context switch (asm) |
| `VmFault` | Trap page fault (`scause` 12/13/15) |
| `lock_holder` | Spinlock thật (ticket lock) |
| `hart.mtimecmp` | SBI `set_timer` / CLINT |
| `env` dict | Trang "boot info" map sẵn vào task |
| `hw.py` | Không port: là phần cứng / QEMU |

Quy tắc khi sửa bản Python: không giữ tham chiếu trực tiếp giữa object (chỉ `ObjRef`),
không đệ quy không giới hạn trong kernel, mọi syscall phải kết thúc trong thời gian có giới hạn.

## Giới hạn còn lại (không giả lập có ý nghĩa bằng Python)

- **Thực thi lệnh máy thật:** chương trình user là generator, không phải mã RISC-V. Trap entry,
  lưu thanh ghi, context switch bằng assembly phải kiểm thử trên QEMU.
- **Độ trễ thật:** 1 tick = 1 lệnh; không có chu kỳ CPU, cache, chi phí khoá thật.
- **Đồng thời thật:** các hart chạy tuần tự trong mỗi tick, nên không thể lộ data race mà BKL đang che.
  Khi chuyển sang khoá mịn, cần kiểm chứng bằng `loom` (Rust) hoặc chạy SMP trên QEMU.
- Đơn giản hoá có chủ đích: chưa có superpage, ASID pool cap (ASID cấp tự động), reply object và
  truyền SchedContext qua Call (passive server) của MCS, DMA chiều đọc, IOMMU dùng bảng dict thay cho page table thật.

## Thứ tự port đề xuất

1. `vifast-abi` từ `ktypes.py` + `abi.py`.
2. `vifast-kernel` (`#![forbid(unsafe_code)]`): port `objects`, `cspace`, `sched`, `ipc`, `tcb`, `vm` (phần logic).
   Dùng một trait `Machine` (đọc/ghi u64 vật lý, flush TLB) để chạy được trên host bằng `cargo test`
   với cùng kịch bản như `tests/test_kernel.py`.
3. `vifast-core` (unsafe nhỏ): boot RISC-V trên QEMU, trap, Sv39, context switch, PLIC, SBI timer/IPI.
4. Ghép 2 + 3, chạy manifest `space` trên QEMU với `-smp 2`.
5. Thay BKL bằng khoá mịn, kiểm chứng với `loom`; kiểm chứng phần lõi bằng Kani/Verus.
