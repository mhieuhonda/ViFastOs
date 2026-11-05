//! Fair ticket big kernel lock groundwork.
//!
//! The lock is global; the next SMP step will replace the single global
//! runqueue, but the lock itself is already FIFO and gives the platform a
//! maintenance-polling hook so a spinning hart can service TLB-shootdown IPIs.

use core::sync::atomic::{AtomicUsize, Ordering};

/// A FIFO ticket lock with an RAII guard.
pub struct BigKernelLock {
    next: AtomicUsize,
    serving: AtomicUsize,
}

/// Guard returned by [`BigKernelLock::lock`] and [`BigKernelLock::try_lock`].
#[must_use = "dropping the guard releases the big kernel lock"]
pub struct BigKernelGuard {
    lock: &'static BigKernelLock,
}

impl BigKernelLock {
    pub const fn new() -> Self {
        Self {
            next: AtomicUsize::new(0),
            serving: AtomicUsize::new(0),
        }
    }

    /// Try to acquire only when no ticket is waiting.
    ///
    /// This is a best-effort helper for single-hart/assert use.  Real kernel
    /// entries use [`BigKernelLock::lock_with`].
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
            Some(BigKernelGuard { lock: self })
        } else {
            None
        }
    }

    /// Acquire the lock and call `poll` while waiting.
    ///
    /// `poll` must service maintenance work (for example an IPI TLB
    /// shootdown) without taking this lock again.
    pub fn lock_with<F: FnMut()>(&'static self, mut poll: F) -> BigKernelGuard {
        let ticket = self.next.fetch_add(1, Ordering::AcqRel);
        while self.serving.load(Ordering::Acquire) != ticket {
            poll();
            core::hint::spin_loop();
        }
        BigKernelGuard { lock: self }
    }

    /// Acquire the lock without a maintenance hook.
    pub fn lock(&'static self) -> BigKernelGuard {
        self.lock_with(|| {})
    }

    /// Whether a ticket is outstanding.
    pub fn is_held(&self) -> bool {
        self.next.load(Ordering::Acquire) != self.serving.load(Ordering::Acquire)
    }
}

impl Default for BigKernelLock {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BigKernelGuard {
    fn drop(&mut self) {
        self.lock.serving.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::BigKernelLock;
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
}
