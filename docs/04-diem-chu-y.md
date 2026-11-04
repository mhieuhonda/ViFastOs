# 04. Toàn bộ điểm cần chú ý

Tài liệu này gom mọi điểm dễ sai, giới hạn và đánh đổi. Nên đọc lại trước mỗi lần sửa kernel.

## 1. Bất biến bảo mật (không được phá)

| # | Bất biến | Cơ chế giữ | Test |
|---|---|---|---|
| S1 | Không ai truy cập object mà không có cap | Mọi syscall đi qua `lookup_cap` | Mọi test |
| S2 | Quyền chỉ giảm khi dẫn xuất | `Cap::derive` lấy giao quyền | `cap_transfer_and_revoke` |
| S3 | Thu hồi được mọi quyền đã cấp | CDT + revoke | `cap_transfer_and_revoke` |
| S4 | Bộ nhớ cấp lại không rò dữ liệu cũ | Xoá trắng khi Retype | `retype_zeroes_memory` |
| S5 | Không ai có cap tới vùng nhớ kernel | Untyped gốc bắt đầu sau `KERNEL_RESERVED` | `iommu_blocks_malicious_dma` |
| S6 | Trang không vừa ghi vừa thực thi | `leaf_flags` W^X | `wx_and_exec_permission` |
| S7 | Unmap có hiệu lực trên mọi hart | Shootdown | `tlb_shootdown_across_harts` |
| S8 | Thiết bị chỉ DMA vào trang được cho phép | IOMMU / IoSpace | `iommu_blocks_malicious_dma` |
| S9 | Không tự nâng ưu tiên | MCP | `mcp_limits_priority` |
| S10 | Một thread không chiếm hết CPU nếu có budget | MCS | `budget_gives_temporal_isolation` |
| S11 | Reply không tới nhầm client sau restart | `reply_epoch` | `server_crash_cancels_waiting_client` |
| S12 | Profile space không tạo được object mới | Xoá cap gốc | `space_is_static` |

Mỗi thay đổi kernel phải chạy lại toàn bộ test này.

## 2. Đúng đắn của kernel

### 2.1. Thời gian syscall có giới hạn
- Mọi vòng lặp trong kernel phải có cận trên phụ thuộc hằng số, không phụ thuộc dữ liệu user. Hiện tại các vòng còn phụ thuộc dữ liệu: duyệt `RunQueue.remove` (O(n) trong hàng), `VecDeque::retain` trong `cancel_ipc`, `find_tcb`. Bản phần cứng cần danh sách liên kết intrusive để gỡ O(1).
- Revoke và reap đã có preemption point. Huỷ Endpoint với hàng đợi rất dài hiện chưa có: cần thêm.

### 2.2. Không đệ quy không giới hạn
Stack kernel nhỏ (thường 8 đến 16 KB). Chuỗi gọi hiện tại: `delete_slot → destroy_object → (tcb_teardown | vspace_teardown...)` có độ sâu cố định, CNode đã chuyển sang zombie. Khi thêm loại object mới, kiểm tra lại chuỗi này.

### 2.3. Thứ tự thao tác khi huỷ
- Gỡ ánh xạ (`unmap_cap`) **trước** khi giảm `refs` và huỷ frame.
- `release_caller` **trước** `cancel_ipc` (để client nhận CANCELLED, không bị treo).
- `finalize_object` sau cùng, sau khi mọi tham chiếu ngược đã được gỡ.

### 2.4. Kiểm tra trước, thay đổi sau
Một syscall lỗi giữa chừng không được để lại trạng thái nửa vời. Hiện có vài chỗ chưa đạt:
- `sys_retype` với `count > 1`: nếu slot thứ k không hợp lệ thì lỗi trước khi tạo gì (tốt), nhưng nếu xoá trắng bộ nhớ thất bại ở object thứ k thì watermark đã tăng.
- `ipc_recv` lấy `pending` của người gửi rồi mới `transfer`; `transfer` không thất bại nên an toàn, nhưng khi thêm lỗi mới phải giữ tính chất này.

Quy tắc: tách mỗi syscall thành pha **decode + kiểm tra** (chỉ đọc) và pha **thực thi** (không còn trả lỗi). seL4 làm đúng như vậy.

### 2.5. `refs` và vòng tham chiếu
`refs` đếm cap. Vòng (CNode chứa cap tới chính nó) không tự giải phóng khi xoá cap ngoài. Cách thu hồi đúng: revoke untyped cha. Nếu sau này cho phép object sinh từ untyped của người khác, cần xem lại.

### 2.6. Generation tràn
`gen: u32` tăng mỗi lần huỷ. Lý thuyết có thể trùng sau 2^32 lần dùng lại cùng chỉ số. Thực tế rất xa, nhưng hệ chạy nhiều năm (vệ tinh) nên dùng `u64` hoặc không bao giờ dùng lại chỉ số đã tràn.

## 3. Đơn giản hoá có chủ đích (phải xử lý trước khi lên phần cứng)

| # | Hiện tại | Phải làm |
|---|---|---|
| 3.1 | Siêu dữ liệu kernel dùng `alloc` (Vec, VecDeque, BTreeMap) | Danh sách intrusive và mảng nằm trong vùng nhớ của object; bảng object cố định. Kernel không còn global allocator |
| 3.2 | CNode là `Vec<Slot>` trên heap | Slot nằm trong RAM của CNode (32 byte mỗi slot) |
| 3.3 | Bản đồ ngược VSpace là BTreeMap | Lưu trong cap (đã có `mapped`) + duyệt page table khi huỷ |
| 3.4 | IOMMU là BTreeMap | Page table IOMMU thật |
| 3.5 | Message truyền như struct Rust | Mã hoá thanh ghi + IPC buffer |
| 3.6 | `DebugPrint` dùng `String` | Truyền con trỏ + độ dài, kernel copy có kiểm tra |
| 3.7 | Shootdown tức thì | IPI + chờ xác nhận |
| 3.8 | Thời gian là tick, 1 tick = 1 lệnh | Timer thật, tính thời gian chạy chính xác cho MCS |
| 3.9 | BKL theo tick | Spinlock thật (ticket lock), sau đó khoá mịn |
| 3.10 | `env` là map | Trang boot info |

## 4. Rust no_std và `unsafe`

- `vifast-kernel` có `#![forbid(unsafe_code)]`. Không được nới.
- Mọi `unsafe` thuộc `vifast-core`, mỗi khối có comment `// SAFETY:` giải thích vì sao đúng.
- `panic = "abort"` cho bản phần cứng. Panic trong kernel = lỗi kernel: ghi log qua UART rồi dừng hart (hoặc reset với watchdog ở profile space).
- Không dùng `unwrap()`/`expect()` trên dữ liệu do user cung cấp. Hiện kernel không dùng `unwrap` ở đường syscall; `Message::new` có `assert!` nhưng chỉ chạy ở phía user.
- Kiểm tra tràn số: phép cộng địa chỉ từ user dùng `checked_add`/`saturating_add`. Shift theo số bit từ user phải chặn `>= 64` (xem `bits_at`).
- Kích thước stack kernel: tránh mảng lớn trên stack. `retype_objects` dùng mảng 4 KB trên stack để xoá trắng: trên phần cứng nên dùng `write_bytes` trực tiếp vào vùng nhớ đích.
- Global allocator (nếu còn dùng tạm): phải có cận và lỗi cấp phát phải thành `OUT_OF_MEMORY`, không được `alloc_error_handler` panic.

## 5. Đa lõi

- BKL che mọi data race trong kernel. Khi gỡ BKL, mọi cấu trúc chia sẻ (bảng object, hàng đợi endpoint, CDT, runqueue của hart khác) cần khoá hoặc thiết kế lock-free. Đây là phần khó nhất của SMP; kiểm chứng bằng `loom` trên host và stress test trên QEMU `-smp 4`.
- Simulator chạy các hart **tuần tự** trong mỗi tick nên không thể phát hiện race thật.
- IPI có độ trễ. Thread được đánh thức trên hart khác không chạy ngay.
- TLB shootdown và IOTLB invalidate phải đồng bộ.

## 6. Hệ thống thời gian thực và space

- Profile space cần tính được **WCET** (thời gian xấu nhất) cho mọi syscall. Điều này đòi hỏi mục 2.1 và 3.1 hoàn tất.
- Bit flip (SEU) có thể làm hỏng cấu trúc kernel. Kernel nên có kiểm tra tính nhất quán định kỳ (checksum bảng object, kiểm tra bất biến CDT) và watchdog phần cứng.
- Cấu hình tĩnh không có nghĩa là an toàn với bức xạ. Cần thêm: ECC + memory scrubbing, dư thừa ở mức hệ thống (nhiều máy tính + vote), ảnh boot A/B.
- `DebugPrint` không được dùng trong bản bay.

## 7. Port Python sang Rust

| Bẫy | Cách xử lý |
|---|---|
| Python giữ tham chiếu object và sửa tại chỗ (`cap.mapped = None`) | Rust: cap là giá trị Copy; sửa qua `set_cap_mapping(slot, ...)` |
| Exception lan qua nhiều tầng | `KResult` + `?`; chỗ bỏ qua lỗi dùng `let _ =` có chú thích |
| `dict` duyệt theo thứ tự chèn | `BTreeMap` duyệt theo khoá: kết quả có thể khác thứ tự (ví dụ địa chỉ object), không khác ngữ nghĩa |
| Generator Python | Async future ở simulator; TrapFrame ở phần cứng |
| Số nguyên Python không tràn | Rust u64 có thể tràn: dùng checked/saturating |
| `x >> 64` = 0 ở Python | Rust panic ở debug: hàm `bits_at` |
| Mượn `self` hai lần | Lấy giá trị cần thiết ra biến (Copy) trước, rồi mới gọi phương thức `&mut self` |

## 8. Trạng thái kiểm chứng của bản Rust

**Bản Rust chưa được biên dịch.** Nó được viết cẩn thận theo đúng logic bản Python (đã chạy, 27/27 test), nhưng môi trường viết không có Rust toolchain. Khả năng cao sẽ có một số lỗi biên dịch nhỏ (mượn, suy luận kiểu, import thừa). Xem `05-port-rust-va-kiem-thu.md` để biết cách xử lý nhanh. Các khu vực rủi ro cao nhất:
1. `vifast-sim/src/user.rs`: future tự viết, `Waker::noop()` (cần Rust 1.85+)
2. `vifast-sim/src/boot.rs`: mượn `machine.kernel` cùng lúc với `machine.boot_env` (hợp lệ vì khác trường)
3. Closure mượn `self.objs` trong khi đang giữ `&mut` tới trường khác (`irq.rs::deliver_irq`)
4. Chuỗi kiểm tra kết quả trong test (định dạng `{:?}` của Vec)

## 9. Ngoài phạm vi kernel nhưng phải nhớ

- **Hệ sinh thái user space quyết định tính dùng được.** Kernel nhỏ nhưng cần: thư viện `vifast-user` (allocator slot, untyped, IPC tiện dụng), runtime async cho dịch vụ, dịch vụ tên (name server), FS, mạng, driver.
- **Tương thích POSIX/Linux** là dự án lớn riêng. Với desktop, cân nhắc chạy Linux trong VM hoặc làm lớp tương thích dần dần.
- **Bộ gõ ViFastKey** trên ViFastOs: là dịch vụ user nhận sự kiện bàn phím từ input service, giao tiếp với compositor qua IPC. Không cần gì đặc biệt từ kernel.
