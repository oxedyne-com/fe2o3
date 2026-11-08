#!/usr/bin/env bash
# QA SWT4b red claims.  Run through rc-build; exits 0 only when every claim holds.
#  F1: overlay_deep_chain_refused_at_path_max, a 200,000-deep chain walked under a 2 GiB address
#      cap, aborts with 7affe96b's overlay.rs and returns one refusal at depth 2,049 now.
#  F2: overlay_refused_dir_keeps_no_other_op, a directory over a lower file refused for want of a
#      descriptor, keeps its delete and create beside the refusal at 7affe96b, and not now.
set -o pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)" || exit 2
base=7affe96b
arms="overlay_deep_chain_refused_at_path_max overlay_refused_dir_keeps_no_other_op"
cd "$root/fe2o3_file" || exit 2
trap 'git -C "$root" checkout HEAD -- fe2o3_file/src/overlay.rs' EXIT
fail=0

cargo test --test overlay > "$here/c_head.log" 2>&1 || { echo "head: overlay arms not green"; exit 1; }
echo "head: overlay arms green"

# c_base_7affe96b.patch puts back 7affe96b's overlay.rs: no PATH_MAX refusal, no drop of a refused path's other ops.
git -C "$root" apply "$here/c_base_7affe96b.patch" || exit 2
cargo test --test overlay --no-run > "$here/c_base_build.log" 2>&1 || { echo "base: arms did not build"; exit 1; }
for arm in $arms; do
	if cargo test --test overlay "$arm" -- --exact > "$here/c_base_$arm.log" 2>&1; then
		echo "$arm: green at $base (no red)"; fail=1
	else
		echo "$arm: red at $base"
	fi
done
exit $fail
