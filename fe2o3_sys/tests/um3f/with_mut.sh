#!/usr/bin/env bash
# with_mut.sh CMD... -- QA um3 B F1's mutant: runs CMD with no_surv.patch applied to this tree (Gate::pass's ignore
# mark cleared by a modify, as before the fix), then reverses it; nothing committed.  Kimberlite's
# dev/kimb_usermgr/vm/um2s_vm.host.sh builds fe2o3_fanotify.mut and kimberlite.nosurv through it, and its VM round
# shows test_fanotify_gate_pass_lets_later_writes_through_as_root and the cost write rows RED on them.
set -euo pipefail
cd "$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
P=fe2o3_sys/tests/um3f/no_surv.patch
git apply "$P"; trap 'git apply -R "$P"' EXIT
"$@"
git apply -R "$P"; trap - EXIT
git diff --quiet -- fe2o3_sys/src
