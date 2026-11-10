#!/bin/sh
# Chạy toàn bộ cổng chất lượng của ViFastOs trong một lệnh.
#
#   ./scripts/gate.sh            # build + QEMU 3/3×3 + persist + harness + docs + fmt
#   ./scripts/gate.sh --quick    # bỏ QEMU/Persist (chỉ build + harness + docs + fmt)
#   ./scripts/gate.sh --with-tcp # thêm kiểm tra TCP echo qua hostfwd
#   ./scripts/gate.sh --with-input  # thêm kiểm tra bàn phím virtio
#
# Trả mã 0 khi mọi bước xanh. Dùng cho các vòng phát triển liên tục và cho
# các đợt soak dài: mỗi bước in PASS/FAIL rõ ràng.
set -u

QUICK=0
TCP=0
INPUT=0
for arg in "$@"; do
    case "$arg" in
        --quick) QUICK=1 ;;
        --with-tcp) TCP=1 ;;
        --with-input) INPUT=1 ;;
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
# Host test của kernel chạy tường minh (audit #3 P1-4): bước fuzz chỉ chạy
# --ignored, nên nếu chỉ dựa vào đó thì các test thường không nằm trong cổng.
step "kernel-test" cargo test -p vifast-kernel
# Fuzz chỉ có nghĩa khi đúng 2 test #[ignore] còn tồn tại; đổi tên chúng thì
# "0 passed" vẫn xanh, nên chặn bằng cách đếm số test liệt kê được.
# 289.5: sàn test là số ĐO ĐƯỢC (scripts/test-floor.json), không phải hằng số
# viết tay. Trước đây chốt cứng `-ge 138` trong khi thực tế 245 — xoá 107 test
# vẫn xanh. Nay giảm bất kỳ crate nào là ĐỎ.
step "tests-present" python3 scripts/test-floor.py --check
step "fuzz" cargo test -p vifast-kernel --release -- --ignored

if [ "$QUICK" -eq 0 ]; then
    QEMU_ARGS=""
    if [ "$TCP" -eq 1 ]; then
        QEMU_ARGS="$QEMU_ARGS --tcp-check"
    fi
    if [ "$INPUT" -eq 1 ]; then
        QEMU_ARGS="$QEMU_ARGS --input-check"
    fi
    step "qemu" sh -c "python3 scripts/test-qemu.py --repeat 3 $QEMU_ARGS"
    # r259 (T21/D19): bất biến sched ở ranh giới user — bản xanh phải in
    # BOUNDARY OK, bản mutation (D18) phải in đúng một BOUNDARY BAD.
    step "boundary" sh -c "python3 scripts/test-qemu.py --feature test-check-boundary && python3 scripts/test-qemu.py --feature test-check-boundary-mutation"
    step "persist" python3 scripts/test-persist.py
    step "docs" sh -c 'python3 scripts/gen-docs.py >/dev/null && python3 scripts/check-docs.py'
fi
step "harness" python3 scripts/test-harness-spec.py
step "fmt" cargo fmt --check

printf '\n=== gate summary ===\n'
if [ -n "$FAILED" ]; then
    printf 'FAILED steps:%s\n' "$FAILED"
    exit 1
fi
if [ "$QUICK" -eq 1 ]; then
    printf 'all executed steps passed (--quick: QEMU and persistence were SKIPPED)\n'
    exit 0
fi
printf 'all steps passed\n'
exit 0
