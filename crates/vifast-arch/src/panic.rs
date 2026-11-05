//! Panic ownership shared by all architecture ports.
//!
//! The first hart that panics owns the console and the shutdown sequence.
//! Any later panic is intentionally silent so the single panic record is not
//! interleaved with a second hart's state.

use core::sync::atomic::{AtomicBool, Ordering};

static PANICKING: AtomicBool = AtomicBool::new(false);
static CONSOLE_LOCK: AtomicBool = AtomicBool::new(false);

/// Returns true for exactly one hart: the first one to panic.
pub fn begin() -> bool {
    PANICKING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// True once any hart has entered the panic path.
pub fn is_panicking() -> bool {
    PANICKING.load(Ordering::Acquire)
}

/// Serialize the one panic record with other panic writers.  This lock is
/// independent of the BKL and must only be used by the panic path.
pub fn lock_console() {
    while CONSOLE_LOCK
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

pub fn unlock_console() {
    CONSOLE_LOCK.store(false, Ordering::Release);
}
