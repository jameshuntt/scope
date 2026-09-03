#!/usr/bin/env bash
# Full verification sweep for the scope runtime.
#   ./verify.sh            # tests + miri + loom
#   ./verify.sh fast       # tests only
set -euo pipefail
cd "$(dirname "$0")"

echo "== cargo test =="
cargo test

if [ "${1:-}" = "fast" ]; then exit 0; fi

echo
echo "== miri (UB / leak check; skips the wall-clock sleep test) =="
MIRIFLAGS="-Zmiri-disable-isolation" cargo +nightly miri test --lib
MIRIFLAGS="-Zmiri-disable-isolation" cargo +nightly miri test --test kernel -- --skip sleep_task

echo
echo "== loom (cross-thread wake/cancel model check) =="
RUSTFLAGS="--cfg loom" cargo test --test loom_wake --release


echo
echo "ALL VERIFICATION PASSED"
