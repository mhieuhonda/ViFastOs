//! Platform integration for the riscv64 QEMU port.
#![no_std]

pub mod platform;
pub use platform::RiscvPlatform;
pub use vifast_arch_riscv64::*;
