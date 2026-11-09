#!/usr/bin/env bash
# Chẩn đoán nhanh trạng thái ViFastOs (không build, không chạy QEMU).
# Dùng khi cần biết "máy này đang ở trạng thái nào" trước khi chạy cổng.
set -u
cd "$(dirname "$0")/.." || exit 2
export PATH="$HOME/.cargo/bin:$PATH"

echo "== nguồn =="
echo "HEAD: $(git rev-parse --short HEAD 2>/dev/null) $(git log -1 --pretty=%s 2>/dev/null)"
if [ -n "$(git status --porcelain 2>/dev/null | grep -v '^?? content/')" ]; then
    echo "cây làm việc: CÓ thay đổi chưa commit"
else
    echo "cây làm việc: sạch"
fi

echo "== kích thước module =="
for spec in \
    "riscv64imac-unknown-none-elf:root-task/vifast-client" \
    "riscv64imac-unknown-none-elf:root-task/vifast-root" \
    "aarch64-unknown-none-softfloat:root-task/vifast-client" \
    "x86_64-unknown-none:root-task/vifast-client"; do
    target="${spec%%:*}"; path="${spec#*:}"
    f="target/$target/$path.stripped"
    if [ -f "$f" ]; then
        size=$(wc -c < "$f" | tr -d ' ')
        pages=$(( (size + 4095) / 4096 ))
        printf '  %-42s %7s B  %s trang\n' "$path ($target)" "$size" "$pages"
    else
        printf '  %-42s CHƯA BUILD\n' "$path ($target)"
    fi
done
# Vòng 221: lấy ngân sách từ build.sh (nguồn duy nhất). Bản cũ ghi cứng 6 trong
# khi build.sh đã nâng 8 ⇒ diag báo động sai ("8 > 6") và người đọc mất tin.
budget=$(grep -oE 'check_client_budget riscv64 "[^"]+" [0-9]+' scripts/build.sh | awk '{print $NF}' | head -1)
budget=${budget:-8}
client="target/riscv64imac-unknown-none-elf/root-task/vifast-client.stripped"
if [ -f "$client" ]; then
    n=$(( ($(wc -c < "$client") + 4095) / 4096 ))
    if [ "$n" -gt "$budget" ]; then
        echo "  CẢNH BÁO: client $n trang > ngân sách $budget (build.sh sẽ chặn)"
    else
        echo "  client riscv: $n/$budget trang"
    fi
fi

echo "== headroom kernel riscv =="
NM="$(rustc --print sysroot 2>/dev/null)/lib/rustlib/$(rustc -vV | awk '/host:/{print $2}')/bin/llvm-nm"
for k in target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64 \
         target/riscv64gc-unknown-none-elf/release/vifast-kernel-riscv64-destroy-smp; do
    if [ -f "$k" ] && [ -x "$NM" ]; then
        end=$("$NM" "$k" | awk '$3 == "__kernel_end" { print "0x" $1; exit }')
        if [ -n "$end" ]; then
            hr=$(( 0x80600000 - end ))
            printf '  %-58s headroom=%s B\n' "$(basename "$k")" "$hr"
            if [ "$hr" -lt 196608 ]; then
                echo "  CẢNH BÁO: dưới ngưỡng 192 KiB"
            fi
        fi
    fi
done

echo "== trạng thái cổng gần nhất =="
for f in target/qemu-status-standard.json; do
    [ -f "$f" ] && echo "  $f: $(tr -d '\n' < "$f" | cut -c1-160)"
done
[ -f "$TMPDIR/vifast-par-local.log" ] && echo "  par-gate local log: $TMPDIR/vifast-par-local.log"
[ -f "$TMPDIR/vifast-par-remote.log" ] && echo "  par-gate remote log: $TMPDIR/vifast-par-remote.log"

echo "== arena (từ serial gần nhất của từng arch) =="
for arch in riscv64 aarch64 x86_64; do
    f="${TMPDIR:-/tmp}/vifast-$arch.serial"
    [ -f "$f" ] || f=$(ls -t "${TMPDIR:-/tmp}"/vifast-keep-$arch-*.serial 2>/dev/null | head -1)
    if [ -n "${f:-}" ] && [ -f "$f" ]; then
        line=$(tr -d '\0' < "$f" 2>/dev/null | grep -a "DATA UNTYPED" | tail -1)
        printf '  %-8s %s\n' "$arch" "${line:-<không thấy DATA UNTYPED>}"
        # Vòng 221: cảnh báo khi arena gần cạn — exec/spawn sẽ thất bại mà
        # không nói rõ lý do, rất khó đoán nếu không thấy con số này.
        pair=$(printf '%s' "$line" | awk -F'pages=' 'NF>1 {split($2, a, " "); print a[1]}')
        if [ -n "$pair" ]; then
            used=${pair%%/*}
            total=${pair##*/}
            # Ngưỡng theo SỐ TRANG TRỐNG, không theo %: một lần exec cần ~5
            # trang, nên chỉ cảnh báo khi còn ≤5. Dùng % sẽ kêu ở trạng thái
            # bình thường (79/86 = 91% là chuyện thường sau khi client nạp).
            if [ "$total" -gt 0 ] && [ $(( total - used )) -le 5 ]; then
                echo "           CẢNH BÁO: chỉ còn $(( total - used )) trang arena (exec cần ~5)"
            fi
        fi
    else
        printf '  %-8s <chưa có serial>\n' "$arch"
    fi
done

echo "== tiến trình QEMU đang chạy =="
count=$(ps -Ao comm | grep -c "[q]emu-system" || true)
echo "  qemu-system: ${count:-0}"
