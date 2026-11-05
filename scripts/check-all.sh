#!/usr/bin/env bash
# Local regression gate: use this instead of hosted CI when GitHub is unavailable.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== formatting =="
cargo fmt --all -- --check

echo "== build + host tests =="
./scripts/build.sh

echo "== QEMU (3 runs per architecture) =="
python3 scripts/test-qemu.py --repeat 3

echo "== persistence =="
python3 scripts/test-persist.py

echo "== harness negative cases =="
python3 scripts/test-harness-spec.py

echo "== docs =="
python3 scripts/check-docs.py

echo "ALL CHECKS PASS"
