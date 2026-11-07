#!/bin/sh
# Chạy toàn bộ cổng chất lượng của ViFastOs trong một lệnh.
#
#   ./scripts/gate.sh            # build + QEMU 3/3×3 + persist + harness + docs + fmt
#   ./scripts/gate.sh --quick    # bỏ QEMU/Persist (chỉ build + harness + docs + fmt)
#   ./scripts/gate.sh --with-tcp # thêm kiểm tra TCP echo qua hostfwd
#
# Trả mã 0 khi mọi bước xanh. Dùng cho các vòng phát triển liên tục và cho
# các đợt soak dài: mỗi bước in PASS/FAIL rõ ràng.
set -u

QUICK=0
TCP=0
for arg in "$@"; do
    case "$arg" in
        --quick) QUICK=1 ;;
        --with-tcp) TCP=1 ;;
    esac
done

cd "$(dirname "$0")/.." || exit 2
PATH="$HOME/.cargo/bin:$PATH"
export PATH
FAILED=""

step() {
    name="$1"
    shift
    printf '\n=== %s ===\n' "$name"
    if "$@"; then
        printf '[gate] %s: PASS\n' "$name"
    else
        printf '[gate] %s: FAIL\n' "$name"
        FAILED="$FAILED $name"
    fi
}

step "build" ./scripts/build.sh
if [ "$TCP" -eq 1 ]; then
    step "qemu" sh -c 'python3 scripts/test-qemu.py --repeat 3 --tcp-check'
else
    step "qemu" sh -c 'python3 scripts/test-qemu.py --repeat 3'
fi
if [ "$QUICK" -eq 0 ]; then
    step "persist" python3 scripts/test-persist.py
fi
step "harness" python3 scripts/test-harness-spec.py
step "fmt" cargo fmt --check
step "docs" sh -c 'python3 scripts/gen-docs.py >/dev/null && python3 scripts/check-docs.py'

printf '\n=== gate summary ===\n'
if [ -n "$FAILED" ]; then
    printf 'FAILED steps:%s\n' "$FAILED"
    exit 1
fi
printf 'all steps passed\n'
exit 0
