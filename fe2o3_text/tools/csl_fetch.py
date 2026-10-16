#!/usr/bin/env python3
"""Rebuild fe2o3_text's CSL archive, `data/csl/`, as the CSL XML hayagriva 0.10.1 archived.

hayagriva embeds its 91 styles and 64 locales as CBOR in citationberg's serde schema. This script
reads each record's `<updated>` stamp and recovers, from the CSL styles and locales repositories,
the XML revision that carries it: walking a file's history backwards from hayagriva 0.10.1's
publication, it takes the first revision whose `<updated>` equals the record's. Where none does,
it takes the latest revision on or before the record's date and marks the row `nearest` in the
index. `alphanumeric` is hayagriva's own style and is copied from the crate.

Std-only. Usage:

    python3 -I tools/csl_fetch.py HAYAGRIVA_CRATE_DIR STYLES_REPO LOCALES_REPO OUT_DATA_DIR OUT_RS

where the repositories are clones (a `--filter=blob:none` clone is enough) of
https://github.com/citation-style-language/styles and .../locales, OUT_DATA_DIR is
`fe2o3_text/data/csl` and OUT_RS is `fe2o3_text/src/csl/archive_index.rs`.
"""

import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cbor  # noqa: E402

CUTOFF = "2026-06-14T10:08:30Z"  # hayagriva 0.10.1 on crates.io
HAYAGRIVA = "hayagriva 0.10.1"


def git(repo, *args, binary=False):
	out = subprocess.run(["git", "-C", repo] + list(args), check=True, capture_output=True)
	return out.stdout if binary else out.stdout.decode("utf-8")


def updated_of_xml(text):
	m = re.search(r"<updated>\s*([^<\s]+)\s*</updated>", text)
	return m.group(1) if m else ""


def updated_of_record(rec):
	info = rec.get("info") or {}
	up = info.get("updated")
	if isinstance(up, dict):
		up = up.get("$text") or up.get("$value")
	return up or ""


def same_stamp(a, b):
	# citationberg normalises the timestamp; compare its date and time, not its spelling.
	def norm(s):
		s = s.strip().replace("Z", "+00:00")
		return s[:19]
	return norm(a) == norm(b)


def recover(repo, path, stamp):
	revs = git(repo, "log", "--format=%H %cI", "--before=" + CUTOFF, "--", path).split("\n")
	revs = [r.split(" ") for r in revs if r.strip()]
	if not revs:
		raise SystemExit("no history for %s in %s" % (path, repo))
	fallback = None
	for sha, date in revs:
		text = git(repo, "show", "%s:%s" % (sha, path))
		up = updated_of_xml(text)
		if same_stamp(up, stamp):
			return sha, text, "exact"
		if fallback is None and up[:10] <= stamp[:10]:
			fallback = (sha, text, "nearest")
		if up[:10] < stamp[:10] and fallback is not None:
			break
	if fallback is None:
		raise SystemExit("no revision of %s at or before %s" % (path, stamp))
	return fallback


def parse_archive_rs(src):
	# Variant to CBOR file, and the names hayagriva accepts for each variant.
	files = dict(re.findall(r"Self::(\w+) => include_bytes!\(\"\.\./\.\./archive/styles/([\w.-]+)\.cbor\"\)", src))
	body = src[src.index("pub fn by_name"):src.index("pub fn by_id")]
	names = {}
	for name, var in re.findall(r"\"([^\"]+)\" => Some\(Self::(\w+)\)", body):
		names.setdefault(var, []).append(name)
	order = re.findall(r"Self::(\w+) => include_bytes!", src)
	locales = re.findall(r"include_bytes!\(\"\.\./\.\./archive/locales/([\w-]+)\.cbor\"\)", src)
	return order, files, names, locales


def rust_str(s):
	return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main(argv):
	if len(argv) != 6:
		raise SystemExit(__doc__)
	crate, styles_repo, locales_repo, out_dir, out_rs = argv[1:]
	src = open(os.path.join(crate, "src/csl/archive.rs"), encoding="utf-8").read()
	order, files, names, locales = parse_archive_rs(src)
	os.makedirs(os.path.join(out_dir, "styles"), exist_ok=True)
	os.makedirs(os.path.join(out_dir, "locales"), exist_ok=True)
	style_rows = []
	for var in order:
		stem = files[var]
		rec = cbor.decode(open(os.path.join(crate, "archive/styles", stem + ".cbor"), "rb").read())
		stamp = updated_of_record(rec)
		cid = rec["info"]["id"]
		if stem == "alphanumeric":
			text = open(os.path.join(crate, "styles/alphanumeric.csl"), encoding="utf-8").read()
			sha, how = HAYAGRIVA, "crate"
		else:
			base = cid.rsplit("/", 1)[-1]
			sha, text, how = recover(styles_repo, base + ".csl", stamp)
		with open(os.path.join(out_dir, "styles", stem + ".csl"), "w", encoding="utf-8") as f:
			f.write(text)
		style_rows.append((stem, cid, names.get(var, []), stamp, sha, how))
		print("style  %-60s %s %s" % (stem, how, sha[:10]), file=sys.stderr)
	locale_rows = []
	newest = ""
	for code in locales:
		rec = cbor.decode(open(os.path.join(crate, "archive/locales", code + ".cbor"), "rb").read())
		stamp = updated_of_record(rec)
		path = "locales-%s.xml" % code
		sha, text, how = recover(locales_repo, path, stamp)
		date = git(locales_repo, "log", "-1", "--format=%cI", sha).strip()
		newest = max(newest, date)
		with open(os.path.join(out_dir, "locales", code + ".xml"), "w", encoding="utf-8") as f:
			f.write(text)
		locale_rows.append((code, stamp, sha, how))
		print("locale %-10s %s %s" % (code, how, sha[:10]), file=sys.stderr)
	# locales.json as it stood with the newest archived locale.
	jsha = git(locales_repo, "log", "-1", "--format=%H", "--before=" + newest, "--", "locales.json").strip()
	with open(os.path.join(out_dir, "locales.json"), "w", encoding="utf-8") as f:
		f.write(git(locales_repo, "show", "%s:locales.json" % jsha))
	with open(os.path.join(out_dir, "index.tsv"), "w", encoding="utf-8") as f:
		f.write("# kind\tfile\tcsl id or code\tnames\tupdated\tsource revision\tmatch\n")
		for stem, cid, ns, stamp, sha, how in style_rows:
			f.write("style\tstyles/%s.csl\t%s\t%s\t%s\t%s\t%s\n" % (stem, cid, ",".join(ns), stamp, sha, how))
		for code, stamp, sha, how in locale_rows:
			f.write("locale\tlocales/%s.xml\t%s\t\t%s\t%s\t%s\n" % (code, code, stamp, sha, how))
		f.write("json\tlocales.json\t\t\t\t%s\texact\n" % jsha)
	write_rs(out_rs, style_rows, locale_rows)


def write_rs(path, style_rows, locale_rows):
	out = []
	out.append("// Generated by tools/csl_fetch.py from hayagriva 0.10.1's archive. Do not edit by hand.\n")
	out.append("// Columns: file stem, CSL id, the names Typst accepts, `<updated>` as hayagriva archived it.\n")
	out.append("pub(super) const STYLES: &[(&str, &str, &[&str], &str)] = &[\n")
	for stem, cid, ns, stamp, _sha, _how in style_rows:
		out.append("\t(%s, %s, &[%s], %s),\n" % (rust_str(stem), rust_str(cid), ", ".join(rust_str(n) for n in ns), rust_str(stamp)))
	out.append("];\n\n")
	out.append("pub(super) const LOCALES: &[&str] = &[\n")
	for code, _stamp, _sha, _how in locale_rows:
		out.append("\t%s,\n" % rust_str(code))
	out.append("];\n\n")
	out.append("#[cfg(feature = \"csl-archive\")]\n")
	out.append("pub(super) fn style_xml(stem: &str) -> Option<&'static str> {\n\tSome(match stem {\n")
	for stem, *_ in style_rows:
		out.append("\t\t%s\t=> include_str!(\"../../data/csl/styles/%s.csl\"),\n" % (rust_str(stem), stem))
	out.append("\t\t_ => return None,\n\t})\n}\n\n")
	out.append("#[cfg(feature = \"csl-archive\")]\n")
	out.append("pub(super) fn locale_xml(code: &str) -> Option<&'static str> {\n\tSome(match code {\n")
	for code, *_ in locale_rows:
		out.append("\t\t%s\t=> include_str!(\"../../data/csl/locales/%s.xml\"),\n" % (rust_str(code), code))
	out.append("\t\t_ => return None,\n\t})\n}\n\n")
	out.append("#[cfg(feature = \"csl-archive\")]\n")
	out.append("pub(super) const LOCALES_JSON: &str = include_str!(\"../../data/csl/locales.json\");\n")
	with open(path, "w", encoding="utf-8") as f:
		f.write("".join(out))


if __name__ == "__main__":
	main(sys.argv)
