//! Early console formatting without `alloc`.
//!
//! The kernel core does not depend on `vifast-arch`; a tiny `Console` trait
//! keeps logging portable and lets tests use a host sink.

use core::fmt::{self, Write};
use core::marker::PhantomData;

pub trait Console {
    const NAME: &'static str;
    fn write_byte(byte: u8);
}

struct Writer<C: Console>(PhantomData<C>);

/// Spinlock cho console Tx (vòng 105). Static đơn giản vì kernel no_std và
/// `log` phải hoạt động trước cả BKL init.
static TX_LOCK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

impl<C: Console> Write for Writer<C> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            if byte == b'\n' {
                C::write_byte(b'\r');
            }
            C::write_byte(byte);
        }
        Ok(())
    }
}

pub fn log<C: Console>(args: fmt::Arguments<'_>) {
    // Vòng 105 (soak-20 x86 fail 2/20): BSP/AP klog đồng thời làm dính ký
    // tự giữa dòng (`P SSE OFFAP LONG MODE OK`), harness EXACT_ONE fail oan.
    // Spinlock quanh cả dòng để serial nguyên tử theo dòng. Bound spins thay
    // vì spin vô hạn: IRQ handler cũng klog, nếu handler ngắt đúng lúc task
    // cùng hart đang giữ lock thì spin vô hạn = deadlock — quá bound thì ghi
    // trực tiếp (chấp nhận dính ký tự cực hiếm thay vì treo máy).
    let mut spins = 0u32;
    let locked = loop {
        match TX_LOCK.compare_exchange_weak(
            false,
            true,
            core::sync::atomic::Ordering::Acquire,
            core::sync::atomic::Ordering::Relaxed,
        ) {
            Ok(_) => break true,
            Err(_) => {
                spins += 1;
                if spins > 1_000_000 {
                    break false;
                }
                core::hint::spin_loop();
            }
        }
    };
    let mut writer = Writer::<C>(PhantomData);
    let _ = writer.write_fmt(args);
    C::write_byte(b'\r');
    C::write_byte(b'\n');
    if locked {
        TX_LOCK.store(false, core::sync::atomic::Ordering::Release);
    }
}

#[macro_export]
macro_rules! klog {
    ($console:ty, $($arg:tt)*) => {
        $crate::console::log::<$console>(format_args!($($arg)*))
    };
}
