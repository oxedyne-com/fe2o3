#!/usr/bin/env bash
# QA hgate4 H, F, B, C, D, E red claims, replayed from the commit: each arm GREEN as committed and
# RED on its mutant (lint_hematite_hgate5_<m>.patch), applied and reversed in turn.
#   h  a test attribute exempt by suffix ::test, not its exact path
#   f  every compared literal read as a pattern (hosts, model ids, paths, URLs pass G11)
#   b  an added symlink not refused
#   c1 a Cargo.toml path or build into tests not refused; c2 a crate under x/tests/src read as tests
#   d  a line setting cfg test not refused
#   e  a line moved from test to product code not counted as added
# Run from the fe2o3 root through rc-build: bash fe2o3_jdat/tests/data/lint_hematite_hgate5_red.sh
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
	"h lint_hematite test_hematite_test_code_exempt_only_where_the_table_says"
	"f lint_hematite test_hematite_each_rule_refused_by_name_and_line"
	"b lint_hematite_git test_hematite_git_symlink_refused"
	"c1 lint_hematite_git test_hematite_git_product_code_from_tests_refused"
	"c2 lint_hematite_git test_hematite_git_product_code_from_tests_refused"
	"d lint_hematite test_hematite_each_rule_refused_by_name_and_line"
	"e lint_hematite test_hematite_line_moved_from_test_to_product_is_added"
	"e lint_hematite_git test_hematite_git_deleted_cfg_test_reads_the_module"
)
build || { echo "build failed"; exit 2; }
for c in "${cases[@]}"; do
	set -- $c
	arm pass none "$2" "$3"
done
for m in h f b c1 c2 d e; do
	p="$here/lint_hematite_hgate5_$m.patch"
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
echo "lint_hematite hgate5 red: $fails wrong"
[ "$fails" -eq 0 ]
