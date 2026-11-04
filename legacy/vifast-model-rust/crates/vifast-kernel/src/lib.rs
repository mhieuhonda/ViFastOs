//! Logic kernel ViFastOs.
//!
//! Crate này là phần "safe" của kernel: `no_std`, `#![forbid(unsafe_code)]`,
//! không biết gì về phần cứng cụ thể. Mọi truy cập phần cứng đi qua trait
//! [`Platform`]. Nhờ vậy cùng một bộ logic chạy được:
//! - trên host (crate `vifast-sim`) để kiểm thử bằng `cargo test`,
//! - trên RISC-V thật/QEMU khi ghép với crate lõi `vifast-core` (unsafe, viết sau).
//!
//! Bản đồ module (tương ứng mô hình Python):
//! | module      | Python      | nội dung |
//! |-------------|-------------|----------|
//! | `object`    | objects.py  | object, cap, slot, bảng object có generation |
//! | `cspace`    | cspace.py   | phân giải cptr, CDT, delete/revoke, zombie, retype |
//! | `ipc`       | ipc.py      | endpoint, call/reply, notification |
//! | `vm`        | vm.py       | Sv39, ánh xạ, shootdown, IOMMU |
//! | `sched`     | sched.py    | hàng đợi theo hart, timer, MCS |
//! | `irq`       | irq.py      | ngắt thành notification |
//! | `tcb`       | tcb.py      | vòng đời thread |
//! | `kernel`    | kernel.py   | struct Kernel, trap, dispatch, fault |

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod cspace;
pub mod ipc;
pub mod irq;
pub mod kernel;
pub mod object;
pub mod platform;
pub mod sched;
pub mod tcb;
pub mod vm;

pub use kernel::{Kernel, KernelConfig, Outcome, Stats};
pub use object::*;
pub use platform::Platform;
pub use tcb::TcbConfig;
pub use vifast_abi as abi;
