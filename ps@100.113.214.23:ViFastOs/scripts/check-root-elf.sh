#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

elf="$1"
target="$2"
machine="$3"
host="$(rustc -vV | sed -n 's/^host: //p')"
objdump="$(rustc --print sysroot)/lib/rustlib/${host}/bin/llvm-objdump"

out="$(cargo run --quiet -p vifast-elf --bin vifast-elf-validate -- \
    "$elf" "$machine" 0x1000200000 0x1000300000)"
printf '%s\n' "$out"

total_memsz=0
while IFS= read -r line; do
    case "$line" in
        *memsz=0x*)
            value="${line#*memsz=}"
            value="${value%% *}"
            total_memsz=$((total_memsz + value))
            ;;
    esac
done <<< "$out"

# A root ELF, its four stack pages and the guard page must fit in the 1 MiB
# root window.  This is deliberately stricter than just the program headers.
if (( total_memsz + 5 * 4096 > 0x100000 )); then
    echo "ROOT ELF WINDOW OVERFLOW: memsz=${total_memsz}" >&2
    exit 1
fi

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

echo "root ELF OK: ${target} size=$(wc -c < "$elf" | tr -d ' ') bytes memsz=${total_memsz}"
