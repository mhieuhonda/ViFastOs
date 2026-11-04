#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

target="$1"
machine="$2"
elf="target/${target}/release/vifast-root"
objdump="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objdump"

cargo run --quiet -p vifast-elf --bin vifast-elf-validate -- \
    "$elf" "$machine" 0x1000200000 0x1000300000

"$objdump" -d "$elf" > /tmp/vifast-root-disassembly.txt
sed -E 's/^[[:space:]]*[0-9a-f]+:[[:space:]]+[0-9a-f]+[[:space:]]+//' \
    /tmp/vifast-root-disassembly.txt > /tmp/vifast-root-mnemonics.txt

case "$target" in
    x86_64-*)
        if grep -Eq '%(xmm|ymm|zmm|st[0-9]?)' /tmp/vifast-root-disassembly.txt; then
            echo "ROOT ELF USES FP/SIMD (x86)" >&2
            exit 1
        fi
        ;;
    aarch64-*)
        if grep -Eq '\b[qsd][0-9]+\b|\b(fmov|fadd|fsub|fmul|fdiv)\b' /tmp/vifast-root-mnemonics.txt; then
            echo "ROOT ELF USES FP/SIMD (aarch64)" >&2
            exit 1
        fi
        ;;
    riscv64*)
        if grep -Eq '\b[fd][0-9]+\b|\bv[0-9]+\b' /tmp/vifast-root-mnemonics.txt; then
            echo "ROOT ELF USES FP/SIMD (riscv64)" >&2
            exit 1
        fi
        ;;
esac

echo "root ELF OK: ${target}"
