//! Platform integration for the x86_64 QEMU port.
#![no_std]

pub mod platform;
pub use platform::X86Platform;
pub use vifast_arch_x86_64::*;
