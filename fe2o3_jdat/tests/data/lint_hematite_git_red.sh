#!/usr/bin/env bash
# HGATE QA F1-F3, F5 red claim, replayed from the commit: tests/lint_hematite_git.rs is GREEN as
# committed and RED on the CLI's old diff (the repository's flags, no attributes refusal), applied
# from lint_hematite_git_m1.patch and reversed after.
# Run from the fe2o3 root through rc-build: bash fe2o3_jdat/tests/data/lint_hematite_git_red.sh
set -u
here=$(dirname "$0")
run() { cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite_git 2>&1 | sed -n 's/^test result: [a-zA-Z]*\. [0-9]* passed; \([0-9]*\) failed.*/\1/p'; }
fails=0
build() { cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite_git --no-run > /dev/null 2>&1 || { echo "build failed"; exit 2; }; }
build
n=$(run); echo "arm git committed: want 0 failed, got $n"; [ "$n" = 0 ] || fails=$((fails + 1))
git apply "$here/lint_hematite_git_m1.patch" || { echo "patch did not apply"; exit 2; }
trap 'git apply -R "$here/lint_hematite_git_m1.patch"' EXIT
build
n=$(run); echo "arm git old-diff mutant: want 5 failed, got $n"; [ "$n" = 5 ] || fails=$((fails + 1))
echo "lint_hematite_git red: $fails wrong"
[ "$fails" -eq 0 ]
