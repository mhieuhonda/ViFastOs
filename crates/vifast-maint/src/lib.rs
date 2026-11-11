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
//! * `publish(kind, payload)`: ghi payload + `kind` (Release) **trước**, rồi
//!   mới tăng `seq` (Release). Người đọc thấy `seq` mới thì **bắt buộc** thấy
//!   cả payload đi kèm.
//! * `take()`: đọc `seq` (Acquire); nếu `ack >= seq` ⇒ `None`; ngược lại lấy
//!   `kind` + payload. Trả `None` khi `kind == 0` (chưa có việc) và **không**
//!   ACK — nhờ vậy một lần publish không bao giờ bị "ACK suông".
//! * `ack(seq)`: chỉ gọi SAU khi đã làm xong việc của `seq` đó.
//!
//! ## Hợp nhất yêu cầu chồng nhau (289.2)
//!
//! Một hộp thư chỉ có MỘT payload. Nếu publisher thứ hai tới trước khi
//! consumer lấy, ghi đè payload cũ là **mất một lần invalidate** — đúng lỗi
//! cũ ở dạng khác. Vì vậy payload có ba mức, chỉ được **nới rộng**, không bao
//! giờ thu hẹp:
//!
//! | Mức | Nghĩa |
//! |---|---|
//! | `Scope::Free` | chưa có yêu cầu |
//! | `Scope::Range` | xoá một khoảng `[va, va + pages*4KiB)` của một ASID |
//! | `Scope::Asid(asid)` | xoá **toàn bộ** ASID đó |
//! | `Scope::All` | xoá tất cả (an toàn nhất) |
//!
//! Quy tắc hợp nhất: cùng ASID ⇒ nới lên `Asid`; khác ASID ⇒ nới lên `All`.
//! Như vậy hai yêu cầu địa chỉ khác nhau luôn được phủ, đúng yêu cầu 289.2.
//!
//! Crate này `no_std`, không `unsafe`, chỉ dùng `core::sync::atomic` — nên có
//! test host thật (kể cả test hai luồng), đúng thứ mà tầng platform đang thiếu.

#![no_std]
#![forbid(unsafe_code)]

use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Bit công việc của mailbox (giá trị do từng port quy định).
pub type Kind = u32;

/// Phạm vi invalidate mà một yêu cầu bao phủ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Không có yêu cầu nào đang chờ.
    Free,
    /// Xoá `pages` trang từ `va` của đúng `asid` này.
    Range { va: u64, pages: usize, asid: u16 },
    /// Xoá toàn bộ ASID (mọi VA).
    Asid(u16),
    /// Xoá tất cả.
    All,
}

impl Scope {
    /// Hợp nhất hai phạm vi — luôn BAO TRÙM cả hai, không bao giờ hẹp lại.
    pub fn merge(self, other: Scope) -> Scope {
        match (self, other) {
            (Scope::All, _) | (_, Scope::All) => Scope::All,
            (Scope::Free, other) => other,
            (this, Scope::Free) => this,
            (Scope::Asid(a), Scope::Asid(b)) if a == b => Scope::Asid(a),
            (Scope::Asid(_), Scope::Asid(_)) => Scope::All,
            (Scope::Asid(a), Scope::Range { asid, .. })
            | (Scope::Range { asid, .. }, Scope::Asid(a)) => {
                if a == asid {
                    Scope::Asid(a)
                } else {
                    Scope::All
                }
            }
            (Scope::Range { asid: a, .. }, Scope::Range { asid: b, .. }) => {
                if a == b {
                    // Cùng ASID nhưng khác khoảng: nới lên toàn ASID.
                    Scope::Asid(a)
                } else {
                    Scope::All
                }
            }
        }
    }

    /// Có phủ được một khoảng cụ thể không (dùng cho test và cho chẩn đoán).
    pub fn covers(self, asid: u16, va: u64, pages: usize) -> bool {
        match self {
            Scope::Free => false,
            Scope::All => true,
            Scope::Asid(a) => a == asid,
            Scope::Range {
                va: base,
                pages: got,
                asid: a,
            } => {
                a == asid && base <= va && va + (pages as u64) * 4096 <= base + (got as u64) * 4096
            }
        }
    }
}

/// Hộp thư maintenance một chiều: publisher ở hart khác, consumer ở hart sở hữu.
pub struct Mailbox {
    seq: AtomicUsize,
    kind: AtomicU32,
    ack: AtomicUsize,
    /// Công tắc nhỏ cho phần payload: `publish` có thể bị gọi từ nhiều hart,
    /// còn đây là vùng dữ liệu "một người ghi tại một thời điểm".
    lock: AtomicBool,
    scope_kind: AtomicU32,
    va: AtomicU64,
    pages: AtomicUsize,
    asid: AtomicU16,
    /// Chẩn đoán 289.3: số yêu cầu đã publish / đã hợp nhất (không phải thống
    /// kê hiệu năng, chỉ để log chẩn đoán và test).
    published: AtomicUsize,
    merged: AtomicUsize,
}

const SCOPE_FREE: u32 = 0;
const SCOPE_RANGE: u32 = 1;
const SCOPE_ASID: u32 = 2;
const SCOPE_ALL: u32 = 3;

impl Mailbox {
    pub const fn new() -> Self {
        Self {
            seq: AtomicUsize::new(0),
            kind: AtomicU32::new(0),
            ack: AtomicUsize::new(0),
            lock: AtomicBool::new(false),
            scope_kind: AtomicU32::new(SCOPE_FREE),
            va: AtomicU64::new(0),
            pages: AtomicUsize::new(0),
            asid: AtomicU16::new(0),
            published: AtomicUsize::new(0),
            merged: AtomicUsize::new(0),
        }
    }

    fn lock(&self) {
        while self
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    fn unlock(&self) {
        self.lock.store(false, Ordering::Release);
    }

    fn current_scope(&self) -> Scope {
        match self.scope_kind.load(Ordering::Acquire) {
            SCOPE_RANGE => Scope::Range {
                va: self.va.load(Ordering::Relaxed),
                pages: self.pages.load(Ordering::Relaxed),
                asid: self.asid.load(Ordering::Relaxed),
            },
            SCOPE_ASID => Scope::Asid(self.asid.load(Ordering::Relaxed)),
            SCOPE_ALL => Scope::All,
            _ => Scope::Free,
        }
    }

    fn store_scope(&self, scope: Scope) {
        match scope {
            Scope::Free => {
                self.scope_kind.store(SCOPE_FREE, Ordering::Release);
            }
            Scope::Range { va, pages, asid } => {
                self.va.store(va, Ordering::Relaxed);
                self.pages.store(pages, Ordering::Relaxed);
                self.asid.store(asid, Ordering::Relaxed);
                self.scope_kind.store(SCOPE_RANGE, Ordering::Release);
            }
            Scope::Asid(asid) => {
                self.asid.store(asid, Ordering::Relaxed);
                self.scope_kind.store(SCOPE_ASID, Ordering::Release);
            }
            Scope::All => {
                self.scope_kind.store(SCOPE_ALL, Ordering::Release);
            }
        }
    }

    /// Công bố một yêu cầu kèm payload. Trả `seq` để publisher chờ ACK.
    ///
    /// Thứ tự là điểm cốt lõi: payload + `kind` TRƯỚC, `seq` SAU, đều `Release`.
    pub fn publish(&self, kind: Kind, scope: Scope) -> usize {
        self.lock();
        let merged_scope = self.current_scope().merge(scope);
        if self.current_scope() != Scope::Free && merged_scope != scope {
            self.merged.fetch_add(1, Ordering::Relaxed);
        }
        self.store_scope(merged_scope);
        self.kind.fetch_or(kind, Ordering::Release);
        self.published.fetch_add(1, Ordering::Relaxed);
        let seq = self.seq.fetch_add(1, Ordering::Release).wrapping_add(1);
        self.unlock();
        seq
    }

    /// Lấy việc ra làm. `None` khi không có việc THẬT (kể cả khi `seq` đã tăng
    /// nhưng `kind` chưa tới) — trường hợp đó KHÔNG được ACK.
    pub fn take(&self) -> Option<(usize, Kind, Scope)> {
        let seq = self.seq.load(Ordering::Acquire);
        if seq == 0 || self.ack.load(Ordering::Acquire) >= seq {
            return None;
        }
        let kind = self.kind.swap(0, Ordering::AcqRel);
        if kind == 0 {
            return None;
        }
        let scope = self.current_scope();
        // Payload thuộc về `seq` vừa lấy; trả vùng payload về Free để yêu cầu
        // sau không "thừa hưởng" nó.
        self.store_scope(Scope::Free);
        Some((seq, kind, scope))
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

    /// Số yêu cầu đã publish (chẩn đoán 289.3).
    pub fn published(&self) -> usize {
        self.published.load(Ordering::Relaxed)
    }

    /// Số lần một yêu cầu phải NỚI RỘNG phạm vi vì chồng với yêu cầu trước
    /// (chẩn đoán 289.3). Khác 0 nghĩa là đã có hợp nhất thật.
    pub fn merged(&self) -> usize {
        self.merged.load(Ordering::Relaxed)
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
    use std::sync::Arc;

    const KIND_TLB: Kind = 1;

    fn range(va: u64, pages: usize, asid: u16) -> Scope {
        Scope::Range { va, pages, asid }
    }

    #[test]
    fn scope_merge_never_narrows() {
        // Cùng ASID, khác khoảng ⇒ nới lên toàn ASID.
        assert_eq!(
            range(0x1000, 1, 7).merge(range(0x9000, 2, 7)),
            Scope::Asid(7)
        );
        // Khác ASID ⇒ nới lên All.
        assert_eq!(range(0x1000, 1, 7).merge(range(0x1000, 1, 8)), Scope::All);
        // Range + Asid cùng ASID ⇒ Asid.
        assert_eq!(range(0x1000, 1, 7).merge(Scope::Asid(7)), Scope::Asid(7));
        // All nuốt tất cả.
        assert_eq!(Scope::All.merge(range(1, 1, 2)), Scope::All);
        assert_eq!(Scope::Free.merge(range(1, 1, 2)), range(1, 1, 2));
    }

    #[test]
    fn published_payload_reaches_the_consumer() {
        let mailbox = Mailbox::new();
        let seq = mailbox.publish(KIND_TLB, range(0x4000, 3, 5));
        assert_eq!(seq, 1);
        let (taken, kind, scope) = mailbox.take().expect("phải có việc");
        assert_eq!((taken, kind), (1, KIND_TLB));
        assert_eq!(scope, range(0x4000, 3, 5));
        assert!(scope.covers(5, 0x5000, 1), "phải phủ trang giữa khoảng");
        mailbox.ack(1);
        assert!(mailbox.acked(1));
        assert_eq!(mailbox.take(), None);
    }

    /// 289.2 đỏ-trước: HAI publisher ghi hai địa chỉ khác nhau TRƯỚC khi
    /// consumer lấy. Thiết kế cũ (payload nằm ở mảng riêng, ngoài hộp thư) ghi
    /// đè ⇒ mất một lần invalidate. Nay consumer phải nhận phạm vi phủ CẢ HAI.
    #[test]
    fn two_publishers_both_get_covered() {
        let mailbox = Mailbox::new();
        mailbox.publish(KIND_TLB, range(0x1000, 1, 3));
        mailbox.publish(KIND_TLB, range(0x8000, 1, 3));
        assert_eq!(mailbox.merged(), 1, "lần thứ hai phải được hợp nhất");
        let (_, kind, scope) = mailbox.take().expect("gộp thành một yêu cầu");
        assert_eq!(kind, KIND_TLB);
        assert!(scope.covers(3, 0x1000, 1), "phải phủ địa chỉ thứ nhất");
        assert!(scope.covers(3, 0x8000, 1), "phải phủ địa chỉ thứ hai");
        mailbox.ack(1);
    }

    #[test]
    fn publishers_with_different_asids_cover_both() {
        let mailbox = Mailbox::new();
        mailbox.publish(KIND_TLB, range(0x1000, 1, 1));
        mailbox.publish(KIND_TLB, range(0x1000, 1, 2));
        let (_, _, scope) = mailbox.take().expect("phải có việc");
        assert_eq!(scope, Scope::All, "khác ASID ⇒ phải nới lên All");
        assert!(scope.covers(1, 0x1000, 1));
        assert!(scope.covers(2, 0x1000, 1));
    }

    /// 289.3 (r289): mô hình hai hộp thư (hart 0 = BSP, hart 1 = AP). Yêu cầu
    /// từ hart PHỤ gửi cho hart CHÍNH phải tới đúng hộp thư của hart chính và
    /// giữ nguyên payload — tức chiều AP→BSP là biểu diễn được (x86 trước đây
    /// chỉ có hộp thư của AP nên chiều này không tồn tại).
    #[test]
    fn ap_can_publish_to_the_bsp_mailbox() {
        let hart0 = Mailbox::new();
        let hart1 = Mailbox::new();
        let seq = hart0.publish(KIND_TLB, range(0x8000, 2, 0));
        assert_eq!(hart0.pending_kind(), KIND_TLB);
        assert_eq!(hart1.pending_kind(), 0, "không được ghi nhầm hộp thư");
        let (taken, kind, scope) = hart0.take().expect("BSP phải thấy yêu cầu");
        assert_eq!(taken, seq);
        assert_eq!(kind, KIND_TLB);
        assert!(scope.covers(0, 0x8000, 2));
        hart0.ack(taken);
        assert!(hart0.acked(seq));
    }

    #[test]
    fn take_without_kind_must_not_ack() {
        let mailbox = Mailbox::new();
        // Mô phỏng publisher CŨ: seq trước, kind sau.
        mailbox.seq.fetch_add(1, Ordering::Release);
        assert_eq!(mailbox.take(), None, "chưa có kind ⇒ chưa có việc");
        assert!(!mailbox.acked(1), "KHÔNG được ACK khi chưa làm gì");
        mailbox.kind.fetch_or(KIND_TLB, Ordering::Release);
        let (seq, kind, _) = mailbox.take().expect("việc phải còn nguyên");
        assert_eq!((seq, kind), (1, KIND_TLB));
    }

    #[test]
    fn concurrent_publish_never_loses_a_flush() {
        const N: usize = 20_000;
        let mailbox = Arc::new(Mailbox::new());
        let taken = Arc::new(AtomicUsize::new(0));
        let worker_mailbox = Arc::clone(&mailbox);
        let worker_taken = Arc::clone(&taken);
        let worker = std::thread::spawn(move || {
            while worker_taken.load(Ordering::Acquire) < N {
                if let Some((seq, kind, _)) = worker_mailbox.take() {
                    assert_eq!(kind, KIND_TLB);
                    worker_taken.fetch_add(1, Ordering::AcqRel);
                    worker_mailbox.ack(seq);
                }
            }
        });
        let mut published = 0usize;
        while published < N {
            let _ = mailbox.publish(KIND_TLB, range(0x1000, 1, 1));
            published += 1;
            let _ = mailbox.wait_ack(mailbox.seq(), 100_000);
        }
        worker.join().expect("consumer không được panic");
        assert_eq!(taken.load(Ordering::Acquire), N);
        assert_eq!(mailbox.merged(), 0, "không có chồng lấn trong test này");
    }
}
