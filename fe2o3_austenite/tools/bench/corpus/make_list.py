#!/usr/bin/env python3
"""Writes real_corpus.txt, the whole-corpus list the sweep reads (D-20261009-10).

The corpus is every document Austenite is measured on, in five tiers:

  universe  every template on Typst Universe (packages.typst.org), at its latest version, as `typst
            init` materialises it. This is the web corpus of D-20260919-01: nothing is vendored in the
            repository, the list pins each package's version, and sweep.sh fetches it on first use.
  fixture   every oracle fixture under tests/fixtures/eval (helpers and `// oracle: none` files left
            out), marked accepts or rejects as the oracle reads it. Robustness and a single timing.
  tree      the other documents in the crate: samples/, the incremental fixtures, the oracle
            fixtures' lone files.
  syn       the synthetic stress set: syn50, syn300 (gen_synthetic.py, seed 20260923) and rich300
            (heap_probe.mjs --dump 300).
  own       the owner's Invitation and the Hematite guide, copied from where they live.

Usage: make_list.py <index.json> <crate dir> > real_corpus.txt
The index is https://packages.typst.org/preview/index.json, saved on the day the list is made.
"""

import json
import os
import sys


def version_key(v):
	return tuple(int(x) if x.isdigit() else 0 for x in v.split("."))


def universe(index_path):
	with open(index_path) as f:
		idx = json.load(f)
	latest = {}
	for p in idx:
		if not p.get("template"):
			continue
		cur = latest.get(p["name"])
		if cur is None or version_key(p["version"]) > version_key(cur["version"]):
			latest[p["name"]] = p
	out = []
	for name in sorted(latest):
		p = latest[name]
		entry = p["template"].get("entrypoint", "main.typ")
		cats = ",".join(p.get("categories") or []) or "-"
		lic = (p.get("license") or "-").replace(" ", "")
		out.append(f"universe\t{name}:{p['version']}\t{entry}\t{cats}\t{lic}")
	return out


def leading_directives(path):
	dirs = []
	try:
		with open(path, encoding="utf-8", errors="replace") as f:
			for line in f:
				s = line.strip()
				if not s.startswith("//"):
					break
				if s.startswith("// oracle:"):
					dirs.append(s[len("// oracle:"):].strip())
	except OSError:
		pass
	return dirs


def fixtures(crate):
	base = os.path.join(crate, "tests", "fixtures", "eval")
	out = []
	for area in sorted(os.listdir(base)):
		adir = os.path.join(base, area)
		if not os.path.isdir(adir):
			continue
		for name in sorted(os.listdir(adir)):
			p = os.path.join(adir, name)
			if not name.endswith(".typ") or name.startswith("_") or not os.path.isfile(p):
				continue
			d = leading_directives(p)
			if any(x.startswith("none") for x in d):
				continue
			expect = "rejects" if any(x.startswith("rejects") for x in d) else "accepts"
			rel = os.path.relpath(p, os.path.dirname(crate))
			out.append(f"fixture\t{area}/{name[:-4]}\t{rel}\t{expect}")
	return out


def tree(crate):
	out = []
	for sub in ("samples", "tests/fixtures/incremental", "tests/oracle/fixtures", "tests/fixtures/survey"):
		d = os.path.join(crate, sub)
		if not os.path.isdir(d):
			continue
		for name in sorted(os.listdir(d)):
			p = os.path.join(d, name)
			if name.endswith(".typ") and not name.startswith("_") and os.path.isfile(p):
				rel = os.path.relpath(p, os.path.dirname(crate))
				out.append(f"tree\t{sub.split('/')[-1]}/{name[:-4]}\t{rel}")
			elif os.path.isdir(p):
				for main in ("root.typ", "main.typ"):	# a multi-file fixture's root
					m = os.path.join(p, main)
					if os.path.isfile(m):
						rel = os.path.relpath(m, os.path.dirname(crate))
						out.append(f"tree\t{sub.split('/')[-1]}/{name}\t{rel}")
						break
	return out


def main():
	if len(sys.argv) != 3:
		sys.exit(__doc__)
	idx, crate = sys.argv[1], os.path.abspath(sys.argv[2])
	print("# The whole Austenite speed and robustness corpus (D-20261009-10). Written by make_list.py;")
	print("# read by sweep.sh. Fields are tab-separated:")
	print("#   universe <pkg:version> <entrypoint> <categories> <licence>")
	print("#   fixture  <area/name> <path from the repo root> <accepts|rejects>")
	print("#   tree     <id> <path from the repo root>")
	print("#   syn      <id> <gen:PAGES | heap:PAGES>")
	print("#   own      <id> <invitation | hematite>")
	for line in ["syn\tsyn50\tgen:50", "syn\tsyn300\tgen:300", "syn\trich300\theap:300",
			"own\tinvitation\tinvitation", "own\thematite\thematite"]:
		print(line)
	for line in tree(crate) + universe(idx) + fixtures(crate):
		print(line)


if __name__ == "__main__":
	main()
