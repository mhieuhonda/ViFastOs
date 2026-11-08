#!/bin/sh
# Chạy cổng QEMU song song trên MÁY NÀY và một máy remote qua SSH.
#
#   ./scripts/par-gate.sh                       # remote mặc định: $VIFAST_REMOTE hoặc ps@100.113.214.23
#   ./scripts/par-gate.sh --repeat 3            # số lần lặp mỗi kiến trúc (mặc định 3)
#   ./scripts/par-gate.sh --remote mac@host     # đổi máy remote
#   ./scripts/par-gate.sh --local-only          # chỉ chạy máy này (fallback)
#   ./scripts/par-gate.sh --full                # thêm persist + harness + docs
#                                               # (persist x86_64 chạy ở máy xa)
#
# Cách chia việc: máy này chạy riscv64 + aarch64, máy remote chạy x86_64 —
# mỗi kiến trúc dùng serial + disk + socket riêng nên không tranh chấp.
# Remote phải có sẵn repo tại ~/ViFastOs, rustup + 4 target bare-metal,
# qemu-system-{riscv64,aarch64,x86_64} và OVMF (/usr/share/OVMF/OVMF_CODE.fd).
# Script đồng bộ mã nguồn (rsync, trừ target/) trước khi chạy.
#
# Trả 0 khi cả hai máy xanh. Dùng cho vòng phát triển/soak để giảm thời gian.
set -u

REPEAT=3
REMOTE="${VIFAST_REMOTE:-ps@100.113.214.23}"
REMOTE_DIR="${VIFAST_REMOTE_DIR:-ViFastOs}"
LOCAL_ONLY=0
FULL=0
while [ $# -gt 0 ]; do
    case "$1" in
        --repeat) REPEAT="$2"; shift 2 ;;
        --remote) REMOTE="$2"; shift 2 ;;
        --local-only) LOCAL_ONLY=1; shift ;;
        --full) FULL=1; shift ;;
        *) echo "unknown arg: $1" >&2; exit 2 ;;
    esac
done

cd "$(dirname "$0")/.." || exit 2
PATH="$HOME/.cargo/bin:$PATH"
export PATH
ROOT="$PWD"
TMP="${TMPDIR:-/tmp}"
LOCAL_LOG="$TMP/vifast-par-local.log"
REMOTE_LOG="$TMP/vifast-par-remote.log"

start_local() {
    for arch in riscv64 aarch64; do
        python3 scripts/test-qemu.py --arch "$arch" --repeat "$REPEAT" || return 1
    done
    if [ "$FULL" -eq 1 ]; then
        # Persist + harness chạy ở máy này (không đụng serial của QEMU thường
        # vì test-persist dùng serial/disk riêng).
        python3 scripts/test-persist.py || return 1
        python3 scripts/test-harness-spec.py || return 1
        python3 scripts/gen-docs.py || return 1
        python3 scripts/check-docs.py || return 1
        cargo fmt --all -- --check || return 1
    fi
}

start_remote() {
    # Đồng bộ mã nguồn (không mang target/) rồi build + chạy x86_64 ở xa.
    if ! rsync -a --delete --exclude target --exclude content \
        --exclude '*.serial' "$ROOT/" "$REMOTE:$REMOTE_DIR/" >/dev/null 2>&1; then
        echo "remote sync failed" >&2
        return 1
    fi
    REMOTE_PERSIST=""
    if [ "$FULL" -eq 1 ]; then
        REMOTE_PERSIST=" && PATH=\$HOME/.cargo/bin:\$PATH python3 scripts/test-persist.py"
    fi
    ssh -o BatchMode=yes "$REMOTE" \
        "cd ~/$REMOTE_DIR && PATH=\$HOME/.cargo/bin:\$PATH ./scripts/build.sh >/dev/null 2>&1 && PATH=\$HOME/.cargo/bin:\$PATH python3 scripts/test-qemu.py --arch x86_64 --repeat $REPEAT$REMOTE_PERSIST"
}

echo "=== par-gate: local riscv64+aarch64 ($REPEAT lần) | remote $REMOTE x86_64 ==="
start_local >"$LOCAL_LOG" 2>&1 &
LOCAL_PID=$!
if [ "$LOCAL_ONLY" -eq 1 ]; then
    REMOTE_PID=""
else
    start_remote >"$REMOTE_LOG" 2>&1 &
    REMOTE_PID=$!
fi

wait "$LOCAL_PID"
LOCAL_RC=$?
if [ -n "${REMOTE_PID:-}" ]; then
    wait "$REMOTE_PID"
    REMOTE_RC=$?
else
    REMOTE_RC=0
fi

echo ""
echo "--- local ---"
tail -6 "$LOCAL_LOG"
if [ -n "${REMOTE_PID:-}" ]; then
    echo "--- remote ($REMOTE) ---"
    tail -6 "$REMOTE_LOG"
fi

if [ "$LOCAL_RC" -ne 0 ] || [ "$REMOTE_RC" -ne 0 ]; then
    echo "par-gate: FAIL (local=$LOCAL_RC remote=$REMOTE_RC)"
    exit 1
fi
echo "par-gate: PASS"
