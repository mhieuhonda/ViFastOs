# 02. Kiến trúc tổng thể

## 1. Các tầng

```
┌──────────────────────────────────────────────────────────────┐
│ Ứng dụng: app native, lớp tương thích (sau này)              │
├──────────────────────────────────────────────────────────────┤
│ Dịch vụ hệ thống (user space, mỗi cái một tiến trình):       │
│ supervisor, pager, driver, FS, net, compositor, ViFastKey    │
├──────────────────────────────────────────────────────────────┤
│ vifast-user: thư viện syscall, cấp phát slot, quản lý bộ nhớ │
════════════════════ ranh giới user/kernel (ecall) ══════════════
│ vifast-kernel (safe Rust, no_std)                            │
│  object · cspace · ipc · sched · vm · irq · tcb · kernel     │
├──────────────────────────────────────────────────────────────┤
│ trait Platform                                               │
├───────────────────────────┬──────────────────────────────────┤
│ vifast-core (unsafe, sau) │ vifast-sim (host, đã có)         │
│ boot, trap, Sv39, PLIC,   │ RAM giả, TLB giả, thiết bị giả,  │
│ SBI timer/IPI, ctx switch │ CPU chạy chương trình async      │
├───────────────────────────┴──────────────────────────────────┤
│ Phần cứng RISC-V / QEMU virt                                 │
└──────────────────────────────────────────────────────────────┘
```

## 2. Các crate

| Crate | `no_std` | `unsafe` | Phụ thuộc | Vai trò |
|---|---|---|---|---|
| `vifast-abi` | Có | Cấm | `alloc` | Hằng số, mã lỗi, quyền, loại object, Message, SysResult, enum Syscall |
| `vifast-kernel` | Có | Cấm | `vifast-abi`, `alloc` | Toàn bộ logic kernel. Không biết phần cứng, chỉ gọi `Platform` |
| `vifast-sim` | Không | Không dùng | `vifast-kernel`, `toml` | Máy giả lập để chạy kernel trên host; boot từ manifest; 27 test |
| `vifast-core` (chưa có) | Có | Cho phép, tối thiểu | `vifast-kernel`, `riscv` | Cài `Platform` trên phần cứng; trap, context switch |
| `vifast-user` (chưa có) | Có | Tối thiểu | `vifast-abi` | Thư viện cho chương trình user: stub `ecall`, allocator slot/untyped |

Quy tắc phụ thuộc: `vifast-kernel` **không bao giờ** phụ thuộc `vifast-core` hay `vifast-sim`. Chiều phụ thuộc chỉ đi từ ngoài vào trong.

## 3. Ranh giới `Platform`

```rust
pub trait Platform {
    fn phys_read(&self, pa: u64, buf: &mut [u8]) -> bool;
    fn phys_write(&mut self, pa: u64, data: &[u8]) -> bool;
    fn tlb_flush(&mut self, hart: usize, asid: Option<u32>, vpn: Option<u64>);
    fn log(&mut self, tick: u64, source: &str, text: &str);
}
```

Đây là **toàn bộ** những gì kernel cần từ phần cứng ở mức logic. Những việc khác của phần cứng (trap entry, lưu thanh ghi, chuyển chế độ, đọc PLIC) do bên thực thi (`vifast-core` hoặc `vifast-sim`) làm rồi gọi vào các API của `Kernel`:

| Sự kiện phần cứng | API kernel được gọi |
|---|---|
| Đầu mỗi tick timer | `tick_start()`, sau đó `tick_housekeeping()` |
| Ngắt ngoài từ PLIC | `deliver_irq(irq)` |
| Hart cần thread để chạy | `schedule(h)` |
| `ecall` từ user | `syscall(h, t, &sc)` |
| Page fault | `vm_fault(h, t, addr)` |
| Lệnh không hợp lệ, thread kết thúc | `user_fault(h, t, kind, detail)` |
| Sau khi thread chạy một lượt | `after_run(h, t)` |
| Thiết bị muốn DMA | `dma_plan(iospace, ...)` (IOMMU giả lập) |

## 4. Nguyên tắc thiết kế

### 4.1. Object chỉ tham chiếu nhau qua `ObjRef`
`ObjRef { index, gen }` trỏ vào một bảng object. Huỷ object thì `gen` tăng, mọi `ObjRef` cũ tự vô hiệu. Không có con trỏ Rust giữa các object, không có `Rc<RefCell<>>`, không lifetime chồng chéo. Đây là kỹ thuật "generational arena", phổ biến trong game engine và hợp với kernel.

### 4.2. Kernel không có heap chung
Mọi object (TCB, endpoint, CNode, page table...) được tạo bằng `Retype` từ một vùng **Untyped** mà user space đang giữ. Hết untyped thì chỉ task đó nhận `OUT_OF_MEMORY`. Ở bản hiện tại, siêu dữ liệu (hàng đợi, bảng tra) vẫn dùng `alloc`; xem `04-diem-chu-y.md` mục 3.1.

### 4.3. Cơ chế ở kernel, chính sách ở user
- Kernel gửi fault như một message. Supervisor quyết định restart, giới hạn số lần, DEGRADED hay dừng hết.
- Kernel báo page fault. Pager quyết định cấp trang nào.
- Kernel cấp ngân sách CPU khi có quyền SchedControl. Ai được bao nhiêu là chính sách của task giữ quyền đó.

### 4.4. Mọi thao tác kernel có thời gian giới hạn
- Revoke xoá tối đa 8 cap mỗi lượt rồi trả `Preempted`.
- CNode zombie dọn tối đa 16 slot mỗi tick.
- Không đệ quy không giới hạn: xoá CNode không xoá đệ quy mà đưa vào hàng đợi.

### 4.5. Lỗi phải trả về, không panic
Mọi đường xử lý syscall trả `KResult<T>`. Lỗi của user không bao giờ làm kernel panic. `panic!` trong kernel chỉ được phép cho bất biến nội bộ bị phá (lỗi lập trình kernel).

### 4.6. Một logic, nhiều cách đóng gói
Driver, FS, giao thức nên viết thành crate `no_std` độc lập với cách chạy. Khi cần tốc độ có thể đặt vào cùng không gian với dịch vụ gọi nó; khi cần cô lập thì là tiến trình riêng.

## 5. Luồng điển hình

### 5.1. Client gọi server (Call/Reply)
```
client: Call(ep, msg) ──trap──► kernel: lookup cap ep (W)
                                 server đang chờ Recv? ─có─► chuyển msg, server READY,
                                                             client BLOCKED_REPLY, server.caller = client
server: xử lý, ReplyRecv(ep, reply) ─► kernel: kiểm tra caller + reply_epoch còn hợp lệ
                                        chuyển reply, client READY; server chờ Recv tiếp
```

### 5.2. Ngắt tới driver
```
thiết bị ─IRQ─► kernel.deliver_irq: che IRQ, signal notification
driver (đang Recv với bound notification) ─► nhận LABEL_NOTIFICATION
driver đọc thanh ghi MMIO (map sẵn), xử lý, IrqAck ─► kernel bỏ che, giao pending nếu có
```

### 5.3. Page fault được pager xử lý
```
lazy: Store(0x4000_0008) ─MMU─► không có PTE ─► kernel.vm_fault
kernel: gửi fault như Call tới fault endpoint (pager), lazy BLOCKED_REPLY
pager: Retype Frame (+ PageTable nếu thiếu), FrameMap vào VSpace của lazy, Reply
kernel: lazy READY; CPU chạy lại đúng lệnh Store (PC chưa tăng)
```

### 5.4. Task lỗi được supervisor restart
```
payload panic ─► kernel.raise_fault(USER_EXCEPTION) ─► Call tới fault_ep, badge = slot TCB
supervisor: Recv, đếm số lần trong cửa sổ, TcbRestart(slot) hoặc ghi DEGRADED
kernel: release_caller (client đang chờ nhận CANCELLED), reset_context, make_ready
```
