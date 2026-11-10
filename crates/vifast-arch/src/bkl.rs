//! Fair ticket big kernel lock groundwork.
//!
//! The lock is global; the next SMP step will replace the single global
//! runqueue, but the lock itself is already FIFO and gives the platform a
//! maintenance-polling hook so a spinning hart can service TLB-shootdown IPIs.

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::MAX_HARTS;

/// A FIFO ticket lock with an RAII guard.
pub const NO_OWNER: usize = usize::MAX;

pub struct BigKernelLock {
    next: AtomicUsize,
    serving: AtomicUsize,
    owner: AtomicUsize,
    /// Live-guard count: incremented when a ticket is taken, decremented when
    /// the guard is dropped (after `serving` advances).  `is_held()` is a
    /// single load of this counter, so — unlike the old `next != serving`
    /// two-load comparison — one observation cannot straddle a lock/unlock
    /// transition and report "free" while another hart is entering (issue
    /// #13: that race silently skipped a TLB-shootdown IPI).
    holders: AtomicUsize,
    acquisitions: [AtomicUsize; MAX_HARTS],
    spins: [AtomicUsize; MAX_HARTS],
}

/// Guard returned by [`BigKernelLock::lock`] and [`BigKernelLock::try_lock`].
#[must_use = "dropping the guard releases the big kernel lock"]
pub struct BigKernelGuard {
    lock: &'static BigKernelLock,
    owner: usize,
}

impl BigKernelLock {
    pub const fn new() -> Self {
        Self {
            next: AtomicUsize::new(0),
            serving: AtomicUsize::new(0),
            owner: AtomicUsize::new(NO_OWNER),
            holders: AtomicUsize::new(0),
            acquisitions: [const { AtomicUsize::new(0) }; MAX_HARTS],
            spins: [const { AtomicUsize::new(0) }; MAX_HARTS],
        }
    }

    /// Try to acquire only when no ticket is waiting.
    ///
    /// This is a best-effort helper for single-hart/assert use.  Real kernel
    /// entries use [`BigKernelLock::lock_with`].
    ///
    /// NOTE (issue #13): the returned guard carries `NO_OWNER`, so `held_by`
    /// stays false while the lock is held.  Never use this in production
    /// paths that assert on `held_by`/`assert_bkl_released` — those would be
    /// fooled into thinking the lock is free.
    #[cfg(test)]
    pub fn try_lock(&'static self) -> Option<BigKernelGuard> {
        let ticket = self.next.load(Ordering::Acquire);
        if self.serving.load(Ordering::Acquire) != ticket {
            return None;
        }
        if self
            .next
            .compare_exchange(
                ticket,
                ticket.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            // Same live-holder accounting as `acquire` (no wait here: we
            // only succeeded because no ticket was outstanding).
            self.holders.fetch_add(1, Ordering::AcqRel);
            Some(BigKernelGuard {
                lock: self,
                owner: NO_OWNER,
            })
        } else {
            None
        }
    }

    /// Acquire the lock and call `poll` while waiting.
    ///
    /// `poll` must service maintenance work (for example an IPI TLB
    /// shootdown) without taking this lock again.
    pub fn lock_with<F: FnMut()>(&'static self, poll: F) -> BigKernelGuard {
        self.acquire(NO_OWNER, poll)
    }

    /// Acquire the lock for a specific hart.  The owner is published while
    /// the guard is live, so a hart can assert that it has really released
    /// the lock before returning to user mode.
    pub fn lock_with_hart<F: FnMut()>(&'static self, hart: usize, poll: F) -> BigKernelGuard {
        assert!(hart < MAX_HARTS, "BKL hart out of range");
        assert!(!self.held_by(hart), "BKL re-entry on the same hart");
        self.acquire(hart, poll)
    }

    fn acquire<F: FnMut()>(&'static self, owner: usize, mut poll: F) -> BigKernelGuard {
        let ticket = self.next.fetch_add(1, Ordering::AcqRel);
        // Count as a live holder from ticket-take (not from ticket-serve):
        // any `is_held()` after this point sees the lock held, closing the
        // #13 race where `next != serving` straddled a lock/unlock pair.
        self.holders.fetch_add(1, Ordering::AcqRel);
        while self.serving.load(Ordering::Acquire) != ticket {
            if owner < MAX_HARTS {
                self.spins[owner].fetch_add(1, Ordering::Relaxed);
            }
            poll();
            core::hint::spin_loop();
        }
        if owner < MAX_HARTS {
            assert!(
                self.owner
                    .compare_exchange(NO_OWNER, owner, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok(),
                "BKL owner already set"
            );
            self.acquisitions[owner].fetch_add(1, Ordering::Relaxed);
        }
        BigKernelGuard { lock: self, owner }
    }

    /// Acquire the lock without a maintenance hook.
    pub fn lock(&'static self) -> BigKernelGuard {
        self.lock_with(|| {})
    }

    /// Hart đang giữ vé phục vụ, nếu có. Dùng cho chẩn đoán "kẹt BKL"
    /// (T33a): biết ai giữ để khỏi đoán.
    pub fn owner_hart(&self) -> Option<usize> {
        let owner = self.owner.load(Ordering::Acquire);
        if owner == NO_OWNER {
            None
        } else {
            Some(owner)
        }
    }

    /// Số hart đã lấy vé mà chưa nhả (kể cả đang xếp hàng).
    pub fn holders(&self) -> usize {
        self.holders.load(Ordering::Acquire)
    }

    /// Whether this exact hart currently owns the lock.
    pub fn held_by(&self, hart: usize) -> bool {
        hart < MAX_HARTS && self.owner.load(Ordering::Acquire) == hart
    }

    /// `(acquisitions, spin iterations)` for one hart.
    pub fn stats(&self, hart: usize) -> (usize, usize) {
        if hart >= MAX_HARTS {
            return (0, 0);
        }
        (
            self.acquisitions[hart].load(Ordering::Relaxed),
            self.spins[hart].load(Ordering::Relaxed),
        )
    }

    /// Whether a ticket is outstanding.
    ///
    /// Single atomic load of the live-guard count — no two-counter race
    /// (issue #13).
    pub fn is_held(&self) -> bool {
        self.holders.load(Ordering::Acquire) != 0
    }
}

impl Default for BigKernelLock {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BigKernelGuard {
    fn drop(&mut self) {
        if self.owner < MAX_HARTS {
            let _ = self.lock.owner.compare_exchange(
                self.owner,
                NO_OWNER,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        self.lock.serving.fetch_add(1, Ordering::Release);
        // Release after `serving` advances so a spinner that just observed
        // its ticket as served still sees the lock held until the critical
        // section is fully exited.
        self.lock.holders.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::BigKernelLock;
    /// 289.4 (r289, đỏ-trước): `is_held()` nghĩa là "CÓ ai giữ hoặc đang CHỜ"
    /// — nó đếm từ lúc lấy vé, không phải lúc được phục vụ (đó là cách vá race
    /// #13). Vì vậy `is_held()` KHÔNG được dùng thay cho "hart này đang giữ":
    /// một hart đang xếp hàng vẫn thấy `is_held() == true` nhưng
    /// `held_by(hart)` phải là `false`. Shootdown TLB phải kiểm `held_by`.
    #[test]
    fn held_by_is_owner_only_while_a_waiter_sees_is_held() {
        static LOCK: BigKernelLock = BigKernelLock::new();
        let holder_ready = std::sync::Arc::new(AtomicBool::new(false));
        let release = std::sync::Arc::new(AtomicBool::new(false));
        let ready = std::sync::Arc::clone(&holder_ready);
        let release_holder = std::sync::Arc::clone(&release);
        let holder = std::thread::spawn(move || {
            let _guard = LOCK.lock_with_hart(0, || {});
            ready.store(true, Ordering::SeqCst);
            while !release_holder.load(Ordering::SeqCst) {
                core::hint::spin_loop();
            }
        });
        while !holder_ready.load(Ordering::SeqCst) {
            core::hint::spin_loop();
        }
        let saw_held = AtomicBool::new(false);
        let saw_not_mine = AtomicBool::new(false);
        let guard = LOCK.lock_with_hart(1, || {
            if LOCK.is_held() {
                saw_held.store(true, Ordering::SeqCst);
            }
            if !LOCK.held_by(1) {
                saw_not_mine.store(true, Ordering::SeqCst);
            }
            release.store(true, Ordering::SeqCst);
        });
        holder.join().expect("holder panic");
        drop(guard);
        assert!(
            saw_held.load(Ordering::SeqCst),
            "người xếp hàng phải thấy is_held() == true"
        );
        assert!(
            saw_not_mine.load(Ordering::SeqCst),
            "nhưng held_by(hart đang chờ) phải là false — nếu không shootdown sẽ publish sai"
        );
        assert!(!LOCK.is_held(), "nhả hết thì is_held() phải false");
    }

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    static REENTRY_LOCK: BigKernelLock = BigKernelLock::new();
    static RELEASE_LOCK: BigKernelLock = BigKernelLock::new();
    static FIFO_LOCK: BigKernelLock = BigKernelLock::new();
    static DEADLOCK_LOCK: BigKernelLock = BigKernelLock::new();
    static POLLED: AtomicBool = AtomicBool::new(false);
    static FIFO_INSIDE: AtomicUsize = AtomicUsize::new(0);
    static FIFO_MAX_INSIDE: AtomicUsize = AtomicUsize::new(0);
    static FIFO_DONE: AtomicUsize = AtomicUsize::new(0);
    static HOLD_READY: AtomicBool = AtomicBool::new(false);
    static REVERSE_LOCK: BigKernelLock = BigKernelLock::new();
    static REVERSE_POLLED: AtomicBool = AtomicBool::new(false);
    static REVERSE_HOLD_READY: AtomicBool = AtomicBool::new(false);

    #[test]
    fn rejects_reentry_while_held() {
        let guard = REENTRY_LOCK.try_lock().expect("first acquire");
        assert!(REENTRY_LOCK.is_held());
        assert!(REENTRY_LOCK.try_lock().is_none());
        drop(guard);
        assert!(!REENTRY_LOCK.is_held());
    }

    #[test]
    fn releases_on_guard_drop() {
        {
            let _guard = RELEASE_LOCK.lock();
            assert!(RELEASE_LOCK.is_held());
        }
        assert!(!RELEASE_LOCK.is_held());
        let guard = RELEASE_LOCK.try_lock().expect("acquire after release");
        drop(guard);
    }

    #[test]
    fn ticket_lock_serializes_threads() {
        FIFO_INSIDE.store(0, Ordering::SeqCst);
        FIFO_MAX_INSIDE.store(0, Ordering::SeqCst);
        FIFO_DONE.store(0, Ordering::SeqCst);
        let mut handles = Vec::new();
        for _ in 0..4 {
            handles.push(thread::spawn(|| {
                let guard = FIFO_LOCK.lock();
                let now = FIFO_INSIDE.fetch_add(1, Ordering::SeqCst) + 1;
                FIFO_MAX_INSIDE.fetch_max(now, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(2));
                FIFO_INSIDE.fetch_sub(1, Ordering::SeqCst);
                FIFO_DONE.fetch_add(1, Ordering::SeqCst);
                drop(guard);
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(FIFO_DONE.load(Ordering::SeqCst), 4);
        assert_eq!(FIFO_MAX_INSIDE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn spinning_hart_can_service_maintenance() {
        POLLED.store(false, Ordering::SeqCst);
        HOLD_READY.store(false, Ordering::SeqCst);
        let holder = thread::spawn(|| {
            let guard = DEADLOCK_LOCK.lock();
            HOLD_READY.store(true, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !POLLED.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                core::hint::spin_loop();
            }
            if !POLLED.load(Ordering::SeqCst) {
                drop(guard);
                panic!("spinning hart did not poll maintenance");
            }
            drop(guard);
        });
        let spinner = thread::spawn(|| {
            while !HOLD_READY.load(Ordering::SeqCst) {
                core::hint::spin_loop();
            }
            let guard = DEADLOCK_LOCK.lock_with(|| {
                POLLED.store(true, Ordering::SeqCst);
            });
            drop(guard);
        });
        holder.join().unwrap();
        spinner.join().unwrap();
        assert!(POLLED.load(Ordering::SeqCst));
    }
    #[test]
    fn maintenance_poll_works_in_both_lock_directions() {
        REVERSE_POLLED.store(false, Ordering::SeqCst);
        REVERSE_HOLD_READY.store(false, Ordering::SeqCst);
        let holder = thread::spawn(|| {
            let guard = REVERSE_LOCK.lock();
            REVERSE_HOLD_READY.store(true, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !REVERSE_POLLED.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                core::hint::spin_loop();
            }
            if !REVERSE_POLLED.load(Ordering::SeqCst) {
                drop(guard);
                panic!("reverse spinner did not poll maintenance");
            }
            drop(guard);
        });
        let spinner = thread::spawn(|| {
            while !REVERSE_HOLD_READY.load(Ordering::SeqCst) {
                core::hint::spin_loop();
            }
            let guard = REVERSE_LOCK.lock_with(|| {
                REVERSE_POLLED.store(true, Ordering::SeqCst);
            });
            drop(guard);
        });
        holder.join().unwrap();
        spinner.join().unwrap();
        assert!(REVERSE_POLLED.load(Ordering::SeqCst));
    }

    #[test]
    fn tracks_owner_per_hart_with_a_waiter() {
        static OWNER_LOCK: BigKernelLock = BigKernelLock::new();
        static FIRST_HELD: AtomicBool = AtomicBool::new(false);
        static SECOND_DONE: AtomicBool = AtomicBool::new(false);
        let first = thread::spawn(|| {
            let guard = OWNER_LOCK.lock_with_hart(0, || {});
            assert!(OWNER_LOCK.held_by(0));
            assert!(!OWNER_LOCK.held_by(1));
            FIRST_HELD.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(20));
            drop(guard);
            assert!(!OWNER_LOCK.held_by(0));
        });
        let second = thread::spawn(|| {
            while !FIRST_HELD.load(Ordering::SeqCst) {
                core::hint::spin_loop();
            }
            let guard = OWNER_LOCK.lock_with_hart(1, || {});
            assert!(OWNER_LOCK.held_by(1));
            assert!(!OWNER_LOCK.held_by(0));
            drop(guard);
            assert!(!OWNER_LOCK.held_by(1));
            SECOND_DONE.store(true, Ordering::SeqCst);
        });
        first.join().unwrap();
        second.join().unwrap();
        assert!(SECOND_DONE.load(Ordering::SeqCst));
        assert!(OWNER_LOCK.stats(0).0 >= 1);
        assert!(OWNER_LOCK.stats(1).0 >= 1);
    }
}
