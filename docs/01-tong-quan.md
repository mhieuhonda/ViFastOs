# 01. Tổng quan và mục tiêu

## 1. ViFastOs là gì

ViFastOs là một hệ điều hành viết từ đầu bằng Rust, lõi là một **microkernel dựa trên capability** theo hướng seL4. Kernel chỉ làm 5 việc:

1. Quản lý object và capability (ai được làm gì với cái gì)
2. IPC (chuyển message, báo hiệu)
3. Lập lịch (ai được chạy CPU, bao lâu)
4. Bộ nhớ ảo (page table, TLB)
5. Chuyển ngắt thành thông báo cho driver

Mọi thứ khác (driver, file system, mạng, pager, supervisor, compositor, bộ gõ ViFastKey) chạy ở user space như các tiến trình bình thường, có thêm capability.

## 2. Hai profile, một kernel

| | Desktop | Space |
|---|---|---|
| Mục đích | Máy tính cá nhân, máy trạm | Máy tính bay, hệ thống an toàn sống còn |
| Tạo tiến trình lúc chạy | Có | Không |
| Cắm thiết bị lúc chạy | Có | Không |
| Ngân sách CPU (MCS) | Tuỳ chọn | Nên dùng cho mọi task không quan trọng |
| Cách khoá | Không khoá | Boot xong xoá untyped gốc, IRQ control, sched control |

Điểm then chốt: **kernel không có "chế độ tĩnh"**. Tính tĩnh có được nhờ việc không task nào còn giữ capability để tạo object mới. Kernel chỉ có một bộ code, một bộ test.

## 3. Các quyết định lớn và lý do

| Quyết định | Lựa chọn | Lý do | Cái giá |
|---|---|---|---|
| Kiến trúc kernel | Microkernel | Driver lỗi chỉ chết driver; hợp với Space; kernel nhỏ dễ kiểm chứng | IPC tốn hơn gọi hàm |
| Mô hình quyền | Capability, không có root | Quyền tối thiểu theo thiết kế; thu hồi được | Khó tương thích POSIX |
| Bộ nhớ kernel | Không heap chung; object sinh từ Untyped do user giữ | Kernel không bao giờ hết bộ nhớ vì một task xấu | User space phải tự quản lý bộ nhớ (cần thư viện tốt) |
| Chính sách | Nằm ở user space (supervisor, pager) | Kernel đơn giản, đổi chính sách không đổi kernel | Phải viết thêm dịch vụ user |
| ISA đầu tiên | RISC-V Sv39 trên QEMU | ISA gọn, mở, hợp hướng nghiên cứu phần cứng vũ trụ | Phần cứng thật ít hơn x86/ARM |
| Ngôn ngữ | Rust, lõi `unsafe` tách riêng | An toàn bộ nhớ trong kernel, `unsafe` tập trung một chỗ | Toolchain no_std, một số crate chưa ổn định |
| Phương pháp | Mô hình Python, port Rust, simulator, QEMU, phần cứng | Chốt ngữ nghĩa trước khi đụng phần cứng; test dùng lại được | Tốn công giữ hai bản đồng bộ |
| Đa lõi | Khoá kernel lớn (BKL) trước | Đơn giản, đúng; seL4 cũng dùng | Syscall bị tuần tự hoá giữa các hart |

## 4. Phạm vi hiện tại

**Đã có (cả Python và Rust):** capability + CDT, CSpace nhiều cấp có guard, untyped + retype + xoá trắng, revoke có preemption point, zombie CNode, IPC đồng bộ + notification + chuyển cap, Sv39 trong bộ nhớ vật lý, TLB + ASID + shootdown, W^X, SMP + affinity + IPI, BKL, timer, MCS budget/period, TrapFrame + ABI thanh ghi, MMIO, DMA + IOMMU, fault handler, supervisor, pager, profile desktop/space.

**Chưa có:** mã máy thật (trap entry, context switch bằng assembly), boot trên QEMU, khoá mịn thay BKL, superpage, reply object và truyền SchedContext qua Call của MCS, IOMMU bằng page table thật, file system, mạng, đồ hoạ. Xem `06-lo-trinh.md`.

## 5. Nguyên tắc làm việc

1. **Mô hình trước, phần cứng sau.** Mọi thay đổi ngữ nghĩa kernel làm ở Python (nhanh), có test, rồi mới port.
2. **Test là hợp đồng.** Cùng một kịch bản test chạy ở Python, ở `cargo test` trên host, và sau này trên QEMU.
3. **Cơ chế ở kernel, chính sách ở user.**
4. **Mọi syscall có thời gian giới hạn.** Thao tác dài phải có preemption point.
5. **`unsafe` chỉ ở một crate**, nhỏ, có tài liệu cho từng khối, và được kiểm chứng.
