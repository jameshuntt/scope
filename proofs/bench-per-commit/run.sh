#!/bin/zsh
# Criterion runs of benches/scheduler.rs at each commit, one after another, in one shared target
# dir, then 19b484b a second time to see how far the numbers move with no code change.
# Each log goes to <new temp dir>/<commit>.log (the second 19b484b run to 19b484b_again.log), so
# the logs the changelog quotes, in logs/, stay as they are. The script prints that dir at the end.
# Compare two logs with: LOGS=<that dir> python3 compare.py 19b484b:e9f73c3
set -u
here=${0:A:h}
repo=$(git -C $here rev-parse --show-toplevel)
L=${LOGS:-$(mktemp -d)}
W=$(mktemp -d)
trap 'rm -rf $W' EXIT
export CARGO_TARGET_DIR=$W/target
mkdir -p $L
for c in 19b484b cfb873a 513cb50 da655cf c4fe22d 967b616 e9f73c3 19b484b; do
  tag=$c
  [ -e $L/$c.log ] && tag=${c}_again
  [ -d $W/$c ] || { mkdir $W/$c; git -C $repo archive $c | tar -x -C $W/$c; }
  cd $W/$c
  find src tests benches -type f -exec touch {} +
  echo "=== $tag start $(date +%T) load $(uptime | sed 's/.*averages: //')" >> $L/progress.txt
  cargo bench --locked --bench scheduler -- --save-baseline $tag --noplot > $L/$tag.log 2>&1
  echo "=== $tag done $(date +%T) exit $?" >> $L/progress.txt
done
echo ALLDONE >> $L/progress.txt
echo "logs in $L"
