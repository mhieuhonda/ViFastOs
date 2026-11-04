#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== host portable-kernel tests =="
cargo test -p vifast-kernel
cargo test -p vifast-pte
cargo test -p vifast-elf
cargo test -p vifast-runtime
cargo test -p vifast-user

echo "== root ELF link check (Step 6.3) =="
cargo rustc -p vifast-root --target riscv64imac-unknown-none-elf --release -- \
    -C code-model=medium -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/riscv64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
scripts/check-root-elf.sh riscv64imac-unknown-none-elf 243

cargo rustc -p vifast-root --target aarch64-unknown-none-softfloat --release -- \
    -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/aarch64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
scripts/check-root-elf.sh aarch64-unknown-none-softfloat 183

cargo rustc -p vifast-root --target x86_64-unknown-none --release -- \
    -C relocation-model=pic \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/x86_64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code \
    -C link-arg=-no-pie
scripts/check-root-elf.sh x86_64-unknown-none 62

echo "== riscv64 bare metal =="
cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== aarch64 bare metal =="
cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release

echo "== x86_64 UEFI =="
cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "build complete"
