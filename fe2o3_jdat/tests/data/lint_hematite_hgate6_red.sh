#!/usr/bin/env bash
# QA hgate5 C1, C2, C3, H1, D1 red claims, replayed from the commit: each arm GREEN as committed and
# RED on its mutant (lint_hematite_hgate6_<m>.patch), applied and reversed in turn.
#   c1 a #[path] or include! literal read raw, so an escape hides tests
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
)
build || { echo "build failed"; exit 2; }
for c in "${cases[@]}"; do
	set -- $c
	arm pass none "$2" "$3"
done
for p in "$here/lint_hematite_hgate6_c1.patch"; do
	m=${p##*_}; m=${m%.patch}
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
