//! Máy RISC-V giả lập để chạy và kiểm thử `vifast-kernel` trên host.
//!
//! - `hw`: RAM vật lý, TLB, thiết bị DMA, cài đặt `Platform`.
//! - `user`: chương trình user viết bằng async, lệnh máy, danh sách image.
//! - `machine`: CPU nhiều hart (MMU + TLB), nguồn ngắt, vòng lặp tick.
//! - `boot`: dựng hệ thống từ manifest TOML (profile desktop/space).
//! - `images`: chương trình mẫu (supervisor, driver, pager...).

pub mod boot;
pub mod hw;
pub mod images;
pub mod machine;
pub mod user;

pub use boot::{build, build_str, load_manifest, parse_manifest, BootError};
pub use images::default_images;
pub use machine::Machine;
pub use user::{program, Images, Program, Sys};

/// Manifest mẫu nhúng sẵn vào binary.
pub const SPACE_MANIFEST: &str = include_str!("../../../manifests/space.toml");
pub const DESKTOP_MANIFEST: &str = include_str!("../../../manifests/desktop.toml");
