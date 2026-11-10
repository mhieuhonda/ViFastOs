//! P0-2 (r289): hộp thư maintenance dùng chung cho cả ba port.
//!
//! Trước đây mỗi port tự cài một giao thức publish/consume, và cả ba đều sai
//! cùng một kiểu: **publish `seq` TRƯỚC payload**, còn consumer **ACK vô điều
//! kiện**. Khe hở: consumer poll đúng giữa hai lệnh publish ⇒ thấy `seq` mới
//! nhưng `kind == 0` ⇒ vẫn ACK ⇒ publisher tưởng đã flush xong ⇒ **mất một lần
//! invalidate TLB**. Hệ quả: entry TLB cũ trỏ vào frame đã trả về Untyped và
//! cấp lại cho tiến trình khác ⇒ đọc/ghi chéo vùng nhớ.
//!
//! Giao thức đúng, cài MỘT lần ở đây:
//!
//! * `publish(kind)`: ghi `kind` (Release) → tăng `seq` (Release) → trả `seq`.
//!   Người đọc thấy `seq` mới thì **bắt buộc** thấy `kind` đi kèm.
//! * `take()`: đọc `seq` (Acquire); nếu `ack >= seq` ⇒ `None`; ngược lại hoán
//!   đổi `kind` về 0. Trả `None` khi `kind == 0` (không có việc thật) và **không**
//!   ACK — nhờ vậy một lần publish không bao giờ bị "ACK suông".
//! * `ack(seq)`: chỉ gọi SAU khi đã làm xong việc của `seq` đó.
//!
//! Crate này `no_std`, không `unsafe`, chỉ dùng `core::sync::atomic` — nên có
//! test host thật (kể cả test hai luồng), đúng thứ mà tầng platform đang thiếu.

#![no_std]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Bit công việc của mailbox (giá trị do từng port quy định).
pub type Kind = u32;

/// Hộp thư maintenance một chiều: publisher ở hart khác, consumer ở hart sở hữu.
pub struct Mailbox {
    seq: AtomicUsize,
    kind: AtomicU32,
    ack: AtomicUsize,
}

impl Mailbox {
    pub const fn new() -> Self {
        Self {
            seq: AtomicUsize::new(0),
            kind: AtomicU32::new(0),
            ack: AtomicUsize::new(0),
        }
    }

    /// Công bố một yêu cầu. Trả `seq` để publisher chờ ACK.
    ///
    /// Thứ tự là điểm cốt lõi: `kind` TRƯỚC, `seq` SAU, cả hai `Release`.
    pub fn publish(&self, kind: Kind) -> usize {
        self.kind.fetch_or(kind, Ordering::Release);
        self.seq.fetch_add(1, Ordering::Release).wrapping_add(1)
    }

    /// Lấy việc ra làm. `None` khi không có việc THẬT (kể cả khi `seq` đã tăng
    /// nhưng `kind` chưa tới) — trường hợp đó KHÔNG được ACK.
    pub fn take(&self) -> Option<(usize, Kind)> {
        let seq = self.seq.load(Ordering::Acquire);
        if seq == 0 || self.ack.load(Ordering::Acquire) >= seq {
            return None;
        }
        let kind = self.kind.swap(0, Ordering::AcqRel);
        if kind == 0 {
            return None;
        }
        Some((seq, kind))
    }

    /// Đánh dấu đã làm xong `seq`.
    pub fn ack(&self, seq: usize) {
        if seq != 0 {
            self.ack.store(seq, Ordering::Release);
        }
    }

    /// Publisher chờ tới khi `seq` được ACK (hoặc hết `spins`).
    pub fn wait_ack(&self, seq: usize, spins: usize) -> bool {
        if seq == 0 {
            return false;
        }
        let mut count = 0usize;
        while count < spins {
            if self.ack.load(Ordering::Acquire) >= seq {
                return true;
            }
            count += 1;
            core::hint::spin_loop();
        }
        self.ack.load(Ordering::Acquire) >= seq
    }

    pub fn acked(&self, seq: usize) -> bool {
        seq != 0 && self.ack.load(Ordering::Acquire) >= seq
    }

    pub fn seq(&self) -> usize {
        self.seq.load(Ordering::Acquire)
    }

    pub fn pending_kind(&self) -> Kind {
        self.kind.load(Ordering::Acquire)
    }
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    const KIND_TLB: Kind = 1;

    #[test]
    fn publish_then_take_yields_the_kind() {
        let mailbox = Mailbox::new();
        let seq = mailbox.publish(KIND_TLB);
        assert_eq!(seq, 1);
        let (taken_seq, kind) = mailbox.take().expect("phải có việc");
        assert_eq!((taken_seq, kind), (1, KIND_TLB));
        mailbox.ack(taken_seq);
        assert!(mailbox.acked(1));
        assert_eq!(mailbox.take(), None, "đã ACK thì không lấy lại");
    }

    /// P0-2 đỏ-trước: `seq` tăng mà `kind` CHƯA tới (đúng trạng thái trung gian
    /// của bản cũ, publish `seq` trước) thì consumer **không được ACK**.
    ///
    /// Với bản cũ, `take()` sẽ ACK luôn `seq` ⇒ yêu cầu bị mất vĩnh viễn.
    #[test]
    fn take_without_kind_must_not_ack() {
        let mailbox = Mailbox::new();
        // Mô phỏng publisher CŨ: seq trước, kind sau.
        mailbox.seq.fetch_add(1, Ordering::Release);
        assert_eq!(mailbox.seq(), 1);
        assert_eq!(mailbox.take(), None, "chưa có kind ⇒ chưa có việc");
        assert!(!mailbox.acked(1), "KHÔNG được ACK khi chưa làm gì");
        // Publisher CŨ viết kind sau cùng:
        mailbox.kind.fetch_or(KIND_TLB, Ordering::Release);
        let (seq, kind) = mailbox.take().expect("việc phải còn nguyên");
        assert_eq!((seq, kind), (1, KIND_TLB));
    }

    /// Hai luồng: publisher bắn N yêu cầu, consumer lấy cho tới khi đủ N.
    /// Không được mất lần nào — đây là bất biến mà giao thức sai đã phá.
    #[test]
    fn concurrent_publish_never_loses_a_flush() {
        const N: usize = 20_000;
        let mailbox = Arc::new(Mailbox::new());
        let taken = Arc::new(AtomicUsize::new(0));
        let worker_mailbox = Arc::clone(&mailbox);
        let worker_taken = Arc::clone(&taken);
        let worker = std::thread::spawn(move || {
            while worker_taken.load(Ordering::Acquire) < N {
                if let Some((seq, kind)) = worker_mailbox.take() {
                    assert_eq!(kind, KIND_TLB);
                    worker_taken.fetch_add(1, Ordering::AcqRel);
                    worker_mailbox.ack(seq);
                }
            }
        });
        let mut published = 0usize;
        while published < N {
            let _ = mailbox.publish(KIND_TLB);
            published += 1;
            // Chờ ACK để mailbox không dồn vô hạn (giống `wait_maintenance`).
            let _ = mailbox.wait_ack(mailbox.seq(), 100_000);
        }
        worker.join().expect("consumer không được panic");
        assert_eq!(
            taken.load(Ordering::Acquire),
            N,
            "mỗi lần publish phải được take đúng một lần"
        );
    }
}
