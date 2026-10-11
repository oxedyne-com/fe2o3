#!/usr/bin/env python3
# Summarises parity.py's JSONL: counts and shares by tier and verdict, the aus_fails causes ranked by the
# number of documents each breaks, and a sample of differs rows. Usage: report_parity.py PARITY.jsonl [N].
import collections
import json
import sys

rows = {}
for l in open(sys.argv[1]):
	try:
		r = json.loads(l)
	except ValueError:
		continue
	rows[r["id"]] = r
top = int(sys.argv[2]) if len(sys.argv) > 2 else 25
V = ["equivalent", "near", "differs", "aus_fails", "typst_fails", "harness_error"]


def table(sel, name):
	c = collections.Counter(r["verdict"] for r in sel)
	base = sum(c[v] for v in V[:4])
	comp = c["equivalent"] + c["near"] + c["differs"]
	print("\n### %s (%d documents; %d where typst compiles)\n" % (name, len(sel), base))
	print("| Measure | Count | Share of typst-compiling |")
	print("|---|---:|---:|")
	pct = lambda n: "%.1f%%" % (100.0 * n / base) if base else "-"
	print("| Austenite compiles | %d | %s |" % (comp, pct(comp)))
	for v in V[:4]:
		print("| %s | %d | %s |" % (v, c[v], pct(c[v])))
	print("| typst_fails (apart) | %d | - |" % c["typst_fails"])
	if c["harness_error"]:
		print("| harness_error (apart) | %d | - |" % c["harness_error"])


allr = list(rows.values())
table(allr, "All tiers")
for t in ("universe", "fixture", "tree"):
	sel = [r for r in allr if r["tier"] == t]
	if sel:
		table(sel, "Tier " + t)

print("\n### aus_fails causes, ranked by documents broken\n")
print("| Rank | Cause | All | Universe | Where (top sites) |")
print("|---:|---|---:|---:|---|")
cz = collections.defaultdict(list)
for r in allr:
	if r["verdict"] == "aus_fails":
		cz[r.get("cause") or "?"].append(r)
for i, (k, rs) in enumerate(sorted(cz.items(), key=lambda kv: -len(kv[1]))[:top], 1):
	u = sum(1 for r in rs if r["tier"] == "universe")
	sites = collections.Counter(r.get("cause_site") or "?" for r in rs).most_common(3)
	where = ", ".join("%s %d" % s for s in sites)
	print("| %d | `%s` | %d | %d | %s |" % (i, k.replace("|", "/"), len(rs), u, where))

print("\n### differs: what differs\n")
d = [r for r in allr if r["verdict"] == "differs"]
pg = sum(1 for r in d if r.get("typst_pages") != r.get("aus_pages"))
tx = sum(1 for r in d if r.get("text_diff"))
print("%d rows: %d with a different page count, %d with text lines differing.\n" % (len(d), pg, tx))
print("| Document | Pages typst/aus | Text lines differing / typst lines | Worst raster | First text difference (typst / aus) |")
print("|---|---|---|---:|---|")
step = max(1, len(d) // 10)
def at_split(a, b):
	# The two lines from just before where they first part.
	i = 0
	while i < min(len(a), len(b)) and a[i] == b[i]:
		i += 1
	s = max(0, i - 25)
	return a[s:s + 60], b[s:s + 60]


for r in d[::step][:10]:
	tf = dict(r.get("text_first") or {})
	tf["typst"], tf["aus"] = at_split(tf.get("typst") or "", tf.get("aus") or "")
	print("| %s | %s/%s | %s/%s | %s | %s / %s |" % (r["id"], r.get("typst_pages"), r.get("aus_pages"),
		r.get("text_diff"), r.get("text_lines"), r.get("raster_worst"),
		tf["typst"].replace("|", "/"), tf["aus"].replace("|", "/")))
