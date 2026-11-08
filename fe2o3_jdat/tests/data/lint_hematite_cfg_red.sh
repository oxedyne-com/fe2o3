#!/usr/bin/env bash
# cfg(all(test, …)) red claim, replayed from the commit: test_hematite_test_code_exempt_only_where_the_table_says
# is GREEN as committed and RED on the old test-code match (exactly #[cfg(test)], or a prefix
# #[cfg(all(test), applied from lint_hematite_cfg_m1.patch and reversed after.
# Run from the fe2o3 root through rc-build: bash fe2o3_jdat/tests/data/lint_hematite_cfg_red.sh
set -u
here=$(dirname "$0")
arm=test_hematite_test_code_exempt_only_where_the_table_says
build() { cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite --no-run > /dev/null 2>&1 || { echo "build failed"; exit 2; }; }
run() { if cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite -- --exact "$arm" > /dev/null 2>&1; then echo pass; else echo fail; fi; }
fails=0
build
got=$(run); echo "arm cfg committed: want pass, got $got"; [ "$got" = pass ] || fails=$((fails + 1))
git apply "$here/lint_hematite_cfg_m1.patch" || { echo "patch did not apply"; exit 2; }
trap 'git apply -R "$here/lint_hematite_cfg_m1.patch"' EXIT
build
got=$(run); echo "arm cfg old-match mutant: want fail, got $got"; [ "$got" = fail ] || fails=$((fails + 1))
echo "lint_hematite_cfg red: $fails wrong"
[ "$fails" -eq 0 ]
