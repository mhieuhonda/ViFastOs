//! Big kernel lock groundwork.
//!
//! This is intentionally a single global lock.  It is safe to use before SMP
//! scheduling is enabled; the next SMP step must replace it with a fair ticket
//! lock or a per-CPU design before secondary harts enter the scheduler.

use core::sync::atomic::{AtomicBool, Ordering};

/// A small test-and-set spin lock with an RAII guard.
///
/// The lock is released by `Drop`, including on early returns through the
/// normal trap path.  Paths that never return (shutdown/exit) intentionally
/// leave it held until the machine stops.
pub struct BigKernelLock {
    held: AtomicBool,
}

/// Guard returned by [`BigKernelLock::lock`] and [`BigKernelLock::try_lock`].
#[must_use = "dropping the guard releases the big kernel lock"]
pub struct BigKernelGuard {
    lock: &'static BigKernelLock,
}

impl BigKernelLock {
    pub const fn new() -> Self {
        Self {
            held: AtomicBool::new(false),
        }
    }

    /// Try to acquire without spinning.
    pub fn try_lock(&'static self) -> Option<BigKernelGuard> {
        if self
            .held
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(BigKernelGuard { lock: self })
        } else {
            None
        }
    }

    /// Acquire the lock, spinning until it becomes available.
    pub fn lock(&'static self) -> BigKernelGuard {
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            core::hint::spin_loop();
        }
    }

    /// Whether the lock is currently held.  Intended for assertions/tests.
    pub fn is_held(&self) -> bool {
        self.held.load(Ordering::Acquire)
    }
}

impl Default for BigKernelLock {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BigKernelGuard {
    fn drop(&mut self) {
        debug_assert!(self.lock.held.load(Ordering::Relaxed));
        self.lock.held.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::BigKernelLock;

    static REENTRY_LOCK: BigKernelLock = BigKernelLock::new();
    static RELEASE_LOCK: BigKernelLock = BigKernelLock::new();

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
}
