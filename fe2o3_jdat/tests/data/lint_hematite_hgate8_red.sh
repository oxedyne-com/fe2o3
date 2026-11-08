#!/usr/bin/env bash
# QA hgate6 F1 and F3, the Cargo scratch guards, replayed from the commit: each arm GREEN as committed and RED
# on its mutant (lint_hematite_hgate8_<m>.patch), applied and reversed in turn.
#   g1 cargo_of's guard on a Cargo manifest above the scratch deleted
#   g2 cargo_of's guard on a .cargo config above the scratch deleted, so a planted rustc-wrapper runs
#   g3 cargo_of's guard on an ancestor another account can write deleted
#   g4 cargo_of's guard on a scratch open to other accounts deleted
#   g5 the CLI's 0700 on its scratch deleted
#   g6 the CLI's scratch back in the shared temporary directory
# Run from the fe2o3 root through rc-build: bash fe2o3_jdat/tests/data/lint_hematite_hgate8_red.sh
set -u
here=$(dirname "$0")
fails=0
build() { cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite --test lint_hematite_git --no-run > /dev/null 2>&1; }
arm() { # <want pass|fail> <mutant> <test file> <test name>
	local got=fail
	cargo test -q -p oxedyne_fe2o3_jdat --test "$3" -- --exact "$4" > /dev/null 2>&1 && got=pass
	echo "arm $4 mutant=$2: want $1, got $got"
	[ "$1" = "$got" ] || fails=$((fails + 1))
}
cases=(
	"g1 lint_hematite_git test_hematite_git_scratch_under_a_manifest_refused"
	"g2 lint_hematite_git test_hematite_git_planted_cargo_config_never_runs"
	"g3 lint_hematite_git test_hematite_git_scratch_another_account_can_write_refused"
	"g4 lint_hematite_git test_hematite_git_scratch_open_to_others_refused"
	"g5 lint_hematite_git test_hematite_git_private_scratch_named_read"
	"g5 lint_hematite_git test_hematite_git_path_dep_outside_the_tree_read"
	"g6 lint_hematite_git test_hematite_git_private_scratch_named_read"
)
build || { echo "build failed"; exit 2; }
for c in "${cases[@]}"; do
	set -- $c
	arm pass none "$2" "$3"
done
patches=(
	lint_hematite_hgate8_g1.patch
	lint_hematite_hgate8_g2.patch
	lint_hematite_hgate8_g3.patch
	lint_hematite_hgate8_g4.patch
	lint_hematite_hgate8_g5.patch
	lint_hematite_hgate8_g6.patch
)
for f in "${patches[@]}"; do
	p="$here/$f"
	m=${f##*_}; m=${m%.patch}
	git apply "$p" || { echo "patch $p did not apply"; exit 2; }
	if build; then
		for c in "${cases[@]}"; do
			set -- $c
			[ "$1" = "$m" ] && arm fail "$m" "$2" "$3"
		done
	else
		echo "mutant $m does not build"; fails=$((fails + 1))
	fi
	git apply -R "$p"
done
echo "lint_hematite hgate8 red: $fails wrong"
[ "$fails" -eq 0 ]
