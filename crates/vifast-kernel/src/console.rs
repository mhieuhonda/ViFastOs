//! Early console formatting without `alloc`.

use core::fmt::{self, Write};
use core::marker::PhantomData;

use vifast_arch::Arch;

struct Writer<A: Arch>(PhantomData<A>);

impl<A: Arch> Write for Writer<A> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            if byte == b'\n' {
                A::putc(b'\r');
            }
            A::putc(byte);
        }
        Ok(())
    }
}

pub fn log<A: Arch>(args: fmt::Arguments<'_>) {
    let mut writer = Writer::<A>(PhantomData);
    let _ = writer.write_fmt(args);
    A::putc(b'\r');
    A::putc(b'\n');
}

#[macro_export]
macro_rules! klog {
    ($arch:ty, $($arg:tt)*) => {
        $crate::console::log::<$arch>(format_args!($($arg)*))
    };
}
