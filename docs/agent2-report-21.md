# ViFastOs — Báo cáo vòng 21

**Ngày:** 2026-11-06  
**Baseline trước vòng:** `a228d3f`  
**Trạng thái:** baseline xanh, 3/3 kiến trúc QEMU PASS, persistence PASS.

## 0. Quyết định phạm vi

Vòng 21 tập trung vào các việc **dễ kiểm chứng, ít rủi ro, mở đường cho driver
và userland**:

1. Chuẩn hóa tên `ViFastOs`.
2. VFS write-through block cache.
3. `fsync` + `BlockFlush` thật cho VirtIO/ATA.
4. VF04 double-indirect.
5. Sleep deadline + BSP idle TCB.
6. `TcbRestart`, `TcbBindNotification`.
7. Console input polling + shell nhỏ.
8. `O_TRUNC`, fsutil utilities, tăng object/data capacity.

CI GitHub đã bỏ theo yêu cầu; không còn `.github/workflows`.

## 1. Commit

| Commit | Nội dung |
|---|---|
| `49de105` | Chuẩn hóa `ViFastOS` → `ViFastOs` |
| `debcf57` | VFS write-through block cache |
| `6e415a7` | Real fsync/BlockFlush cho VirtIO + ATA |
| `b9ed3bc` | VF04 double-indirect |
| `8f7ece9` | Sleep deadline primitive |
| `3e33cfa` | BSP idle TCB + blocking Sleep |
| `26b9c2c` | TcbRestart + TcbBindNotification |
| `bdc85e5` | O_TRUNC + fsutil utilities |
| `5bd47c8` | Console input + shell |
| `af197ea` | Shell mở rộng + tăng object/data capacity |
| `bdc4048`, `3c00dd8`, `302d7cd`, `7340cdf`, `b41fc29`, `a6afcb5`, `6cbc030` | Docs sync |

## 2. Chi tiết

### 2.1. VFS block cache

- `CachedDev<D>` 8 slot × 512 B, write-through.
- `hits/misses/writes`, clear sau format.
- Server dùng `Fs<CachedDev<ServerDev>>`; không đổi ABI.
- Test host: hit, write-through, eviction, bọc FS.

### 2.2. fsync/flush

- `BlockFlush = 45`; cap `BlockDevice` quyền WRITE.
- VirtIO type 4; ATA `0xE7`.
- `Fs::fsync` → `BlockDev::flush`; server FSYNC thật.
- Client gọi FSYNC sau file lớn/hard link/symlink.

### 2.3. VF04 double-indirect

- 10 direct + single-indirect + double-indirect.
- File logic tối đa 16.522 block (~8,4 MiB); ảnh 4 MiB giới hạn thực tế.
- Ảnh VF03 cũ vẫn mount với 11 direct + single (69 KiB).
- Truncate/fsck/free table cấp hai có test host.
- Persistence phase 7 dùng file 80 KiB.

### 2.4. Sleep + idle TCB

- `ThreadState::BlockedSleep`, `Tcb.wake_at`, syscall `Sleep = 46`.
- `tick()` gọi `wake_sleepers()`; deadline hết hạn đưa TCB vào ready queue.
- Root tạo idle TCB hart 0 (`idle_main`, stack `IDLE_STACK_BASE..TOP`,
  priority 0), để scheduler không hết task khi mọi task khác block.
- User wrapper `sleep` block; chỉ fallback poll khi `WouldBlock` (hart phụ).

### 2.5. TCB control

- `TcbRestart = 18`: reset entry/stack, clear pending/fd/wake, trả
  `RestartTcb` để runtime dựng lại context.
- `TcbBindNotification = 21`: bind notification, kiểm tra cap/quyền/waiter.

### 2.6. Console input + shell

- `Arch::getc` cho 16550/PL011/COM1.
- `ConsoleRead = 47` trả byte hoặc `WouldBlock`.
- Client shell: `help`, `time`, `echo`, `ls`, `cat`, `mkdir`, `rm`, `mv`,
  `ln`, `stat`, `sync`, `truncate`, `fsck`.
- QEMU test-exit vẫn tự thoát; shell in `shell> ` nhưng không cần input.

### 2.7. VFS utility

- `O_TRUNC = 8`; server truncate file khi mở.
- `fsutil`: `mkdir`, `rm`, `mv`, `stat`, `truncate`.
- Client dùng O_TRUNC cho file demo.

### 2.8. Capacity

- Shell làm client ELF lớn hơn; tăng Data Untyped 256→288 KiB.
- `DATA_MAX_OBJECTS` 64→72; `MAX_OBJECTS` 136→144.
- RISC-V headroom còn **261.536 byte** (`kernel_end=0x803c0260`).
- `Kernel = 691.288`, `Object = 4.720`, `Tcb = 4.672`, `Cap = 64` byte.

## 3. Test cuối

```text
build.sh                          PASS, 0 warning
vifast-kernel                     104/104
vifast-arch                         6/6
vifast-pte                         11/11
vifast-elf                         17/17
vifast-acpi                          8/8
vifast-fs                           21/21
QEMU --repeat 3                    3/3 cả 3 kiến trúc
test-persist.py                    7 phase ×3 PASS
test-harness-spec.py               348 case âm PASS
cargo fmt --check                  PASS
check-docs.py                      PASS
```

## 4. Nợ còn lại

- Hardware IOMMU (VT-d/SMMU).
- CNode, SchedContext, IRQ external delivery (6 syscall placeholder).
- Network, input IRQ, GPU/compositor, ViFastKey.
- Block cache write-back/mount/pager/barrier phần cứng.
- Khóa mịn/x2APIC, test >2 hart.
- Allocator vật lý tổng quát/free object ngoài revoke.
- Formal verification/CI.
- Userland đầy đủ (libc, shell hệ thống, dynamic linking).

## 5. Ghi chú cho người tiếp nhận

- `Sleep` trên hart phụ có thể fallback poll vì chỉ hart 0 có idle TCB; muốn
  block hoàn toàn thì thêm idle TCB hart 1 hoặc platform idle path.
- Shell hiện polling console, chưa dùng IRQ; muốn shell dùng được lâu dài cần
  input IRQ + line discipline.
- QEMU test mặc định bật `test-exit`; muốn shell tương tác lâu cần build
  không bật feature `test-exit`.
