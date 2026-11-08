#!/usr/bin/env bash
# HGATE red claims, replayed from the commit: the seeds arm is GREEN on the table as committed, and
# RED naming the rule when its mutant switch HEMATITE_M<n> turns rule G<n> off.
# Run from the fe2o3 root through rc-build: bash fe2o3_jdat/tests/data/lint_hematite_red.sh
set -u
fails=0
dir=$(mktemp -d -p "$HOME/.cache")
trap 'rm -rf "${dir:?}"' EXIT
cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite --no-run > /dev/null 2>&1 || { echo "build failed"; exit 2; }
run() { env "$@" cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite -- --exact test_hematite_each_rule_refused_by_name_and_line 2>&1; }
if run > "$dir/out" ; then echo "arm seeds table=committed: want pass, got pass"; else echo "arm seeds table=committed: want pass, got fail"; fails=$((fails + 1)); fi
for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14; do
	id=G$i
	if run "HEMATITE_M$i=1" > "$dir/out"; then got=pass; else got=fail; fi
	named=no; grep -q "seed for $id:" "$dir/out" && named=yes
	echo "arm seeds rule_off=$id: want fail naming $id, got $got naming=$named"
	[ "$got" = fail ] && [ "$named" = yes ] || fails=$((fails + 1))
done
echo "lint_hematite red: $fails wrong"
[ "$fails" -eq 0 ]
