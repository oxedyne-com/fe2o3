#!/usr/bin/env python3
# Compares two parity.py JSONL runs row by row: regressions (a row equivalent or near in the base that is
# now worse, equivalent to near included), gains, verdict moves and the compile, equivalent and near shares per tier.
# Usage: parity_diff.py BASE.jsonl NEW.jsonl [--moves]. Exits 1 on any regression.
import collections
import json
import sys

RANK = {"equivalent": 0, "near": 1, "differs": 2, "aus_fails": 3}
KEEP = ("equivalent", "near")


def load(path):
	rows = {}
	for l in open(path):
		try:
			r = json.loads(l)
		except ValueError:
			continue
		rows[r["id"]] = r
	return rows


def shares(rows, tier):
	c = collections.Counter(r["verdict"] for r in rows.values() if r["tier"] == tier)
	base = sum(c[v] for v in RANK)
	comp = c["equivalent"] + c["near"] + c["differs"]
	return base, comp, c["equivalent"], c["near"]


def pct(n, d):
	return "%.1f%%" % (100.0 * n / d) if d else "-"


def main(argv):
	if len(argv) < 3:
		print("usage: parity_diff.py BASE.jsonl NEW.jsonl [--moves]")
		return 2
	a = load(argv[1])
	b = load(argv[2])
	moves = "--moves" in argv
	tiers = sorted(set(r["tier"] for r in a.values()) | set(r["tier"] for r in b.values()))
	print("| Tier | Measure | Base | New | Delta |")
	print("|---|---|---:|---:|---:|")
	for t in tiers:
		ba, ca, ea, na = shares(a, t)
		bb, cb, eb, nb = shares(b, t)
		if not (ba or bb):
			continue
		for name, x, y in (("compile", ca, cb), ("equivalent", ea, eb), ("near", na, nb)):
			print("| %s | %s | %d/%d %s | %d/%d %s | %+d |" % (t, name, x, ba, pct(x, ba), y, bb, pct(y, bb), y - x))
	regs = []
	gains = []
	mv = collections.Counter()
	for k in sorted(set(a) & set(b)):
		va = a[k]["verdict"]
		vb = b[k]["verdict"]
		if va == vb:
			continue
		mv[(va, vb)] += 1
		if va in KEEP and RANK.get(vb, 9) > RANK[va]:
			regs.append((k, va, vb))
		elif vb in KEEP and va not in KEEP:
			gains.append((k, va, vb))
		if moves:
			print("move %s: %s -> %s" % (k, va, vb))
	print("\nVerdict moves:")
	for (va, vb), n in sorted(mv.items(), key=lambda x: -x[1]):
		print("  %s -> %s: %d" % (va, vb, n))
	print("\nGains into equivalent or near: %d" % len(gains))
	for k, va, vb in gains:
		print("  + %s: %s -> %s" % (k, va, vb))
	print("Regressions out of equivalent or near: %d" % len(regs))
	for k, va, vb in regs:
		print("  - %s: %s -> %s" % (k, va, vb))
	only = (set(a) ^ set(b))
	if only:
		print("Rows in one run only: %d" % len(only))
	return 1 if regs else 0


if __name__ == "__main__":
	sys.exit(main(sys.argv))
