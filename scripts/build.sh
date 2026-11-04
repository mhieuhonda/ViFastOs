#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== host portable-kernel tests =="
cargo test -p vifast-kernel
cargo test -p vifast-runtime
cargo test -p vifast-user

echo "== riscv64 bare metal =="
cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== aarch64 bare metal =="
cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release

echo "== x86_64 UEFI =="
cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "build complete"
