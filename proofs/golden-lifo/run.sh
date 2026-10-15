#!/bin/sh
# Does tests/schedule_golden.rs catch a schedule change?
# Copies the tree at e9f73c3 (the 0.3.1 release) to a temp dir, runs the golden test as it is,
# then gives WakeOrdering::Lifo the FIFO key in order_key (src/scope.rs line 136) and runs it again.
set -eu
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$repo" archive e9f73c3 | tar -x -C "$work"
cd "$work"
export CARGO_TARGET_DIR="$work/target"
echo "== as released"
cargo test -q --locked --test schedule_golden 2>&1 | grep -E 'test result' || true
sed -i.orig '136s/u64::MAX - e.runnable_seq/e.runnable_seq/' src/scope.rs
echo "== with the change"
diff src/scope.rs.orig src/scope.rs || true
cargo test -q --locked --test schedule_golden 2>&1 | grep -E 'schedule changed|test result' || true
