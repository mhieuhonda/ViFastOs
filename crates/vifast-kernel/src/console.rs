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
    let mut writer = Writer::<C>(PhantomData);
    let _ = writer.write_fmt(args);
    C::write_byte(b'\r');
    C::write_byte(b'\n');
}

#[macro_export]
macro_rules! klog {
    ($console:ty, $($arg:tt)*) => {
        $crate::console::log::<$console>(format_args!($($arg)*))
    };
}
