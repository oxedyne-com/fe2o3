#!/bin/bash
# check.sh -- the memory gate's own check: heap_probe.mjs --gate-file on synthetic measurements whose
# median ratio and growth are known, each with the exit status and the ratio it must give. Run it from
# anywhere; it needs only node. Exit status 0 when every case behaves.
D=$(cd "$(dirname "$0")" && pwd)
P=$D/../heap_probe.mjs
bad=0
# file                       exit   text the output must carry
while read -r f want text; do
	out=$(node "$P" --gate-file "$D/$f" 2>&1)
	got=$?
	if [ "$got" != "$want" ] || ! grep -qF -- "$text" <<<"$out"; then
		echo "RED  $f: exit $got (wanted $want), wanted the text '$text'"
		echo "$out" | sed 's/^/     /'
		bad=1
	else
		echo "ok   $f: exit $got, '$text'"
	fi
done <<'CASES'
r061.json 0 R = H_A(300)/H_T(300) = 0.610
r062.json 0 R = H_A(300)/H_T(300) = 0.620
r063.json 1 R = H_A(300)/H_T(300) = 0.630
g047.json 1 = 0.470 MiB a page over 700 pages
g_per_page_produced.json 1 = 0.470 MiB a page over 650 pages
incomplete.json 2 gate INCOMPLETE: no measurement for austenite/1000
CASES
exit $bad
