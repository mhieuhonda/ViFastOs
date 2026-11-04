# Legacy ViFastOs model

Thư mục này giữ bản mô hình Rust trước đây (workspace `vifast-abi`,
`vifast-kernel`, `vifast-sim`) để tham chiếu. Bản đó:

- chạy trên host với `std`/`toml`;
- có 27 test hành vi và simulator RISC-V;
- dùng `alloc`, chưa phải bare-metal;
- không nằm trong workspace multi-arch ở thư mục gốc.

Bản đang được phát triển là workspace ở thư mục gốc: lõi portable `no_std`,
không heap, boot thật trên QEMU x86_64/OVMF, AArch64 và RISC-V64.
