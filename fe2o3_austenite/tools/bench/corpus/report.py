#!/usr/bin/env python3
"""Summarises the sweep's JSONL (sweep.sh) as markdown: robustness, speed ratios, the worst documents, the
stage ranking and the warm edit. A later file's record for an id replaces an earlier one, so a rerun file
goes last.

Usage: report.py [--rerun-ids FILE] [--fail-tsv FILE] <sweep.jsonl> [rerun.jsonl]...
  --rerun-ids  writes the ids to measure again with RUNS=4: the slowest 10 by ratio, every document over 3x,
               and every document with a run at a load above 6.
  --fail-tsv   writes every Austenite failure: id, tier, status, typst status, message.
"""

import json
import statistics
import sys

STAGES = ("load", "eval", "realise", "layout", "emit")
DOCS = ("universe", "syn", "own", "tree")	# the headline set; fixtures are reported apart


def pct(xs, p):
	xs = sorted(xs)
	if not xs:
		return float("nan")
	k = (len(xs) - 1) * p
	f = int(k)
	c = min(f + 1, len(xs) - 1)
	return xs[f] + (xs[c] - xs[f]) * (k - f)


def load_max(r):
	m = 0.0
	for e in ("aus", "typ"):
		for run in (r.get(e) or {}).get("runs", []):
			try:
				m = max(m, float(run.get("load") or 0))
			except ValueError:
				pass
	return m


def excess(r):
	a = (r["aus"].get("stages") or {})
	t = (r["typ"].get("stages") or {})
	if not a or not t:
		return None
	return {s: a.get(s, 0.0) - t.get(s, 0.0) for s in STAGES}


def dominant(r):
	x = excess(r)
	if x:
		s = max(x, key=x.get)
		return s, x[s]
	a = r["aus"].get("stages") or {}
	if a:
		s = max(STAGES, key=lambda k: a.get(k, 0.0))
		return s + " (share)", a.get(s, 0.0)
	return "-", 0.0


def ratio_stats(rs):
	xs = [r["ratio"] for r in rs]
	if not xs:
		return "n=0"
	w = max(rs, key=lambda r: r["ratio"])
	return (f"n={len(xs)}, median {statistics.median(xs):.2f}x, p90 {pct(xs, 0.9):.2f}x, "
		f"worst {w['ratio']:.2f}x ({w['id']}), geomean {statistics.geometric_mean(xs):.2f}x")


def main():
	args = sys.argv[1:]
	rerun_out = fail_out = None
	while args and args[0].startswith("--"):
		flag = args.pop(0)
		if flag == "--rerun-ids":
			rerun_out = args.pop(0)
		elif flag == "--fail-tsv":
			fail_out = args.pop(0)
	recs = {}
	for path in args:
		for line in open(path):
			try:
				r = json.loads(line)
			except ValueError:
				continue
			recs[r["id"]] = r
	allr = list(recs.values())
	staged = [r for r in allr if "aus" in r]
	print(f"# Sweep report: {len(allr)} records ({len(allr) - len(staged)} could not be staged)\n")

	# Robustness.
	print("## Robustness\n")
	print("| tier | docs | aus ok | aus error | panic | timeout | killed/OOM | typst ok | both ok | pages differ |")
	print("|---|---|---|---|---|---|---|---|---|---|")
	tiers = sorted({r["tier"] for r in allr})
	for tier in tiers:
		rs = [r for r in staged if r["tier"] == tier]
		st = lambda r: r["aus"]["status"]
		cnt = lambda s: sum(1 for r in rs if st(r) == s)
		both = [r for r in rs if st(r) == "ok" and r["typ"]["status"] == "ok"]
		diff = sum(1 for r in both if r.get("pages") and r.get("typst_pages") and r["pages"] != r["typst_pages"])
		nstage = sum(1 for r in allr if r["tier"] == tier and "aus" not in r)
		sig = sum(1 for r in rs if st(r).startswith("signal"))
		print(f"| {tier} | {len(rs)}{f' (+{nstage} unstaged)' if nstage else ''} | {cnt('ok')} | {cnt('error')} | "
			f"{cnt('panic') + sig} | {cnt('timeout')} | {cnt('killed')} | "
			f"{sum(1 for r in rs if r['typ']['status'] == 'ok')} | {len(both)} | {diff} |")
	crash = [r for r in staged if r["aus"]["status"] in ("panic", "timeout", "killed") or r["aus"]["status"].startswith("signal")]
	print(f"\n### Every Austenite crash, hang or kill ({len(crash)})\n")
	for r in sorted(crash, key=lambda r: (r["aus"]["status"], r["id"])):
		print(f"- `{r['id']}` ({r['tier']}): **{r['aus']['status']}**, typst {r['typ']['status']}: {r['aus'].get('msg', '')}")
	wfail = [r for r in staged if r.get("warm", {}).get("status") == "fail"]
	print(f"\n### Warm-edit failures ({len(wfail)})\n")
	for r in wfail[:40]:
		print(f"- `{r['id']}`: rc {r['warm'].get('rc')}: {r['warm'].get('msg', '')[:200]}")
	# Errors where typst succeeds, grouped by message head.
	refused = [r for r in staged if r["aus"]["status"] == "error" and r["typ"]["status"] == "ok"]
	groups = {}
	for r in refused:
		m = r["aus"].get("msg", "")
		key = m.split(" | ")[-1][:110]
		groups.setdefault(key, []).append(r["id"])
	print(f"\n### Austenite refuses, typst compiles ({len(refused)}), top messages\n")
	for k, ids in sorted(groups.items(), key=lambda kv: -len(kv[1]))[:25]:
		print(f"- {len(ids)} x `{k}` (e.g. {', '.join(ids[:3])})")
	fx = [r for r in staged if r["tier"] == "fixture"]
	agree = sum(1 for r in fx if (r["aus"]["status"] == "ok") == (r["typ"]["status"] == "ok"))
	print(f"\nFixtures: Austenite and typst agree on accept/reject for {agree} of {len(fx)}.")

	# Speed.
	print("\n## Speed (cold, Austenite wall / typst -j1 wall, median of runs)\n")
	both = [r for r in staged if r.get("ratio")]
	docs = [r for r in both if r["tier"] in DOCS]
	print(f"- **Documents (universe, syn, own, tree): {ratio_stats(docs)}**")
	big = [r for r in docs if r["typ"]["wall"] >= 0.5]
	print(f"- Documents where typst takes >= 0.5 s: {ratio_stats(big)}")
	for tier in tiers:
		print(f"- {tier}: {ratio_stats([r for r in both if r['tier'] == tier])}")
	tot_a = sum(r["aus"]["wall"] for r in docs)
	tot_t = sum(r["typ"]["wall"] for r in docs)
	print(f"- Summed wall over documents: Austenite {tot_a:.1f} s, typst {tot_t:.1f} s ({tot_a / max(tot_t, 1e-9):.2f}x)")
	over = [r for r in docs if r["ratio"] > 3]
	print(f"- Documents over 3x: {len(over)}; over 1.5x: {sum(1 for r in docs if r['ratio'] > 1.5)}")

	print("\n### The 20 worst documents by ratio\n")
	print("| doc | tier | pages | typst s | Austenite s | ratio | passes | dominant stage (excess s) | warm s / passes / view s | load max |")
	print("|---|---|---|---|---|---|---|---|---|---|")
	worst = sorted(docs, key=lambda r: -r["ratio"])[:20]
	for r in worst:
		dom, ex = dominant(r)
		w = r.get("warm") or {}
		ws = f"{w.get('compile_s', '-')} / {w.get('passes', '-')} / {w.get('save_to_view_s', '-')}" if "mode" in w else "-"
		st = r["aus"].get("stages") or {}
		print(f"| `{r['id']}` | {r['tier']} | {r.get('pages', '-')} | {r['typ']['wall']:.3f} | {r['aus']['wall']:.3f} | "
			f"{r['ratio']:.2f}x | {st.get('passes', '-')} | {dom} ({ex:+.3f}) | {ws} | {load_max(r):.1f} |")

	print("\n### The 10 slowest documents by Austenite wall\n")
	for r in sorted(docs, key=lambda r: -r["aus"]["wall"])[:10]:
		dom, ex = dominant(r)
		print(f"- `{r['id']}`: {r['aus']['wall']:.2f} s against {r['typ']['wall']:.2f} s ({r['ratio']:.2f}x), "
			f"{r.get('pages', '-')} pages, dominant {dom} ({ex:+.2f} s)")

	# Stage ranking.
	print("\n## Stages ranked by total excess over typst (documents with both stage splits)\n")
	tot = dict.fromkeys(STAGES, 0.0)
	pos = dict.fromkeys(STAGES, 0.0)
	dom_n = dict.fromkeys(STAGES, 0)
	a_tot = dict.fromkeys(STAGES, 0.0)
	t_tot = dict.fromkeys(STAGES, 0.0)
	n = 0
	for r in docs:
		x = excess(r)
		if not x:
			continue
		n += 1
		for s in STAGES:
			tot[s] += x[s]
			pos[s] += max(x[s], 0.0)
			a_tot[s] += r["aus"]["stages"].get(s, 0.0)
			t_tot[s] += r["typ"]["stages"].get(s, 0.0)
		if r["ratio"] > 1:
			dom_n[max(x, key=x.get)] += 1
	print(f"n={n}. Excess = Austenite stage time less typst's, summed over documents.\n")
	print("| stage | Austenite s | typst s | net excess s | positive excess s | dominant in (docs slower than typst) |")
	print("|---|---|---|---|---|---|")
	for s in sorted(STAGES, key=lambda s: -pos[s]):
		print(f"| {s} | {a_tot[s]:.1f} | {t_tot[s]:.1f} | {tot[s]:+.1f} | {pos[s]:.1f} | {dom_n[s]} |")
	flow = sum((r["aus"].get("stages") or {}).get("flow", 0.0) for r in docs if excess(r))
	print(f"\nOf Austenite's layout, flow (which holds line breaking) is {flow:.1f} s.")

	# Warm edit.
	ws = [r for r in docs if "mode" in (r.get("warm") or {})]
	if ws:
		cs = [r["warm"]["compile_s"] for r in ws]
		vs = [r["warm"]["save_to_view_s"] for r in ws]
		passes = {}
		for r in ws:
			passes[r["warm"]["passes"]] = passes.get(r["warm"]["passes"], 0) + 1
		cold = sum(1 for r in ws if r["warm"]["mode"] != "warm")
		print(f"\n## Warm one-letter edit (n={len(ws)}; {cold} ran cold)\n")
		print(f"- compile: median {statistics.median(cs):.3f} s, p90 {pct(cs, 0.9):.3f} s, worst {max(cs):.3f} s")
		print(f"- save-to-view: median {statistics.median(vs):.3f} s, p90 {pct(vs, 0.9):.3f} s, worst {max(vs):.3f} s "
			f"({max(ws, key=lambda r: r['warm']['save_to_view_s'])['id']})")
		print(f"- passes: {dict(sorted(passes.items(), key=lambda kv: (kv[0] is None, kv[0] or 0)))}")
		for r in sorted(ws, key=lambda r: -r["warm"]["save_to_view_s"])[:5]:
			print(f"  - `{r['id']}`: view {r['warm']['save_to_view_s']} s, compile {r['warm']['compile_s']} s, "
				f"{r['warm']['passes']} passes, cold {r['aus']['wall']:.2f} s")

	hi = [r for r in both if load_max(r) > 6]
	print(f"\nRuns measured at a load above 6: {len(hi)} documents.")

	if rerun_out:
		ids = {r["id"] for r in sorted(docs, key=lambda r: -r["ratio"])[:10]}
		ids |= {r["id"] for r in docs if r["ratio"] > 3}
		ids |= {r["id"] for r in hi if r["tier"] in DOCS}
		with open(rerun_out, "w") as f:
			for i in sorted(ids):
				f.write(i + "\n")
	if fail_out:
		with open(fail_out, "w") as f:
			for r in staged:
				if r["aus"]["status"] != "ok":
					f.write("\t".join([r["id"], r["tier"], r["aus"]["status"], r["typ"]["status"],
						r["aus"].get("msg", "").replace("\t", " ")]) + "\n")
			for r in allr:
				if "aus" not in r:
					f.write("\t".join([r["id"], r["tier"], "unstaged", "-", r.get("stage_err", "")]) + "\n")


if __name__ == "__main__":
	main()
