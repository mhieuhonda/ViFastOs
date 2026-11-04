//! Platform integration for the aarch64 QEMU port.
#![no_std]

pub mod platform;
pub use platform::Aarch64Platform;
pub use vifast_arch_aarch64::*;
