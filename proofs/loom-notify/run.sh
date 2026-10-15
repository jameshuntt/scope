#!/bin/sh
# Do the two loom models in src/wake.rs catch a send that skips its notify?
# Copies the tree at e9f73c3 (the 0.3.1 release) to a temp dir, runs the lib tests under
# --cfg loom as they are, then changes `if q.waiting > 0` to `if q.waiting > 1` in
# WakeBus::send (src/wake.rs line 115) and runs them again.
set -eu
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$repo" archive e9f73c3 | tar -x -C "$work"
cd "$work"
export CARGO_TARGET_DIR="$work/target" RUSTFLAGS="--cfg loom"
echo "== as released"
cargo test -q --locked --lib --release 2>&1 | grep -E '^test |test result' || true
sed -i.orig '115s/q.waiting > 0/q.waiting > 1/' src/wake.rs
echo "== with the change"
diff src/wake.rs.orig src/wake.rs || true
cargo test --locked --lib --release 2>&1 | grep -E '^test |deadlock|test result' || true
