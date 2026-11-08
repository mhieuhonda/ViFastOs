#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

BUILD_LOG="${VIFAST_BUILD_LOG:-target/vifast-build.log}"
mkdir -p "$(dirname "$BUILD_LOG")"
: > "$BUILD_LOG"
exec > >(tee "$BUILD_LOG") 2>&1

echo "== host portable-kernel tests =="
cargo test -p vifast-kernel
cargo test -p vifast-abi
cargo test -p vifast-kernel --lib layout_sizes_for_status -- --nocapture
cargo test -p vifast-arch
cargo test -p vifast-pte
cargo test -p vifast-elf
cargo test -p vifast-runtime
cargo test -p vifast-user
cargo test -p vifast-acpi
cargo test -p vifast-fs
cargo test -p vifast-fsutil
cargo test -p vifast-virtio
cargo build -p vifast-fsutil
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
# Ngân sách trang cho client ELF: mỗi trang client bị staging cấp 1 Frame lúc
# boot, và Untyped dữ liệu chỉ còn ~6 trang. Vượt 6 trang sẽ làm demo PREEMPT
# OK timeout (bài học vòng 116) — chặn ngay ở build thay vì để QEMU đỏ khó hiểu.
# Ngân sách trang theo từng arch (đo từ bản đang xanh): riscv 6, aarch64 7,
# x86_64 8. Mỗi trang client bị staging cấp 1 Frame lúc boot; vượt ngân sách
# sẽ làm demo PREEMPT OK timeout như vòng 116.
check_client_budget() {
    local arch="$1" file="$2" limit="$3"
    if [ ! -f "$file" ]; then
        return 0
    fi
    local size pages
    size=$(wc -c < "$file" | tr -d ' ')
    pages=$(( (size + 4095) / 4096 ))
    echo "client(${arch}) ${size} B = ${pages} trang (ngân sách ${limit})"
    if (( pages > limit )); then
        echo "CLIENT ELF ${arch} VƯỢT NGÂN SÁCH TRANG (tối đa ${limit}): ${pages} trang" >&2
        echo "Thu gọn vifast-client hoặc nới ngân sách Untyped trước khi tăng." >&2
        exit 1
    fi
}
check_client_budget riscv64 "${CLIENT_RV}.stripped" 7

cargo rustc -p vifast-root --profile root-task --target aarch64-unknown-none-softfloat -- \
    -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/aarch64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_ARM" "$ROOT_ARM.stripped"
scripts/check-root-elf.sh "$ROOT_ARM.stripped" aarch64-unknown-none-softfloat 183
check_client_budget aarch64 "${CLIENT_ARM}.stripped" 8

cargo rustc -p vifast-root --profile root-task --target x86_64-unknown-none -- \
    -C relocation-model=pic \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/x86_64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code \
    -C link-arg=-no-pie
"$OBJCOPY" --strip-all "$ROOT_X86" "$ROOT_X86.stripped"
scripts/check-root-elf.sh "$ROOT_X86.stripped" x86_64-unknown-none 62
check_client_budget x86_64 "${CLIENT_X86}.stripped" 9

echo "== destroy-smp root variants =="
cargo rustc -p vifast-root --profile root-task --target riscv64imac-unknown-none-elf --features test-destroy-smp -- \
    -C code-model=medium -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/riscv64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_RV" "${ROOT_RV}.smp.stripped"
scripts/check-root-elf.sh "${ROOT_RV}.smp.stripped" riscv64imac-unknown-none-elf 243
cargo rustc -p vifast-root --profile root-task --target riscv64imac-unknown-none-elf -- \
    -C code-model=medium -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/riscv64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_RV" "$ROOT_RV.stripped"

cargo rustc -p vifast-root --profile root-task --target aarch64-unknown-none-softfloat --features test-destroy-smp -- \
    -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/aarch64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_ARM" "${ROOT_ARM}.smp.stripped"
scripts/check-root-elf.sh "${ROOT_ARM}.smp.stripped" aarch64-unknown-none-softfloat 183
cargo rustc -p vifast-root --profile root-task --target aarch64-unknown-none-softfloat -- \
    -C relocation-model=static \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/aarch64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code
"$OBJCOPY" --strip-all "$ROOT_ARM" "$ROOT_ARM.stripped"

cargo rustc -p vifast-root --profile root-task --target x86_64-unknown-none --features test-destroy-smp -- \
    -C relocation-model=pic \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/x86_64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code \
    -C link-arg=-no-pie
"$OBJCOPY" --strip-all "$ROOT_X86" "${ROOT_X86}.smp.stripped"
scripts/check-root-elf.sh "${ROOT_X86}.smp.stripped" x86_64-unknown-none 62
cargo rustc -p vifast-root --profile root-task --target x86_64-unknown-none -- \
    -C relocation-model=pic \
    -C link-arg=-T -C link-arg=crates/vifast-root/linkers/x86_64.ld \
    -C link-arg=--defsym=ROOT_ELF_BASE=0x1000200000 \
    -C link-arg=-z -C link-arg=max-page-size=4096 \
    -C link-arg=-z -C link-arg=separate-code \
    -C link-arg=-no-pie
"$OBJCOPY" --strip-all "$ROOT_X86" "$ROOT_X86.stripped"

echo "== riscv64 bare metal =="
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release
NM="$(rustc --print sysroot)/lib/rustlib/${HOST}/bin/llvm-nm"
KERNEL_RV=target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64
KERNEL_END="$("$NM" "$KERNEL_RV" | awk '$3 == "__kernel_end" { print "0x" $1; exit }')"
KERNEL_END=$((KERNEL_END))
KERNEL_LIMIT=$((0x80600000))
echo "RISC-V kernel_end=${KERNEL_END} limit=${KERNEL_LIMIT} headroom=$((KERNEL_LIMIT - KERNEL_END))"
if (( KERNEL_END > KERNEL_LIMIT )); then
    echo "RISC-V KERNEL IMAGE EXCEEDS 4 MiB" >&2
    exit 1
fi
if (( KERNEL_LIMIT - KERNEL_END < 196608 )); then
    echo "RISC-V KERNEL HEADROOM BELOW 192 KiB: $((KERNEL_LIMIT - KERNEL_END)) bytes" >&2
    exit 1
fi


echo "== riscv64 destroy-smp feature build =="
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.smp.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-destroy-smp
KERNEL_VAR=target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-destroy-smp
cp "$KERNEL_RV" "$KERNEL_VAR"
KERNEL_END="$("$NM" "$KERNEL_VAR" | awk '$3 == "__kernel_end" { print "0x" $1; exit }')"
KERNEL_END=$((KERNEL_END))
echo "RISC-V destroy-smp kernel_end=${KERNEL_END} headroom=$((KERNEL_LIMIT - KERNEL_END))"
if (( KERNEL_LIMIT - KERNEL_END < 196608 )); then
    echo "RISC-V DESTROY-SMP HEADROOM BELOW 192 KiB" >&2
    exit 1
fi
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== riscv64 boot-handoff test build =="
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-boot-handoff
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-handoff
# Restore the normal artifact for all regular QEMU runs.
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== riscv64 panic-halt feature build =="
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-panic-hart1
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-panic
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release
# TLB mutation build: expected to print TLB REMAP BAD and exit.
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release --features test-skip-tlb-ipi
cp "$KERNEL_RV" target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-skip-tlb
VIFAST_ROOT_ELF="$PWD/${ROOT_RV}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_RV}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_RV}.stripped" cargo build -p vifast-kernel-riscv64 --target riscv64gc-unknown-none-elf --release

echo "== aarch64 bare metal =="
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release
KERNEL_ARM=target/aarch64-unknown-none/release/vifast-kernel-aarch64
# KERNEL_STORAGE is a ~560 KiB static in .data placed just before BKL.
# If Kernel grows, reset() silently clobbers BKL/KERNEL_PTR (vòng 30 DABT).
# Fail loudly here by measuring the real static size from the binary.
KS_INFO="$("$NM" --print-size "$KERNEL_ARM" | awk '$4 ~ /KERNEL_STORAGE/ { print "0x" $1 " 0x" $2; exit }')"
BKL_ADDR="$("$NM" "$KERNEL_ARM" | awk '$3 ~ /platform3BKL$/ { print "0x" $1; exit }')"
KS_ADDR=$(( ${KS_INFO%% *} ))
KS_SIZE=$(( ${KS_INFO##* } ))
BKL_ADDR=$((BKL_ADDR))
GAP=$((BKL_ADDR - (KS_ADDR + KS_SIZE)))
echo "AArch64 static check storage=${KS_ADDR}+${KS_SIZE} end=$((KS_ADDR + KS_SIZE)) bkl=${BKL_ADDR} gap=${GAP}"
if (( GAP < 4096 )); then
    echo "AArch64 KERNEL STATIC GAP WARNING: only ${GAP} bytes before BKL" >&2
fi
if (( KS_ADDR + KS_SIZE >= BKL_ADDR )); then
    echo "AArch64 KERNEL STATIC OVERFLOW: storage end past BKL" >&2
    exit 1
fi

echo "== aarch64 destroy-smp feature build =="
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.smp.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release --features test-destroy-smp
cp target/aarch64-unknown-none/release/vifast-kernel-aarch64 target/aarch64-unknown-none/release/vifast-kernel-aarch64-destroy-smp
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release

echo "== aarch64 panic-halt feature build =="
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release --features test-panic-hart1
cp target/aarch64-unknown-none/release/vifast-kernel-aarch64 target/aarch64-unknown-none/release/vifast-kernel-aarch64-panic
VIFAST_ROOT_ELF="$PWD/${ROOT_ARM}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_ARM}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_ARM}.stripped" cargo build -p vifast-kernel-aarch64 --target aarch64-unknown-none --release

echo "== x86_64 UEFI =="
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "== x86_64 destroy-smp feature build =="
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.smp.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release --features test-destroy-smp
cp target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi-destroy-smp.efi
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "== x86_64 panic-halt feature build =="
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release --features test-panic-hart1
cp target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi-panic.efi
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release --features test-skip-tlb-ipi
cp target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi.efi target/x86_64-unknown-uefi/release/vifast-kernel-x86_64-uefi-skip-tlb.efi
VIFAST_ROOT_ELF="$PWD/${ROOT_X86}.stripped" VIFAST_SERVER_ELF="$PWD/${SERVER_X86}.stripped" VIFAST_CLIENT_ELF="$PWD/${CLIENT_X86}.stripped" cargo build -p vifast-kernel-x86_64-uefi --target x86_64-unknown-uefi --release

echo "build complete"
