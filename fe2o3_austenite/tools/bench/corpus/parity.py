#!/usr/bin/env python3
# Render parity over the whole corpus (FIRE-3, 2026-10-11): every document in real_corpus.txt is copied once
# to a work directory and compiled from that one copy by Austenite's evaluator (`--eval`) and by typst
# 0.15.1, each under its own memory scope and wall-time cap. The two PDFs are compared by page count, by
# `pdftotext -layout` text (runs of spaces collapsed, then the count of differing lines) and by raster (every
# page up to MAXPAGES at DPI, `compare -metric AE -fuzz 10%`, the worst page's fraction). One JSONL row per
# document. A crash, hang or out-of-memory is recorded and the run moves on. A document already in the JSONL
# is skipped, so a killed run is simply started again.
#
# Verdicts: equivalent (same pages, no text line differs, worst raster <= RASTER_EQ), near (same pages, text
# equal, raster above it), differs, aus_fails, typst_fails (counted apart from the shares). `cause` keys an
# aus_fails row by the construct its diagnostic names, so failures can be ranked (report_parity.py).
#
# Run it detached as one job:
#   systemd-run --user --scope --quiet -p MemoryMax=4G --slice=claude-rc.slice --unit=<name> \
#       python3 -I tools/bench/corpus/parity.py
#
# Environment (defaults): W (~/.cache/austenite/claude-rc-7/parity), BIN (~/.local/bin/austenite), TYPST
# (~/bin/typst), LIST (real_corpus.txt beside this script), OUT ($W/parity.jsonl), TMO (120 s per engine),
# MEM (3G per engine), TIERS ("universe fixture tree"), ONLY (a file of ids, one per line), LIMIT (0: all),
# DPI (50), MAXPAGES (20), RASTER_EQ (0.001).
import difflib
import json
import os
import re
import shutil
import subprocess
import sys
import time
import unicodedata

HERE	= os.path.dirname(os.path.abspath(__file__))
CRATE	= os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
REPO	= os.path.dirname(CRATE)
HOME	= os.path.expanduser("~")
W		= os.environ.get("W", os.path.join(HOME, ".cache/austenite/claude-rc-7/parity"))
BIN		= os.environ.get("BIN", os.path.join(HOME, ".local/bin/austenite"))
TYPST	= os.environ.get("TYPST", os.path.join(HOME, "bin/typst"))
LIST	= os.environ.get("LIST", os.path.join(HERE, "real_corpus.txt"))
OUT		= os.environ.get("OUT", os.path.join(W, "parity.jsonl"))
TMO		= int(os.environ.get("TMO", "120"))
MEM		= os.environ.get("MEM", "3G")
TIERS	= os.environ.get("TIERS", "universe fixture tree").split()
ONLY	= os.environ.get("ONLY", "")
LIMIT	= int(os.environ.get("LIMIT", "0"))
DPI		= os.environ.get("DPI", "50")
MAXP	= int(os.environ.get("MAXPAGES", "20"))
RASTER_EQ	= float(os.environ.get("RASTER_EQ", "0.001"))
os.environ.setdefault("TYPST_PACKAGE_CACHE_PATH", os.path.join(W, "pkgs"))

LOG = os.path.join(W, "parity.log")


def log(msg):
	with open(LOG, "a") as f:
		f.write("%s %s\n" % (time.strftime("%T"), msg))


def capped(cmd, cwd, out_prefix):
	# One engine run under its own memory scope and wall-time cap; returns (exit code, seconds).
	t0 = time.time()
	with open(out_prefix + ".out", "w") as o, open(out_prefix + ".err", "w") as e:
		rc = subprocess.call(
			["systemd-run", "--user", "--scope", "--quiet", "-p", "MemoryMax=" + MEM, "-p", "MemorySwapMax=0",
				"--slice=claude-rc.slice", "timeout", "-k", "5", str(TMO)] + cmd,
			cwd=cwd, stdout=o, stderr=e)
	return rc, round(time.time() - t0, 2)


def read(p, limit=None):
	try:
		with open(p, errors="replace") as f:
			return f.read() if limit is None else f.read(limit)
	except OSError:
		return ""


ANSI = re.compile(r"\x1b\[[0-9;]*m")


def first_error(prefix, rc):
	# The first line that names an error, else the last non-empty line, of stderr then stdout.
	if rc == 124 or rc == 137:
		return "timeout after %d s" % TMO if rc == 124 else "killed (memory cap %s or timeout)" % MEM
	text = ANSI.sub("", read(prefix + ".err") + "\n" + read(prefix + ".out"))
	lines = [l.strip() for l in text.splitlines() if l.strip()]
	for l in lines:
		if re.search(r"\berror\b|panicked|Error|ERR", l):
			return l[:400]
	return (lines[-1] if lines else "")[:400]


def pages(pdf):
	try:
		info = subprocess.run(["pdfinfo", pdf], capture_output=True, text=True, timeout=60).stdout
	except (subprocess.TimeoutExpired, OSError):
		return None
	m = re.search(r"^Pages:\s+(\d+)", info, re.M)
	return int(m.group(1)) if m else None


SPACES = re.compile(r" +")


def text_lines(pdf):
	try:
		t = subprocess.run(["pdftotext", "-layout", pdf, "-"], capture_output=True, text=True,
			errors="replace", timeout=120).stdout
	except (subprocess.TimeoutExpired, OSError):
		return None
	# A page break is a line break; runs of spaces collapse; trailing space is not a difference.
	# Invisible format characters (word joiner, zero-width space, soft hyphen; Unicode category Cf) draw
	# nothing, so one engine emitting them and the other not is not a text difference.
	t = "".join(c for c in t.replace("\f", "\n") if unicodedata.category(c) != "Cf")
	return [SPACES.sub(" ", l).strip() for l in t.split("\n")]


def text_diff(a, b):
	if a is None or b is None:
		return None
	n = 0
	for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
		if tag != "equal":
			n += max(i2 - i1, j2 - j1)
	return n


def raster(pdf, pfx, npages):
	subprocess.run(["pdftoppm", "-r", DPI, "-png", "-l", str(min(npages, MAXP)), pdf, pfx],
		capture_output=True, timeout=300)


def page_pngs(d, pfx):
	out = {}
	for f in os.listdir(d):
		m = re.match(re.escape(pfx) + r"-0*(\d+)\.png$", f)
		if m:
			out[int(m.group(1))] = os.path.join(d, f)
	return out


def png_size(p):
	with open(p, "rb") as f:
		h = f.read(24)
	return int.from_bytes(h[16:20], "big"), int.from_bytes(h[20:24], "big")


def page_diff(a, b):
	# Fraction of pixels differing beyond 10% fuzz; 1.0 when the page sizes differ.
	wa, ha = png_size(a)
	wb, hb = png_size(b)
	if (wa, ha) != (wb, hb):
		return 1.0
	r = subprocess.run(["compare", "-metric", "AE", "-fuzz", "10%", a, b, "null:"],
		capture_output=True, text=True, timeout=120)
	m = re.match(r"\s*([0-9.e+]+)", r.stderr)
	if not m:
		return 1.0
	return float(m.group(1)) / float(wa * ha)


# Cause keys. The evaluator's diagnostic names the construct; the key is the construct, not the site.
CAUSES = [
	(r"bibliography style `?[^`]*`? is not supported", "unsupported: bibliography style (CSL)"),
	(r"type `?([\w.-]+)`? has no method `?([\w.-]+)", "no method: {}.{}"),
	(r"`?([\w.-]+)`? does not contain field `?([\w.-]+)", "no field: {}.{}"),
	(r"(expected [^,]+(?:, [^,]+)*,? found [\w.-]+)", "type mismatch: {}"),
	(r"unknown variable:? `?([\w.-]+)", "unknown variable: {}"),
	(r"unknown function:? `?([\w.-]+)", "unknown function: {}"),
	(r"unknown field:? `?([\w.-]+)`?", "unknown field: {}"),
	(r"(?:type|value) `?([\w.-]+)`? (?:has|does not have) (?:no )?(?:field|method) `?([\w.-]+)", "no field: {}.{}"),
	(r"(?:no|unknown) (?:field|method) `?([\w.-]+)`? (?:on|of|for) `?([\w.-]+)", "no field: {1}.{0}"),
	(r"unsupported element:? `?([\w.-]+)", "unsupported element: {}"),
	(r"element `?([\w.-]+)`? (?:is )?(?:not supported|unsupported)", "unsupported element: {}"),
	(r"unknown (?:argument|parameter):? `?([\w.-]+)", "unknown argument: {}"),
	(r"unexpected argument:? `?([\w.-]+)", "unexpected argument: {}"),
	(r"package `?(@[\w/.:-]+)`?", "package: {}"),
	(r"(?:cannot|could not|failed to) (?:read|open|find|load) (?:file )?`?([^`:\s]+)", "file: {}"),
]


def norm_cause(err, rc):
	if rc == 124:
		return "timeout"
	if rc == 137:
		return "killed (memory or timeout)"
	if "panicked" in err:
		m = re.search(r"panicked at ([^:]+)", err)
		return "panic: %s" % (m.group(1) if m else "?")
	for pat, fmt in CAUSES:
		m = re.search(pat, err, re.I)
		if m:
			g = m.groups()
			name = fmt.format(*g)
			if name.startswith("file: "):
				name = "file: " + os.path.splitext(name[6:])[1]
			return name
	# The error class: the message with its names, numbers and quotes dropped.
	e = re.sub(r"^.*?error:?\s*", "", err, flags=re.I)
	e = re.sub(r"`[^`]*`|\"[^\"]*\"|'[^']*'", "_", e)
	e = re.sub(r"\d+", "N", e)
	e = re.sub(r"\s+at\s+\S+$", "", e)
	return ("class: " + e.strip())[:120]


SITE = re.compile(r"(@[\w-]+/[\w.-]+:[\d.]+)/|(?:^|\s)(/\S+?\.typ):\d+")


def cause_site(err):
	# Where the failing construct sits: a package (name:version) or "document" for the template's own files.
	m = SITE.search(err or "")
	if not m:
		return None
	return m.group(1) if m.group(1) else "document"


def stage(tier, ident, spec, work):
	# Copies the document's tree into work/src; returns (root, source, fonts) or raises RuntimeError.
	src_dir = os.path.join(work, "src")
	if tier == "universe":
		cache = os.path.join(W, "src", "universe", ident.replace(":", "-"))
		if not os.path.isfile(os.path.join(cache, ".staged")):
			shutil.rmtree(cache, ignore_errors=True)
			os.makedirs(os.path.dirname(cache), exist_ok=True)
			r = subprocess.run([TYPST, "init", "@preview/" + ident, cache], capture_output=True, text=True,
				timeout=180)
			if r.returncode != 0:
				shutil.rmtree(cache, ignore_errors=True)
				raise RuntimeError("typst init: " + " ".join(r.stderr.split())[:300])
			open(os.path.join(cache, ".staged"), "w").close()
		shutil.copytree(cache, src_dir, symlinks=True, ignore=shutil.ignore_patterns(".staged"))
		source = os.path.join(src_dir, spec)
	else:
		path = os.path.join(REPO, spec)
		shutil.copytree(os.path.dirname(path), src_dir, symlinks=True, ignore=shutil.ignore_patterns("*.pdf"))
		source = os.path.join(src_dir, os.path.basename(path))
	if not os.path.isfile(source):
		raise RuntimeError("no source " + spec)
	fonts = None
	for dp, _, fs in os.walk(src_dir):
		if any(f.lower().endswith((".ttf", ".otf", ".ttc")) for f in fs):
			fonts = src_dir
			break
	return src_dir, source, fonts


def measure(tier, ident, spec, extra):
	row = {"id": ident, "tier": tier, "spec": spec, "extra": extra}
	work = os.path.join(W, "run", "cur")
	shutil.rmtree(work, ignore_errors=True)
	os.makedirs(work)
	try:
		root, source, fonts = stage(tier, ident, spec, work)
	except (RuntimeError, OSError, subprocess.TimeoutExpired) as e:
		row.update({"typst_ok": False, "typst_rc": None, "typst_err": "stage: " + str(e)[:300],
			"aus_ok": None, "verdict": "typst_fails", "stage_fail": True})
		return row
	fa = ["--font-path", fonts] if fonts else []
	tpdf = os.path.join(work, "t.pdf")
	adir = os.path.join(work, "ao")
	# typst first: it fetches the template's imports into the package cache Austenite then reads.
	trc, tsec = capped([TYPST, "compile", "--ignore-system-fonts", "--root", root] + fa + [source, tpdf],
		os.path.dirname(source), os.path.join(work, "t"))
	arc, asec = capped([BIN, "--eval", "--root", root] + fa + [source, adir], os.path.dirname(source),
		os.path.join(work, "a"))
	apdf = os.path.join(adir, "document.pdf")
	if not os.path.isfile(apdf):
		cands = [f for f in os.listdir(adir) if f.endswith(".pdf")] if os.path.isdir(adir) else []
		apdf = os.path.join(adir, cands[0]) if cands else apdf
	t_ok = trc == 0 and os.path.isfile(tpdf)
	a_ok = arc == 0 and os.path.isfile(apdf)
	row.update({
		"typst_ok": t_ok, "typst_rc": trc, "typst_s": tsec, "typst_err": None if t_ok else first_error(os.path.join(work, "t"), trc),
		"aus_ok": a_ok, "aus_rc": arc, "aus_s": asec, "aus_err": None if a_ok else first_error(os.path.join(work, "a"), arc),
	})
	if not t_ok:
		row["verdict"] = "typst_fails"
		return row
	row["typst_pages"] = pages(tpdf)
	if not a_ok:
		row["verdict"] = "aus_fails"
		row["cause"] = norm_cause(row["aus_err"], arc)
		row["cause_site"] = cause_site(row["aus_err"])
		return row
	row["aus_pages"] = pages(apdf)
	tl = text_lines(tpdf)
	al = text_lines(apdf)
	row["text_lines"] = len(tl) if tl is not None else None
	row["text_diff"] = text_diff(tl, al)
	if tl is not None and al is not None and row["text_diff"]:
		# The first differing pair, for the report's sample.
		for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, tl, al, autojunk=False).get_opcodes():
			if tag != "equal":
				row["text_first"] = {"line": i1 + 1, "typst": " | ".join(tl[i1:i2][:2])[:200],
					"aus": " | ".join(al[j1:j2][:2])[:200]}
				break
	worst, worst_p, npages = None, None, min(row["typst_pages"] or 0, row["aus_pages"] or 0)
	if npages:
		raster(tpdf, os.path.join(work, "t"), npages)
		raster(apdf, os.path.join(work, "a"), npages)
		tp = page_pngs(work, "t")
		ap = page_pngs(work, "a")
		for p in sorted(tp):
			if p in ap:
				try:
					d = page_diff(tp[p], ap[p])
				except (OSError, subprocess.TimeoutExpired):
					d = 1.0
				if worst is None or d > worst:
					worst, worst_p = d, p
	row["raster_worst"] = None if worst is None else round(worst, 6)
	row["raster_page"] = worst_p
	same = row["typst_pages"] == row["aus_pages"] and row["typst_pages"] is not None
	if same and row["text_diff"] == 0 and worst is not None and worst <= RASTER_EQ:
		row["verdict"] = "equivalent"
	elif same and row["text_diff"] == 0:
		row["verdict"] = "near"
	else:
		row["verdict"] = "differs"
	if row["verdict"] == "differs" and worst_p is not None:
		keep = os.path.join(W, "keep", re.sub(r"[^\w.-]", "_", ident))
		os.makedirs(keep, exist_ok=True)
		for pfx, pp in (("t", tp), ("a", ap)):
			shutil.copy(pp[worst_p], os.path.join(keep, "%s-%d.png" % (pfx, worst_p)))
	return row


def main():
	os.makedirs(W, exist_ok=True)
	os.makedirs(os.environ["TYPST_PACKAGE_CACHE_PATH"], exist_ok=True)
	done = set()
	if os.path.isfile(OUT):
		for l in open(OUT):
			try:
				done.add(json.loads(l)["id"])
			except (ValueError, KeyError):
				pass
	want = None
	if ONLY:
		want = set(x.strip() for x in open(ONLY) if x.strip())
	tv = subprocess.run([TYPST, "--version"], capture_output=True, text=True).stdout.strip()
	log("parity start: list %s, out %s, bin %s, %s" % (LIST, OUT, BIN, tv))
	n = 0
	for line in open(LIST):
		f = line.rstrip("\n").split("\t")
		if not f[0] or f[0].startswith("#") or len(f) < 3 or f[0] not in TIERS:
			continue
		tier, ident, spec = f[0], f[1], f[2]
		extra = f[3] if len(f) > 3 else ""
		if want is not None and ident not in want:
			continue
		if ident in done:
			continue
		if LIMIT and n >= LIMIT:
			break
		n += 1
		t0 = time.time()
		try:
			row = measure(tier, ident, spec, extra)
		except Exception as e:
			row = {"id": ident, "tier": tier, "spec": spec, "verdict": "harness_error", "harness_err": repr(e)[:300]}
		with open(OUT, "a") as o:
			o.write(json.dumps(row) + "\n")
		log("%d %s %s %s %.1fs" % (n, tier, ident, row.get("verdict"), time.time() - t0))
		shutil.rmtree(os.path.join(W, "run", "cur"), ignore_errors=True)
	log("parity end: %d documents" % n)


if __name__ == "__main__":
	main()
