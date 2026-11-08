#!/usr/bin/env bash
# SWT4 red claims.  Run through rc-build; exits 0 only when every claim holds.
#  m1: with the walk following links (m1_follow.patch), a hostile upper directory over a lower
#      symbolic link is walked through the link instead of refused.
#  QA SWT4 F1-F4: base_arms.rs, the fixes' arms in the old API, each fails beside d8b04911's
#      overlay.rs, while tests/overlay.rs passes at the tree.
set -o pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)" || exit 2
base=d8b04911
cd "$root/fe2o3_file" || exit 2
restore() {
	git -C "$root" checkout HEAD -- fe2o3_file/src/overlay.rs
	rm -f "$root/fe2o3_file/tests/base_arms.rs"
}
trap restore EXIT
fail=0

cargo test --test overlay > "$here/head.log" 2>&1 || { echo "head: overlay arms not green"; exit 1; }
echo "head: overlay arms green"

arm=overlay_refuses_symlinked_parent
git -C "$root" apply "$here/m1_follow.patch" || exit 2
cargo test --test overlay "$arm" > "$here/m1.log" 2>&1
st=$?
git -C "$root" apply -R "$here/m1_follow.patch" || exit 2
if [ $st -ne 0 ] && grep -q 'Create("link/evil"' "$here/m1.log"; then
	echo "m1_follow: red as claimed"
else
	echo "m1_follow: not red on the followed link"; fail=1
fi

git -C "$root" show "$base:fe2o3_file/src/overlay.rs" > src/overlay.rs || exit 2
cp "$here/base_arms.rs" tests/base_arms.rs || exit 2
scratch="${CARGO_TARGET_DIR:-$root/target}/tmp/overlay_base"
rm -rf "${scratch:?}"
cargo test --test base_arms --no-run > "$here/base_build.log" 2>&1 || { echo "base: arms did not build"; exit 1; }
for arm in base_f1_deep base_f2_odd_alone base_f3_xattr_meta base_f4_user_mark_is_data_under_trusted; do
	if cargo test --test base_arms "$arm" -- --exact > "$here/$arm.log" 2>&1; then
		echo "$arm: green at $base (no red)"; fail=1
	else
		echo "$arm: red at $base"
	fi
done
exit $fail
