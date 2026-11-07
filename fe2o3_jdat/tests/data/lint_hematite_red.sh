#!/usr/bin/env bash
# HGATE red claims, replayed from the commit: the seeds arm is GREEN on the table as committed, and
# RED naming the rule when that one rule is switched off ("on": false) in a copy of the table.
# Run from the fe2o3 root through rc-build: bash fe2o3_jdat/tests/data/lint_hematite_red.sh
set -u
fails=0
table=hematite_rules.jdat
dir=$(mktemp -d -p "$HOME/.cache")
trap 'rm -rf "${dir:?}"' EXIT
cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite --no-run > /dev/null 2>&1 || { echo "build failed"; exit 2; }
run() { HEMATITE_RULES=$1 cargo test -q -p oxedyne_fe2o3_jdat --test lint_hematite -- --exact test_hematite_each_rule_refused_by_name_and_line 2>&1; }
if run "$PWD/$table" > "$dir/out" ; then echo "arm seeds table=committed: want pass, got pass"; else echo "arm seeds table=committed: want pass, got fail"; fails=$((fails + 1)); fi
for id in G1 G2 G3 G4 G5 G6 G7 G8 G9 G10 G11 G12 G13; do
	sed "s/{ \"id\": \"$id\", \(.*\)\"on\": true/{ \"id\": \"$id\", \1\"on\": false/" "$table" > "$dir/$id.jdat"
	if run "$dir/$id.jdat" > "$dir/out"; then got=pass; else got=fail; fi
	named=no; grep -q "seed for $id:" "$dir/out" && named=yes
	echo "arm seeds rule_off=$id: want fail naming $id, got $got naming=$named"
	[ "$got" = fail ] && [ "$named" = yes ] || fails=$((fails + 1))
done
echo "lint_hematite red: $fails wrong"
[ "$fails" -eq 0 ]
