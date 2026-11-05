# ViFastOs — Báo cáo Agent 2, vòng 20

**Ngày:** 2026-11-05  
**Baseline:** `0decb25` (docs vòng 19) / code `8342553`  
**Code cuối:** `925f633`  
**Người quyết định:** Agent 2 (Agent 0 vắng mặt, tự chọn phạm vi)

## 0. Quyết định phạm vi

Agent 0 vắng mặt nên vòng 20 tự chọn hai mốc có thể kiểm chứng end-to-end:

1. **Per-hart ready queue + work stealing** — mốc SMP kế tiếp sau TLB per-VA;
   host test chứng minh home/steal/affinity move, QEMU chứng minh không hồi quy.
2. **VFS bulk I/O + FS hard link** — sửa luôn điểm yếu vòng 19: demo "20 KiB"
   thực chất chỉ ghi 960 byte vì `MAX_IO=24`; đồng thời thêm hard link và
   persistence readback.

Hardware IOMMU chưa chọn vì block hiện tại là PIO/polling, chưa có DMA để test
end-to-end. Lock mịn/x2APIC để sau khi scheduler tách queue đã ổn định.

## 1. Task → trạng thái → commit

| Task | Trạng thái | Commit | Test |
|---|---|---|---|
| 1. Per-hart runqueue + work stealing | xong | `c37982e` | 1 host test mới; QEMU 50/50 |
| 2. VFS bulk read + hard link | xong | `925f633` | FS 14/14; QEMU; persist phase 7 |
| 3. Docs/soak/bundle | xong | `docs: round 20` | check-docs; harness 336 case âm |

## 2. Task 1 — scheduler per-hart

Thay `runq: [Option<ObjRef>; MAX_TCBS]` bằng:

```text
runq: [[Option<ObjRef>; MAX_TCBS]; MAX_HARTS]
runq_len: [usize; MAX_HARTS]
sched_local/sched_steals: [u64; MAX_HARTS]
```

Quy tắc:

- `enqueue` chọn home queue = hart thấp nhất trong affinity mask còn chỗ.
- `pick_on(hart)` chỉ lấy local queue trước; nếu rỗng mới quét queue hart khác
  và chỉ steal TCB có affinity chứa hart đích.
- `TcbSetAffinity` trên TCB đang queued sẽ remove rồi enqueue lại theo mask mới.
- `remove_from_runq` quét mọi queue; detach/fault/suspend dùng cùng đường.
- Invariant kiểm: độ dài từng queue, slot rỗng sau `len`, không trùng TCB giữa
  hai queue, state Ready/queued, affinity chứa hart sở hữu.
- Stats `scheduler_stats(hart) = (local_picks, steals)` dùng trong host test.

Host test mới kiểm:

- TCB affinity `01`/`10` vào đúng queue; TCB `11` home hart 0.
- `pick_on(1)` steal TCB mobile từ queue hart 0, `steals=1`.
- Đổi affinity TCB đang queued từ hart 1 sang hart 0 di chuyển queue đúng.

Worker E được đặt priority bằng worker C (5), thay vì 1, để timer chắc chắn
quan sát preemption trên AArch64/x86; đây là sửa flake `preempts=0`, không hạ
harness.

## 3. Task 2 — VFS bulk + hard link

### 3.1. Bulk read/write

- `vifast_abi::vfs::MAX_IO` 24 → **128 byte**.
- READ trước đây trả tối đa 3 word (24 byte) trong message data. Nay server đọc
  vào buffer 128 byte, copy vào IPC buffer server; reply copy IPC buffer
  server→client; client copy từ IPC buffer của mình. Một round-trip 128 byte.
- WRITE giữ đường IPC buffer; MAX_IO tăng lên 128.
- Client `FS LARGE` ghi/đọc **20 KiB thật** = 160 chunk × 128 byte, kiểm `STAT`
  và từng byte. Vòng 19 chỉ ghi 40 × 24 = 960 byte; báo cáo vòng 20 sửa lại
  con số cũ.

### 3.2. Hard link

- `vifast-fs::link(target, new_path, uid)`:
  - target phải là file, không phải dir;
  - parent của new path cần quyền write;
  - tăng `nlink`, journal inode + directory entry trong cùng transaction;
  - rollback `nlink` nếu không ghi được directory entry.
- `unlink` sẵn có giảm `nlink`; chỉ free block khi về 0. `fsck` đối chiếu
  `nlink` bằng cách đếm mọi directory entry, nên hard link được kiểm tự động.
- VFS op `LINK=0x200E`, two-path qua IPC buffer như RENAME.
- Client demo: tạo `/hard_a`, ghi 4×128 byte, link `/hard_b`, đọc lại qua b,
  unlink a nhưng đọc b vẫn đúng, unlink b, FSCK, in `HARD LINK OK`.
- `vifast-fsutil` thêm `link <disk> <old> <new>`.
- `test-persist.py` phase 7: put-file 20 KiB, link `/p/large2`, boot guest,
  fsck, host đọc lại cả hai tên khớp.

Host test FS:

- data còn nguyên sau khi xóa một trong hai tên; nlink 2→1→free;
- missing/existing/directory target bị từ chối;
- `fsck` sạch ở mọi bước.

## 4. Số liệu

- `Kernel = 613.720` byte (tăng 664 byte cho 4 ready queue + stats).
- RISC-V headroom vẫn **392.624 byte**, `kernel_end=0x803a0250`.
- TCB thường vẫn 4.656 byte; Object 4.704; Cap 64.
- VFS MAX_IO 128; file tối đa 69 KiB; hard link không giới hạn thêm ngoài
  `nlink: u16` (tối đa 65.535 tên).
- QEMU per-arch 50/50; mutation/panic/handoff giữ nguyên.

## 5. Test cuối

```text
build.sh                          PASS, 0 warning
kernel                            102/102
arch                                6/6
pte                                11/11
elf                                17/17
acpi                                8/8
fs                                 14/14
fsutil                              0/0

test-qemu --repeat 10             10/10 cả 3
riscv64/aarch64/x86_64 --repeat 50 50/50 cả 3
test-skip-tlb-ipi --repeat 5       5/5 RISC + 5/5 x86
test-panic-hart1 --repeat 5        5/5 mỗi kiến trúc
test-boot-handoff --repeat 10     10/10
test-persist.py                    7 phase × 3, hard-link readback
test-harness-spec.py              336 case âm
cargo fmt --check / check-docs    PASS
```

## 6. Nợ còn lại

- Task 9 user IRQ AArch64 vẫn chưa làm.
- Chưa có symlink, block cache, double-indirect, barrier phần cứng.
- Hardware IOMMU, lock mịn/x2APIC, network/input, compositor vẫn backlog.
- Work stealing hiện vẫn dưới BKL; bước tối ưu tiếp theo là khóa per-hart hoặc
  runqueue lock riêng.
