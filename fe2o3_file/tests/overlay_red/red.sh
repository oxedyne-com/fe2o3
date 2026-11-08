#!/usr/bin/env bash
# SWT4 red claim: with the walk following links (m1_follow.patch), a hostile upper directory over a
# lower symbolic link is walked through the link instead of refused.  Run from the worktree root,
# through rc-build.  Exits 0 when the arm is green on the tree and red on the mutant.
set -o pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)" || exit 2
arm=overlay_refuses_symlinked_parent
cd "$root/fe2o3_file" || exit 2
cargo test --test overlay "$arm" > "$here/base.log" 2>&1 || { echo "base: arm not green"; exit 1; }
git -C "$root" apply "$here/m1_follow.patch" || exit 2
cargo test --test overlay "$arm" > "$here/m1.log" 2>&1
st=$?
git -C "$root" apply -R "$here/m1_follow.patch" || exit 2
if [ $st -eq 0 ]; then echo "m1_follow: arm stayed green (no red)"; exit 1; fi
grep -q 'Expected a refusal' "$here/m1.log" || { echo "m1_follow: failed, but not on the refusal"; exit 1; }
echo "m1_follow: red as claimed"
