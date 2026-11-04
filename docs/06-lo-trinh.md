# 06. Lộ trình

## Giai đoạn 0: Hiện tại
- Mô hình Python (27 test qua)
- Port Rust: abi, kernel (safe, no_std), simulator, 27 test (chưa biên dịch)

**Mục tiêu kế tiếp:** `cargo test -p vifast-sim` xanh, kết quả khớp Python.

## Giai đoạn 1: Củng cố logic trên host
1. Sửa lỗi biên dịch, chạy test.
2. `check_invariants()` + fuzz chuỗi syscall.
3. Thay `alloc` trong kernel bằng cấu trúc intrusive và mảng cố định (mục 3.1 đến 3.3 trong `04-diem-chu-y.md`).
4. Tách syscall thành pha decode/kiểm tra và pha thực thi.
5. Mã hoá syscall qua thanh ghi + IPC buffer; test round-trip encode/decode.

## Giai đoạn 2: `vifast-core` trên QEMU RISC-V (1 hart)
1. Boot: OpenSBI → kernel ở S-mode, stack, `.bss`, UART log.
2. Direct map RAM cho kernel; cài `Platform` (`phys_read/write` qua direct map, `tlb_flush` = `sfence.vma`).
3. Trap entry/exit bằng assembly: lưu/khôi phục TrapFrame, phân loại `scause` (ecall, page fault, timer, ngắt ngoài).
4. Chuyển sang U-mode lần đầu, context switch.
5. Timer qua SBI `set_timer`; PLIC cho ngắt ngoài; VirtIO console làm thiết bị đầu tiên.
6. Nạp chương trình user dạng ELF (root task do kernel nạp; các task khác do root task nạp).
7. `vifast-user`: stub `ecall`, allocator slot/untyped.
8. Chạy manifest `space` trên QEMU với chương trình user thật.

**Mục tiêu:** cùng kịch bản test chạy được ở 3 nơi: Python, host, QEMU.

## Giai đoạn 3: SMP
1. Bật nhiều hart qua SBI HSM, BKL bằng ticket lock.
2. IPI qua SBI, shootdown đồng bộ.
3. Stress test QEMU `-smp 4`.
4. Thay BKL bằng khoá mịn từng phần (runqueue theo hart không khoá; endpoint có khoá riêng), kiểm chứng bằng `loom`.

## Giai đoạn 4: Hai nhánh sản phẩm

### 4a. ViFastOs Space
- WCET cho mọi syscall; hoàn tất preemption point.
- Kiểm tra nhất quán định kỳ, watchdog, ảnh A/B.
- Chạy trên bo RISC-V thật (hoặc FPGA) có ECC.
- Kiểm chứng lõi `unsafe` (Verus/Kani), chứng minh các bất biến capability chính.
- Kết hợp với thí nghiệm dư thừa nhiều máy tính (3 ESP32 hoặc 3 bo RISC-V vote).

### 4b. ViFastOs Desktop
- Dịch vụ: name server, FS (bắt đầu bằng FS trong RAM), driver VirtIO block/net/gpu/input.
- IOMMU thật cho driver DMA.
- Compositor (Rust), input service, **ViFastKey** tích hợp như một dịch vụ.
- Lớp tương thích: chạy Linux trong VM hoặc từng bước tương thích ABI.

## Song song: ViFastOs trên nhân Linux
Nếu cần một sản phẩm dùng được sớm: Linux + compositor Rust (Smithay/COSMIC) + ViFastKey. Kernel riêng là hướng nghiên cứu dài hạn; hai nhánh dùng chung thành phần user space (bộ gõ, ứng dụng) nếu viết tách lõi và adapter.

## Rủi ro chính

| Rủi ro | Giảm thiểu |
|---|---|
| Driver và hệ sinh thái tốn công nhất | Bắt đầu bằng VirtIO trên QEMU; dùng crate có sẵn (`virtio-drivers`, `smoltcp`) |
| SMP sinh race khó tìm | Giữ BKL lâu; `loom`; stress test |
| Hai bản Python/Rust lệch nhau | Test là hợp đồng; sau giai đoạn 1 có thể chỉ giữ Rust + simulator |
| Phạm vi quá rộng | Mỗi giai đoạn có tiêu chí "xong" rõ ràng, không làm giai đoạn sau khi giai đoạn trước chưa xanh |
