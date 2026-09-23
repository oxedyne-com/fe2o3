#!/usr/bin/env python3
"""Aggregates the S0 bench harness's raw JSONL runs into one JSON report and
one markdown summary.

A run flagged `flagged_under_load` (the host's 1-minute load average was above
`BENCH_LOAD_THRESHOLD`, default 2, before or after that run) is never folded
into a median silently. By default the whole report refuses to print numbers
when ANY run is flagged: `--force` overrides that and prints them anyway,
loudly labelled as taken under load. This is the harness's own gate on the
plan's rule: "A run whose load average is above 2 is flagged and repeated,
never averaged in silently."

Usage: aggregate.py --in DIR [--in DIR ...] --out-json FILE --out-md FILE
                     [--force] [--pages N]
"""
import argparse
import json
import statistics as st
import sys
from collections import defaultdict
from pathlib import Path


def load_jsonl(path):
	rows = []
	if not path.exists():
		return rows
	for line in path.read_text().splitlines():
		line = line.strip()
		if not line:
			continue
		try:
			rows.append(json.loads(line))
		except json.JSONDecodeError as e:
			print(f"aggregate: skipping malformed line in {path}: {e}", file=sys.stderr)
	return rows


def median_min_max(values):
	if not values:
		return None
	return {"median": st.median(values), "min": min(values), "max": max(values), "n": len(values)}


def summarise_wall(rows, key_fields, wall_field="wall_s"):
	"""Groups rows by key_fields, returns {key: median_min_max} over wall_field."""
	groups = defaultdict(list)
	for r in rows:
		if r.get("exit_code", 0) != 0 and wall_field == "wall_s":
			continue  # a failed run has no meaningful wall time
		key = tuple(r.get(f) for f in key_fields)
		v = r.get(wall_field)
		if v is not None:
			groups[key].append(v)
	return {k: median_min_max(v) for k, v in groups.items()}


def main():
	ap = argparse.ArgumentParser()
	ap.add_argument("--in", dest="in_dirs", action="append", required=True)
	ap.add_argument("--out-json", required=True)
	ap.add_argument("--out-md", required=True)
	ap.add_argument("--force", action="store_true",
		help="print numbers even though some runs were flagged under load")
	ap.add_argument("--label", default="", help="one line noting the run's circumstances")
	args = ap.parse_args()

	native, wasm, edits = [], [], []
	for d in args.in_dirs:
		d = Path(d)
		native += load_jsonl(d / "native_runs.jsonl")
		wasm += load_jsonl(d / "wasm_runs.jsonl")
		edits += load_jsonl(d / "edit_latency_runs.jsonl")

	all_rows = native + wasm + edits
	flagged = [r for r in all_rows if r.get("flagged_under_load")]
	under_load = bool(flagged)

	report = {
		"label": args.label,
		"under_load": under_load,
		"flagged_run_count": len(flagged),
		"total_run_count": len(all_rows),
		"forced": args.force,
		"native": {},
		"wasm_compile": {},
		"edit_latency": {},
	}

	if under_load and not args.force:
		Path(args.out_json).write_text(json.dumps(report, indent=2) + "\n")
		Path(args.out_md).write_text(
			"# S0 bench report -- REFUSED\n\n"
			f"{len(flagged)} of {len(all_rows)} runs were made while the host's 1-minute load "
			"average was above the threshold. This harness refuses to report numbers taken under "
			"load, per the S0 protocol, unless run with `--force`.\n\n"
			"Re-run when the host is idle, or pass `--force` to see the (labelled, unreliable) "
			"numbers anyway.\n"
		)
		print("REFUSED: runs were made under load; re-run idle or pass --force", file=sys.stderr)
		return

	# Native: doc x engine(jobs)
	native_summary = summarise_wall(native, ["doc", "engine"])
	rss_summary = summarise_wall(native, ["doc", "engine"], wall_field="rss_kb")
	for (doc, engine), stat in native_summary.items():
		report["native"].setdefault(doc, {})[engine] = {
			"wall_s": stat,
			"rss_kb": rss_summary.get((doc, engine)),
		}
	# Ratio austenite-native / typst-j16 median, per doc.
	for doc, engines in report["native"].items():
		a = engines.get("austenite-native", {}).get("wall_s")
		t16 = engines.get("typst-j16", {}).get("wall_s")
		if a and t16 and t16["median"]:
			report["native"][doc]["ratio_austenite_over_typst_j16"] = round(a["median"] / t16["median"], 3)

	# Wasm compile: doc x engine(wasm-austenite/wasm-typstts). Each row holds one
	# process invocation's `runs` array (every warm-up-then-measured call inside
	# that one capped process), not a single `wall_s` -- unlike the native rows,
	# where `run_timed` writes one row per measured run.
	wasm_by_key = defaultdict(list)
	wasm_rss_rows = defaultdict(list)
	for r in wasm:
		if r.get("exit_code", 0) != 0 or not r.get("ok", True):
			continue
		key = (r.get("doc"), r.get("engine"))
		wasm_by_key[key].extend(r.get("runs") or [])
		if r.get("rss_kb") is not None:
			wasm_rss_rows[key].append(r["rss_kb"])
	for key, samples in wasm_by_key.items():
		doc, engine = key
		report["wasm_compile"].setdefault(doc, {})[engine] = {
			"wall_s": median_min_max(samples),
			"rss_kb": median_min_max(wasm_rss_rows.get(key, [])),
		}
	for doc, engines in report["wasm_compile"].items():
		a = engines.get("wasm-austenite", {}).get("wall_s")
		t = engines.get("wasm-typstts", {}).get("wall_s")
		if a and t and t["median"]:
			report["wasm_compile"][doc]["ratio_austenite_over_typstts"] = round(a["median"] / t["median"], 3)

	# Edit latency: p50_s/p95_s come pre-computed from edit_latency.mjs, one row per (doc, engine).
	for r in edits:
		doc, engine = r.get("doc"), r.get("engine")
		if doc is None:
			continue
		report["edit_latency"].setdefault(doc, {})[engine] = {
			"p50_s": r.get("p50_s"), "p95_s": r.get("p95_s"),
			"edits": r.get("edits"), "rss_kb": r.get("rss_kb"),
			"ok": r.get("ok"),
		}

	Path(args.out_json).write_text(json.dumps(report, indent=2) + "\n")

	md = []
	md.append("# S0 bench report")
	if args.label:
		md.append(f"\n{args.label}\n")
	if under_load:
		md.append(
			f"\n**TAKEN UNDER LOAD** -- {len(flagged)}/{len(all_rows)} runs were flagged "
			"(1-minute load average above threshold before or after the run). These numbers "
			"are NOT a valid baseline; re-run when the host is idle.\n"
		)
	md.append("\n## Native (typst -j16 / -j1 vs austenite)\n")
	md.append("| Doc | Engine | wall median (s) | min | max | n | peak RSS median (kB) |")
	md.append("|---|---|---|---|---|---|---|")
	for doc, engines in sorted(report["native"].items()):
		for engine, v in sorted(engines.items()):
			if engine.startswith("ratio_"):
				continue
			w = v["wall_s"]
			rss = v.get("rss_kb") or {}
			md.append(f"| {doc} | {engine} | {w['median']:.3f} | {w['min']:.3f} | {w['max']:.3f} | "
				f"{w['n']} | {rss.get('median', '-')} |")
		ratio = engines.get("ratio_austenite_over_typst_j16")
		if ratio is not None:
			md.append(f"| {doc} | **ratio austenite/typst-j16** | {ratio} | | | | |")

	md.append("\n## Wasm compile (Austenite wasm vs typst.ts wasm, node)\n")
	md.append("| Doc | Engine | wall median (s) | min | max | n | peak RSS median (kB) |")
	md.append("|---|---|---|---|---|---|---|")
	for doc, engines in sorted(report["wasm_compile"].items()):
		for engine, v in sorted(engines.items()):
			if engine.startswith("ratio_"):
				continue
			w = v["wall_s"]
			rss = v.get("rss_kb") or {}
			md.append(f"| {doc} | {engine} | {w['median']:.3f} | {w['min']:.3f} | {w['max']:.3f} | "
				f"{w['n']} | {rss.get('median', '-')} |")
		ratio = engines.get("ratio_austenite_over_typstts")
		if ratio is not None:
			md.append(f"| {doc} | **ratio austenite/typst.ts** | {ratio} | | | | |")

	md.append("\n## Edit latency (one-character edit to recompiled page)\n")
	md.append("| Doc | Engine | p50 (s) | p95 (s) | edits | peak RSS (kB) | ok |")
	md.append("|---|---|---|---|---|---|---|")
	def fmt4(x):
		return f"{x:.4f}" if isinstance(x, (int, float)) else "-"

	for doc, engines in sorted(report["edit_latency"].items()):
		for engine, v in sorted(engines.items()):
			md.append(f"| {doc} | {engine} | {fmt4(v['p50_s'])} | {fmt4(v['p95_s'])} | "
				f"{v['edits']} | {v.get('rss_kb', '-')} | {v['ok']} |")

	Path(args.out_md).write_text("\n".join(md) + "\n")
	print(f"wrote {args.out_json} and {args.out_md}", file=sys.stderr)


if __name__ == "__main__":
	main()
