# Shared bash helpers for the S0 bench harness: host-load reading, the memory
# cap, and one timed run. Sourced, not executed.

BENCH_LOAD_THRESHOLD="${BENCH_LOAD_THRESHOLD:-2}"   # 1-min loadavg above this flags a run (plan §S0)

# The 1-minute load average, as a bare number.
loadavg1() {
	awk '{print $1}' /proc/loadavg
}

# `some avg10` from /proc/pressure/<resource> (cpu, memory or io), or 0 if PSI
# is not exposed (some kernels/containers disable it) -- absence must not fail
# a run, only leave the PSI field at a documented zero.
psi_avg10() {
	local resource="$1"
	local f="/proc/pressure/${resource}"
	if [[ -r "$f" ]]; then
		awk -F'avg10=' '/^some/{split($2,a," "); print a[1]}' "$f"
	else
		echo "0"
	fi
}

# Is the host loaded right now, by the 1-minute average against the threshold?
host_is_loaded() {
	local l1
	l1="$(loadavg1)"
	awk -v l="$l1" -v t="$BENCH_LOAD_THRESHOLD" 'BEGIN{exit !(l>t)}'
}

# Runs one command under the memory cap and /usr/bin/time -v, emitting a single
# JSON line (via jq) to stdout with wall time, peak RSS and before/after host
# load. Args: <doc> <engine> <jobs> <out_jsonl_path> -- <cmd...>
run_timed() {
	local doc="$1" engine="$2" jobs="$3" out="$4"
	shift 4
	if [[ "$1" != "--" ]]; then
		echo "run_timed: expected -- before the command" >&2
		return 2
	fi
	shift

	local load_before load_after psi_cpu_before psi_cpu_after psi_mem_before psi_mem_after
	load_before="$(loadavg1)"
	psi_cpu_before="$(psi_avg10 cpu)"
	psi_mem_before="$(psi_avg10 memory)"

	local time_out rss_kb=0 exit_code=0
	time_out="$(mktemp)"
	local t0 t1 wall_s
	t0=$(date +%s.%N)
	systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice -- \
		/usr/bin/time -v -o "$time_out" -- "$@" \
		> /dev/null 2>&1
	exit_code=$?
	t1=$(date +%s.%N)
	wall_s=$(awk -v a="$t0" -v b="$t1" 'BEGIN{printf "%.4f", b-a}')

	if [[ -f "$time_out" ]]; then
		rss_kb="$(awk -F': ' '/Maximum resident set size/{print $2}' "$time_out")"
		[[ -z "$rss_kb" ]] && rss_kb=0
	fi
	rm -f "$time_out"

	load_after="$(loadavg1)"
	psi_cpu_after="$(psi_avg10 cpu)"
	psi_mem_after="$(psi_avg10 memory)"

	local flagged=false
	if awk -v a="$load_before" -v b="$load_after" -v t="$BENCH_LOAD_THRESHOLD" \
		'BEGIN{exit !(a>t || b>t)}'; then
		flagged=true
	fi

	jq -nc \
		--arg doc "$doc" --arg engine "$engine" --arg jobs "$jobs" \
		--argjson wall_s "$wall_s" --argjson rss_kb "${rss_kb:-0}" \
		--argjson exit_code "$exit_code" \
		--argjson load_before "$load_before" --argjson load_after "$load_after" \
		--argjson psi_cpu_before "$psi_cpu_before" --argjson psi_cpu_after "$psi_cpu_after" \
		--argjson psi_mem_before "$psi_mem_before" --argjson psi_mem_after "$psi_mem_after" \
		--argjson flagged_under_load "$flagged" \
		--arg ts "$(date -u +%FT%TZ)" \
		'{ts:$ts, doc:$doc, engine:$engine, jobs:$jobs, wall_s:$wall_s, rss_kb:$rss_kb,
		  exit_code:$exit_code, load_before:$load_before, load_after:$load_after,
		  psi_cpu_before:$psi_cpu_before, psi_cpu_after:$psi_cpu_after,
		  psi_mem_before:$psi_mem_before, psi_mem_after:$psi_mem_after,
		  flagged_under_load:$flagged_under_load}' \
		>> "$out"

	return $exit_code
}
