#!/bin/sh
# Size of a task entry, and of a slab slot holding one, before and after c4fe22d.
# Copies the tree at da655cf and at c4fe22d to a temp dir, adds a test that prints both sizes,
# and runs it. TaskEntry and Slot are private, so this cannot be done from outside the crate.
set -eu
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for c in da655cf c4fe22d; do
  mkdir "$work/$c"
  git -C "$repo" archive "$c" | tar -x -C "$work/$c"
  cat >> "$work/$c/src/ids.rs" <<'RS'

#[cfg(test)]
pub(crate) fn slot_size<T>() -> usize {
    core::mem::size_of::<Slot<T>>()
}
RS
  cat >> "$work/$c/src/scope.rs" <<'RS'

#[cfg(test)]
#[test]
fn print_entry_size() {
    println!(
        "TaskEntry {} bytes, slab slot {} bytes",
        core::mem::size_of::<TaskEntry<'static, ()>>(),
        crate::ids::slot_size::<TaskEntry<'static, ()>>()
    );
}
RS
  printf '%s: ' "$c"
  (cd "$work/$c" && CARGO_TARGET_DIR="$work/target-$c" cargo test -q --locked --lib print_entry_size -- --nocapture 2>&1 | grep 'TaskEntry')
done
