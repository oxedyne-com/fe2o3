#!/usr/bin/env python3
"""Folds one document's run directory into one JSONL record (called by sweep.sh).

The directory holds, for run i, `a<i>.{time,out,err,json,load,rc}` (Austenite) and `t<i>.{time,err,load,rc}`
(typst), `t.pages`, optionally `tt.json` and `tt.time` (one typst run under `--timings`, for its stage split)
and `w.out` (watch_edit.sh's line). Stages are put side by side under one set of names:

  load     Austenite run.load; typst's wall time outside its trace plus parsing and font discovery
  eval     run.eval; typst `eval`
  realise  the passes' realise; typst `realize` and the show rules under it (each span takes the
           stage of its own name, else of its nearest named ancestor)
  layout   the passes' flow, place, decorate, settle and probe (line breaking is inside flow); typst's
           layout iterations less their realize
  emit     sink, finish and write; typst `pdf`

Typst's stages are read from the trace run and fitted to its median wall time (see typ_stages), so the
trace's own cost does not count. Usage: rec.py <dir> <runs> <meta-json>
"""

import json
import os
import re
import statistics
import sys

STAGES = ("load", "eval", "realise", "layout", "emit")
# Typst's layout spans that are not named for it; a show rule's own span under `realize` stays realise.
LAYOUT = {"block", "collect", "prepare", "finalize", "page run", "introspect pages", "check stabilized",
	"grid_to_cellgrid"}


def read(path, default=""):
	try:
		with open(path, encoding="utf-8", errors="replace") as f:
			return f.read()
	except OSError:
		return default


def classify(rc, err):
	if rc == 0:
		return "ok"
	if rc == 124:
		return "timeout"
	if rc in (137, 9):
		return "killed"
	if "panicked at" in err or rc == 101:
		return "panic"
	if rc > 128:
		return f"signal{rc - 128}"
	return "error"


def message(err):
	lines = [l.strip() for l in err.splitlines() if l.strip()]
	pan = [l for l in lines if "panicked at" in l]
	if pan:
		i = lines.index(pan[0])
		return " | ".join(lines[i:i + 2])[:400]
	errs = [l for l in lines if l.startswith("error") or "Err" in l or "rror:" in l]
	pick = errs[-2:] if errs else lines[-2:]
	return " | ".join(pick)[:400]


def engine(d, tag, runs):
	out = {"runs": []}
	worst = None
	for i in range(1, runs + 1):
		rc_s = read(os.path.join(d, f"{tag}{i}.rc")).strip()
		if not rc_s:
			break
		rc = int(rc_s)
		err = read(os.path.join(d, f"{tag}{i}.err"))
		st = classify(rc, err)
		r = {"rc": rc, "status": st, "load": read(os.path.join(d, f"{tag}{i}.load")).strip()}
		t = read(os.path.join(d, f"{tag}{i}.time")).split()
		if st == "ok" and len(t) >= 4:
			r.update(wall=float(t[0]), user=float(t[1]), sys=float(t[2]), rss_kb=int(t[3]))
			w = read(os.path.join(d, f"{tag}{i}.wall")).strip()
			if w:
				r["wall"] = float(w)
		out["runs"].append(r)
		if st != "ok" and worst is None:
			worst = (st, message(err))
	if not out["runs"]:
		out["status"] = "absent"
		return out
	if worst:
		out["status"], out["msg"] = worst
	else:
		out["status"] = "ok"
		walls = [r["wall"] for r in out["runs"] if "wall" in r]
		out["wall"] = statistics.median(walls) if walls else None
		out["cpu"] = statistics.median([r["user"] + r["sys"] for r in out["runs"] if "user" in r]) if walls else None
		out["rss_kb"] = max(r.get("rss_kb", 0) for r in out["runs"])
	return out


def aus_stages(d, runs, wall):
	# The run whose wall is the median's, else the last.
	best, best_gap = None, None
	for i in range(1, runs + 1):
		t = (read(os.path.join(d, f"a{i}.wall")) or read(os.path.join(d, f"a{i}.time"))).split()
		p = os.path.join(d, f"a{i}.json")
		if not t or not os.path.exists(p):
			continue
		gap = abs(float(t[0]) - wall) if wall is not None else 0
		if best_gap is None or gap < best_gap:
			best, best_gap = p, gap
	if best is None:
		return None
	try:
		j = json.loads(read(best))
	except ValueError:
		return None
	ns = lambda node, k: (node.get(k) or {}).get("ns", 0) / 1e9
	run = j.get("run", {})
	s = dict.fromkeys(STAGES, 0.0)
	s["load"] = ns(run, "load")
	s["eval"] = ns(run, "eval")
	s["emit"] = ns(run, "finish") + ns(run, "write")
	flow = 0.0
	for p in j.get("pass", []):
		s["realise"] += ns(p, "realise")
		f = ns(p, "flow")
		flow += f
		s["layout"] += f + ns(p, "place") + ns(p, "decorate") + ns(p, "settle") + ns(p, "probe")
		s["emit"] += ns(p, "sink")
	s["flow"] = flow
	s["total"] = j.get("total", 0) / 1e9
	s["passes"] = j.get("passes")
	return s


def typ_stages(d, wall):
	p = os.path.join(d, "tt.json")
	if not os.path.exists(p) or wall is None:
		return None
	try:
		ev = json.loads(read(p))
	except ValueError:
		return None
	cat_of = {"eval": "eval", "create source": "load", "parse": "load", "discover fonts": "load",
		"realize": "realise", "layout document": "layout", "pdf": "emit"}
	acc = dict.fromkeys(STAGES, 0.0)
	stack = []	# (name, start, category, child time)
	for e in ev:
		ph = e.get("ph")
		if ph == "B":
			name = e.get("name", "")
			cat = cat_of.get(name)
			if cat is None and (name.startswith(("iter (", "layout", "linebreak", "math ")) or name in LAYOUT):
				cat = "layout"
			if cat is None and stack:
				cat = stack[-1][2]
			stack.append([name, e["ts"], cat, 0.0])
		elif ph == "E" and stack:
			name, start, cat, child = stack.pop()
			dur = e["ts"] - start
			if cat:
				acc[cat] += (dur - child) / 1e6
			if stack:
				stack[-1][3] += dur if cat else 0.0
	# The trace run's own wall is noisy (the trace costs time), so the median wall less what was traced is
	# the load; a trace that adds up to more than the median wall is scaled down to it.
	traced = sum(acc.values())
	if traced > wall:
		return {k: v / traced * wall for k, v in acc.items()}
	acc["load"] += wall - traced
	return acc


def main():
	d, runs, meta = sys.argv[1], int(sys.argv[2]), json.loads(sys.argv[3])
	rec = dict(meta)
	a = engine(d, "a", runs)
	t = engine(d, "t", runs)
	m = re.search(r"-> (\d+) page\(s\)", read(os.path.join(d, "a1.out")))
	if m:
		rec["pages"] = int(m.group(1))
	tp = read(os.path.join(d, "t.pages")).strip()
	if tp.isdigit():
		rec["typst_pages"] = int(tp)
	if a.get("status") == "ok":
		a["stages"] = aus_stages(d, runs, a.get("wall"))
	if t.get("status") == "ok":
		t["stages"] = typ_stages(d, t.get("wall"))
	rec["aus"], rec["typ"] = a, t
	if a.get("wall") and t.get("wall"):
		rec["ratio"] = round(a["wall"] / t["wall"], 3)
	w = read(os.path.join(d, "w.out"))
	if w:
		m = re.search(r"mode=(\S*) compile_s=(\S*) passes=(\S*) save_to_view_s=(\S*)", w)
		rc = read(os.path.join(d, "w.rc")).strip()
		if m:
			rec["warm"] = {"mode": m.group(1), "compile_s": float(m.group(2) or "nan"),
				"passes": int(m.group(3)) if m.group(3).isdigit() else None,
				"save_to_view_s": float(m.group(4) or "nan")}
		else:
			rec["warm"] = {"status": "fail", "rc": int(rc) if rc.lstrip("-").isdigit() else None,
				"msg": message(read(os.path.join(d, "w.err")) + "\n" + w)}
	print(json.dumps(rec, separators=(",", ":")))


if __name__ == "__main__":
	main()
