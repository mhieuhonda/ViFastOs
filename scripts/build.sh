#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

BUILD_LOG="${VIFAST_BUILD_LOG:-target/vifast-build.log}"
mkdir -p "$(dirname "$BUILD_LOG")"
: > "$BUILD_LOG"
exec > >(tee "$BUILD_LOG") 2>&1

echo "== host portable-kernel tests =="
cargo test -p vifast-kernel
cargo test -p vifast-arch
cargo test -p vifast-pte
cargo test -p vifast-elf
cargo test -p vifast-runtime
cargo test -p vifast-user
python3 scripts/test-harness-spec.py

echo "== root ELF build/validate (Step 6.3) =="
HOST="$(rustc -vV | sed -n 's/^host: //p')"
OBJCOPY="$(rustc --print sysroot)/lib/rustlib/${HOST}/bin/llvm-objcopy"
ROOT_RV=target/riscv64imac-unknown-none-elf/root-task/vifast-root
ROOT_ARM=target/aarch64-unknown-none-softfloat/root-task/vifast-root
ROOT_X86=target/x86_64-unknown-none/root-task/vifast-root
SERVER_RV=target/riscv64imac-unknown-none-elf/root-task/vifast-server
CLIENT_RV=target/riscv64imac-unknown-none-elf/root-task/vifast-client
SERVER_ARM=target/aarch64-unknown-none-softfloat/root-task/vifast-server
CLIENT_ARM=target/aarch64-unknown-none-softfloat/root-task/vifast-client
SERVER_X86=target/x86_64-unknown-none/root-task/vifast-server
CLIENT_X86=target/x86_64-unknown-none/root-task/vifast-client

build_modules() {
    target="$1"
    linker="$2"
    shift 2
    for pkg in vifast-server vifast-client; do
        cargo rustc -p "$pkg" --profile root-task --target "$target" -- \
            "$@" \
            -C link-arg=-T -C link-arg="$linker" \
            -C link-arg=--defsym=ROOT_ELF_BASE=0x1000010000 \
            -C link-arg=-z -C link-arg=max-page-size=4096 \
            -C link-arg=-z -C link-arg=separate-code
    done
}

build_modules riscv64imac-unknown-none-elf crates/vifast-root/linkers/riscv64.ld \
    -C code-model=medium -C relocation-model=static
build_modules aarch64-unknown-none-softfloat crates/vifast-root/linkers/aarch64.ld \
    -C relocation-model=static
build_modules x86_64-unknown-none crates/vifast-root/linkers/x86_64.ld \
    -C relocation-model=pic -C link-arg=-no-pie

for elf in "$SERVER_RV" "$CLIENT_RV" "$SERVER_ARM" "$CLIENT_ARM" "$SERVER_X86" "$CLIENT_X86"; do
    "$OBJCOPY" --strip-all "$elf" "$elf.stripped"
done

cargo rustc -p vifast-root --profile root-task --target riscv64imac-unknown-none-elf -- \
    -C code-model=medium -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/riscv64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_RV" "$ROOT_RV.stripped"
scripts/check-root-elf.sh "$ROOT_RV.stripped" riscv64imac-unknown-none-elf 243

cargo rustc -p vifast-root --profile root-task --target aarch64-unknown-none-softfloat -- \
    -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/aarch64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_ARM" "$ROOT_ARM.stripped"
scripts/check-root-elf.sh "$ROOT_ARM.stripped" aarch64-unknown-none-softfloat 183

cargo rustc -p vifast-root --profile root-task --target x86_64-unknown-none -- \
    -C relocation-model=pic \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/x86_64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code \
    -C link-arg=-no-pie
"$OBJCOPY" --strip-all "$ROOT_X86" "$ROOT_X86.stripped"
scripts/check-root-elf.sh "$ROOT_X86.stripped" x86_64-unknown-none 62

echo "== riscv64 bare metal =="
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release
NM="$(rustc --print sysroot)/lib/rustlib/${HOST}/bin/llvm-nm"
KERNEL_RV=target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64
KERNEL_END="$("$NM" "$KERNEL_RV" | awk '$3 == "__kernel_end" { print "0x" $1; exit }')"
KERNEL_END=$((KERNEL_END))
KERNEL_LIMIT=$((0x80400000))
echo "RISC-V kernel_end=${KERNEL_END} limit=${KERNEL_LIMIT} headroom=$((KERNEL_LIMIT - KERNEL_END))"
if (( KERNEL_END > KERNEL_LIMIT )); then
    echo "RISC-V KERNEL IMAGE EXCEEDS 4 MiB" >&2
    exit 1
fi

echo "== riscv64 boot-handoff test build =="
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-boot-handoff
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-handoff
# Restore the normal artifact for all regular QEMU runs.
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== aarch64 bare metal =="
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release

echo "== x86_64 UEFI =="
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "build complete"
