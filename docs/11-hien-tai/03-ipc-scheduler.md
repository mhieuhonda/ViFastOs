# 11.3. IPC, notification, scheduler và preemption

Nguồn: `crates/vifast-kernel/src/ipc.rs`,
`crates/vifast-kernel/src/lib.rs`,
`crates/vifast-runtime/src/lib.rs`.

## 1. Endpoint

```rust
pub struct Endpoint {
    pub send: Queue,
    pub recv: Queue,
}
```

- `send` chứa TCB đang `BlockedSend`.
- `recv` chứa TCB đang `BlockedRecv`.
- Queue cố định 8 phần tử, FIFO.

## 2. Send

`Kernel::send(sender, endpoint, badge, msg, is_call)`:

1. Nếu message có cap, kiểm tra sender có quyền `GRANT`.
2. Nếu có receiver đang chờ:
   - chuyển cap nếu cần;
   - gán `receiver.resume = Some(message)`;
   - đưa receiver sang `Ready`;
   - nếu là `Call`, đặt sender `BlockedReply` và lưu caller.
3. Nếu không có receiver:
   - lưu `PendingSend` trong sender;
   - đặt sender `BlockedSend`;
   - đẩy sender vào `endpoint.send`.
4. Queue đầy trả `OutOfMemory`.

`Send` không phải Call có thể trả ngay nếu receiver đang chờ. `Call` luôn
block cho đến khi server `Reply`.

## 3. Recv

`Kernel::recv(receiver, endpoint, recv_slot)`:

- Nếu receiver đang có caller chưa Reply: trả `WouldBlock` để tránh overwrite
  caller slot.
- Nếu có sender đang chờ:
  - lấy `PendingSend`;
  - chuyển cap vào đúng `recv_slot` receiver đã chỉ định; slot phải trống,
    không fallback;
  - nếu là Call, ghi caller;
  - trả message.
- Nếu không có sender:
  - đặt `BlockedRecv`;
  - lưu `recv_slot`;
  - đẩy receiver vào `endpoint.recv`.

## 4. Reply

`Kernel::reply(server, msg)`:

- lấy `caller` của server;
- kiểm tra client vẫn `BlockedReply` và đúng `reply_epoch`;
- gán resume message cho client;
- đưa client sang `Ready`.

Server phải Reply trước khi nhận Call tiếp theo.

## 5. Notification

```rust
pub struct Notification {
    pub word: u64,
    pub waiter: Option<ObjRef>,
}
```

- `signal(ntfn, bits)` OR bit vào word; nếu có waiter thì đánh thức và giao
  word qua `resume`.
- `wait_as(tcb, ntfn)` trả word nếu có; nếu không thì block.
- `poll(ntfn)` trả word và clear, không block.
- Signal khi không có waiter sẽ tích luỹ word.

## 6. Chuyển capability qua IPC

`Message.cap` chứa slot nguồn. Khi gửi:

1. `lookup_cap(sender, slot, None, Rights::GRANT)`.
2. Chọn slot đích của receiver:
   - đúng `recv_slot` receiver đã chỉ định;
  - nếu slot 0/đã dùng, trả `InvalidArg`/`SlotOccupied`; không fallback.
3. `copy_cap(sender, src, receiver, dest, rights.without(GRANT), 0)`.
4. Ghi lại `message.cap = dest`.

Capability nhận được không có `GRANT`; do đó không thể chuyển tiếp tùy tiện.
Nếu chuyển cap thất bại, sender được đưa lại queue để không bị mất trạng thái.

## 7. Scheduler

Scheduler hiện tại:

- một runqueue cố định;
- ưu tiên số lớn hơn chạy trước;
- cùng ưu tiên dùng `queue_seq` FIFO;
- `pick()` lấy TCB tốt nhất, đặt `Running`, tăng `context_switches`;
- `yield_current()` đặt current về `Ready` và enqueue lại;
- block state không nằm trong runqueue.

### Chuyển context

Runtime `syscall::switch_blocked`:

1. Lưu context cũ vào `Platform::contexts()[old]`;
2. `kernel.pick()`;
3. nạp context mới;
4. nếu TCB mới có `resume`, ghi message vào register.

### Timer/preemption

`runtime::timer::on_timer`:

1. Lưu context current;
2. `kernel.tick()`;
3. nếu `ticks >= 20`, log `PREEMPT OK` và exit QEMU;
4. reload timer, ack IRQ;
5. `yield_current` + `pick`;
6. nạp context mới + resume.

Timer hiện chỉ chạy trên một hart. Hai hart phụ RISC-V/AArch64 mới idle,
chưa nhận runqueue.

## 8. Trạng thái và lỗi cần nhớ

- Server giữ caller slot chỉ một.
- Reply hai lần trả `NoCaller`.
- Send vào endpoint đầy trả `OutOfMemory`.
- `Yield` syscall chưa thực sự chuyển context.
- Chưa có priority donation, MCS, sleep, suspend/resume.
