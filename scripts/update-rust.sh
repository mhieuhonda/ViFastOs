#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Update rustup itself, then the latest stable compiler and the three
# bare-metal targets required by this workspace.
rustup update
rustup component add rustfmt clippy
rustup target add \
  x86_64-unknown-none \
  aarch64-unknown-none \
  riscv64gc-unknown-none-elf

# Rebuild and re-run the host tests after the toolchain change.
./scripts/build.sh
