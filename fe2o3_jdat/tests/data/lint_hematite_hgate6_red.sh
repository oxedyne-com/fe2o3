#!/usr/bin/env bash
# QA hgate5 C1, C2, C3, H1, D1 red claims, and the out-of-tree path dependency of hgate7 (d2-d4), replayed from the commit: each arm GREEN as committed and
# RED on its mutant (lint_hematite_hgate6_<m>.patch), applied and reversed in turn.
#   c1 a #[path] or include! literal read raw, so an escape hides tests
#   c2 a manifest building a lib from tests/ not refused
#   c3 test code read from the path (a tests component, no src), so a package in tests/ is test code
#   h1 c::test kept whatever crate Cargo resolves c to
#   t  a test file turned product, unchanged, not refused
#   d1 a changed build script or Cargo config not flagged for his card
#   d2 the tree written with no links to its ancestors' entries, so a path dependency leaving it is lost
#   d3 a manifest whose dependency is absent refused without naming where to check it out
#   d4 the CLI's Cargo scratch back inside the repository, under its manifest
# Run from the fe2o3 root through rc-build: bash fe2o3_jdat/tests/data/lint_hematite_hgate6_red.sh
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
	"c1 lint_hematite_git test_hematite_git_escaped_path_into_tests_refused"
	"c2 lint_hematite_git test_hematite_git_cargo_manifest_read_by_cargo"
	"c3 lint_hematite_git test_hematite_git_package_in_tests_is_product"
	"h1 lint_hematite_git test_hematite_git_renamed_test_crate_not_a_test"
	"h1 lint_hematite test_hematite_test_code_exempt_only_where_the_table_says"
	"t lint_hematite_git test_hematite_git_test_code_turned_product_refused"
	"d1 lint_hematite_git test_hematite_git_build_script_changed_flagged_not_refused"
	"d2 lint_hematite_git test_hematite_git_path_dep_outside_the_tree_read"
	"d3 lint_hematite_git test_hematite_git_missing_path_dep_named"
	"d4 lint_hematite_git test_hematite_git_path_dep_outside_the_tree_read"
)
build || { echo "build failed"; exit 2; }
for c in "${cases[@]}"; do
	set -- $c
	arm pass none "$2" "$3"
done
patches=(
	lint_hematite_hgate6_c1.patch
	lint_hematite_hgate6_c2.patch
	lint_hematite_hgate6_c3.patch
	lint_hematite_hgate6_h1.patch
	lint_hematite_hgate6_t.patch
	lint_hematite_hgate6_d1.patch
	lint_hematite_hgate6_d2.patch
	lint_hematite_hgate6_d3.patch
	lint_hematite_hgate6_d4.patch
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
echo "lint_hematite hgate6 red: $fails wrong"
[ "$fails" -eq 0 ]
