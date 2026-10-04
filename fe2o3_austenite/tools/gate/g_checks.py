#!/usr/bin/env python3
"""The gate's checks: do two PDFs, one Typst's and one Austenite's, agree?

    g_checks.py compare <a.pdf> <b.pdf>          G3, the text of every page (also `g3`)
    g_checks.py pages <x.pdf>                    the per-page hashes of G3, for one PDF
    g_checks.py g1  <a.pdf> <b.pdf>              page count and each page's MediaBox
    g_checks.py g2  <a.pdf> <b.pdf>              the set of font base names, and that every font is embedded
                                                 and has a Unicode map
    g_checks.py g4  <a.pdf> <b.pdf>              outline count, level sequence and the titles
    g_checks.py g5  <a.pdf> <b.pdf>              Title, Author, Subject and Keywords
    g_checks.py g7  <a.pdf> <b.pdf>              pages rasterised at 50 dpi, the fraction of pixels that differ
                                                 (needs GATE_SCRATCH, a directory that exists)
    g_checks.py g8  <a.pdf> <b.pdf>              the character count of each page, within 1%
    g_checks.py g12 <cli1> <cli2> <door1> <door2> <door3>
                                                 two CLI PDFs, two from one door instance, one from another
    g_checks.py g9  <a.pdf> <b.pdf>              where the text sits on each page: lines, blocks, words, extents,
                                                 line pitch, word height, images and links, then line by line

`a` is the oracle's PDF (Typst's) and `b` the PDF under test. G9 is measured, with no verdict: it carries no word of
a page, only where the words sit, from `pdftotext -bbox-layout`. In `compare` and `pages` a page is its `pdftotext
-raw` text, normalised to NFKC and split on whitespace, with the tokens that are only punctuation dropped. Page i
of one PDF is compared with page i of the other by the 12-hex hash of its first token, the 12-hex hash of its
last, and the difflib ratio over its token lists. Text is read from a pipe and never written to disk, and
nothing of a document is printed: every line is built from numbers, hashes and the fixed words below, so the
output is safe for a document whose text must not be logged. A font name is printed only when it matches
`[A-Za-z0-9+-]+`; the pages of G7 are rasterised into a directory made under GATE_SCRATCH and removed at once.

Every check ends in a `verdict` line and exits 0 for `same`, 1 for `differs`; on any failure it prints
`g<N> error <unreadable|usage|scratch>` and exits 2.

    g3 pages a <n> b <n>
    g3 differ <n> pages <ranges|none>     (pages whose first or last hash differs, 1-based, e.g. 3-5,9)
    g3 min-similarity <d.ddd|none>        (over the pages both have)
    g3 verdict <same|differs>             (differs on a count, a hash or a similarity below 1)
    g3 page <i> first <hex12> last <hex12> tokens <n>                     (`pages`, one per page)

    g1 pages a <n> b <n>
    g1 mediabox differ <n> pages <ranges|none>     (a side is within 0.01 pt of the other on all four numbers)
    g1 mediabox max-deviation <d.dddd>             (pt, over the pages both have)
    g1 verdict <same|differs>

    g2 fonts a <n> b <n>
    g2 only-a <name>                      (a base name, subset tag and `-Identity-H` stripped, one per font)
    g2 only-b <name>
    g2 flags a <n> b <n>                  (fonts that are not `emb yes` and `uni yes`)
    g2 verdict <same|differs>

    g4 outline a <n> b <n>                (entries, every level)
    g4 levels <equal|differs> first-diff <i|none>      (the depth of each entry in document order)
    g4 titles <equal|differs> unmatched a <n> b <n>    (the multiset of title sha256)
    g4 titles position differ <n> of <m>               (title i against title i)
    g4 titles prefix-only <n>             (of those, one is the other after a leading number or label)
    g4 verdict <same|differs>             (the count, the levels and every title at its place)

    g5 <title|author|subject|keywords> <equal|differs|absent-both>   (by sha256 of the Info entry)
    g5 verdict <same|differs>

    g7 compared <n> of a <n> b <n>
    g7 page <i> <d.dddd>                  (the fraction of pixels that differ by more than 16/255 in a channel)
    g7 max <d.dddd>
    g7 median <d.dddd>

    g8 pages a <n> b <n>
    g8 out-of-tolerance <n> pages <ranges|none>
    g8 page <i> a <n> b <n>               (each page out of tolerance: its counts of non-space characters)
    g8 max-deviation <d.dddd>             (|b - a| / a, with 1.0000 where a is 0 and b is not)
    g8 empty <n> pages <ranges|none>      (b has no text where a has some)
    g8 verdict <same|differs>

    g9 pages differ <n> pages <ranges|none>          (a count or an extent below differs)
    g9 page <i> <lines|blocks|words|sizes|images|links> a <n> b <n>        (counts; sizes is the distinct word heights)
    g9 page <i> <left|right|top|bottom|pitch|height> a <d.dd> b <d.dd>     (pt; pitch is the median gap between lines
                                                 of a block, height the median word height; 0.5 pt, pitch 0.1 pt)
    g9 line <i> <j> y a <d.dd> b <d.dd> x a <d.dd> b <d.dd> w a <d.dd> b <d.dd>
                                                 (line j of page i where its top, left or width differs by 0.5 pt;
                                                 the first 40 such lines of each of the first 3 differing pages)

    g12 cli-twice <equal|differs|absent>
    g12 door-one-instance <equal|differs|absent>
    g12 door-two-instances <equal|differs|absent>
    g12 cli-door <equal|differs|absent>   (reported, not required)
    g12 size cli <n> door <n>
    g12 verdict <same|differs>            (the first three; an absent file differs)
"""

import difflib
import glob
import hashlib
import os
import re
import statistics
import subprocess
import sys
import tempfile
import unicodedata

NONE = '0' * 12  # the hash of the first or last token of a page with no tokens


def page_texts(pdf):
	"""The text of each page of `pdf`, from pdftotext's pipe."""
	r = subprocess.run(['pdftotext', '-raw', pdf, '-'], capture_output=True)
	if r.returncode != 0:
		raise OSError('pdftotext')
	pages = r.stdout.decode('utf-8', 'replace').split('\f')
	# pdftotext ends every page, the last included, with a form feed, so one empty piece trails.
	if pages and pages[-1] == '':
		pages.pop()
	return pages


def is_punct(tok):
	return all(unicodedata.category(c).startswith('P') for c in tok)


def tokens(text):
	"""NFKC, split on whitespace, the tokens that are only punctuation dropped."""
	return [t for t in unicodedata.normalize('NFKC', text).split() if not is_punct(t)]


def h12(tok):
	return hashlib.sha256(tok.encode('utf-8')).hexdigest()[:12]


def summary(toks):
	"""The hashes of a page's first and last token."""
	if not toks:
		return NONE, NONE
	return h12(toks[0]), h12(toks[-1])


def ratio(ta, tb):
	if not ta and not tb:
		return 1.0
	return difflib.SequenceMatcher(None, ta, tb, autojunk=False).ratio()


def ranges(idx):
	"""1-based indices as `3-5,9`, or `none`."""
	if not idx:
		return 'none'
	out = []
	start = prev = idx[0]
	for i in idx[1:] + [None]:
		if i is not None and i == prev + 1:
			prev = i
			continue
		out.append(str(start) if start == prev else '%d-%d' % (start, prev))
		if i is not None:
			start = prev = i
	return ','.join(out)


def compare_pages(pa, pb):
	"""The pages of `pa` and `pb`, each a list of tokens, compared page for page.

	Returns (differing 1-based page indices, minimum similarity or None).
	"""
	n = min(len(pa), len(pb))
	differ = []
	low = None
	for i in range(n):
		a, b = pa[i], pb[i]
		if summary(a) != summary(b):
			differ.append(i + 1)
		r = ratio(a, b)
		low = r if low is None else min(low, r)
	return differ, low


def cmd_compare(a_pdf, b_pdf):
	pa = [tokens(t) for t in page_texts(a_pdf)]
	pb = [tokens(t) for t in page_texts(b_pdf)]
	differ, low = compare_pages(pa, pb)
	print('g3 pages a %d b %d' % (len(pa), len(pb)))
	print('g3 differ %d pages %s' % (len(differ), ranges(differ)))
	print('g3 min-similarity %s' % ('none' if low is None else '%.3f' % low))
	same = len(pa) == len(pb) and not differ and (low is None or low >= 1.0)
	print('g3 verdict %s' % ('same' if same else 'differs'))
	return 0 if same else 1


def cmd_pages(pdf):
	for i, t in enumerate(page_texts(pdf)):
		toks = tokens(t)
		first, last = summary(toks)
		print('g3 page %d first %s last %s tokens %d' % (i + 1, first, last, len(toks)))
	return 0


# ── G1, G2, G4, G5, G7, G8 and G12 ───────────────────────────────────────────

def pdf_open(path):
	import pikepdf
	return pikepdf.open(path)


def mediaboxes(path):
	"""The MediaBox of each page, as four floats."""
	with pdf_open(path) as pdf:
		return [[float(x) for x in page.mediabox] for page in pdf.pages]


def cmd_g1(a_pdf, b_pdf):
	ma = mediaboxes(a_pdf)
	mb = mediaboxes(b_pdf)
	n = min(len(ma), len(mb))
	differ = []
	worst = 0.0
	for i in range(n):
		d = max(abs(x - y) for x, y in zip(ma[i], mb[i]))
		worst = max(worst, d)
		if d >= 0.01:
			differ.append(i + 1)
	print('g1 pages a %d b %d' % (len(ma), len(mb)))
	print('g1 mediabox differ %d pages %s' % (len(differ), ranges(differ)))
	print('g1 mediabox max-deviation %.4f' % min(worst, 99999.9999))
	same = len(ma) == len(mb) and not differ
	print('g1 verdict %s' % ('same' if same else 'differs'))
	return 0 if same else 1


FONT_TAG = re.compile(r'^[A-Z]{6}\+')
FONT_NAME = re.compile(r'^[A-Za-z0-9+-]{1,64}$')


def base_name(name):
	"""A font's name without its subset tag and without `-Identity-H`."""
	name = FONT_TAG.sub('', name)
	if name.endswith('-Identity-H'):
		name = name[:-len('-Identity-H')]
	return name


def fonts_of(pdf):
	"""(base name, is it embedded and does it have a Unicode map) for each font `pdffonts` lists."""
	r = subprocess.run(['pdffonts', pdf], capture_output=True)
	if r.returncode != 0:
		raise OSError('pdffonts')
	out = []
	for line in r.stdout.decode('utf-8', 'replace').splitlines()[2:]:
		t = line.split()
		# name, type (one to three words), encoding, emb, sub, uni, object number, generation.
		if len(t) < 8:
			continue
		out.append((base_name(t[0]), t[-5] == 'yes' and t[-3] == 'yes'))
	return out


def shown(name):
	return name if FONT_NAME.match(name) else 'other'


def cmd_g2(a_pdf, b_pdf):
	fa = fonts_of(a_pdf)
	fb = fonts_of(b_pdf)
	sa = {n for n, _ in fa}
	sb = {n for n, _ in fb}
	flags_a = sum(1 for _, ok in fa if not ok)
	flags_b = sum(1 for _, ok in fb if not ok)
	print('g2 fonts a %d b %d' % (len(sa), len(sb)))
	for n in sorted(sa - sb)[:64]:
		print('g2 only-a %s' % shown(n))
	for n in sorted(sb - sa)[:64]:
		print('g2 only-b %s' % shown(n))
	print('g2 flags a %d b %d' % (flags_a, flags_b))
	same = sa == sb and flags_a == 0 and flags_b == 0
	print('g2 verdict %s' % ('same' if same else 'differs'))
	return 0 if same else 1


def outline_entries(path):
	"""(depth, title) for every outline entry in document order; depth 1 is the top."""
	out = []
	with pdf_open(path) as pdf:
		top = pdf.Root.get('/Outlines')
		if top is None:
			return out
		seen = set()

		def walk(item, depth):
			while item is not None:
				key = item.objgen
				if key != (0, 0):
					if key in seen:
						return
					seen.add(key)
				title = item.get('/Title')
				out.append((depth, '' if title is None else str(title)))
				child = item.get('/First')
				if child is not None:
					walk(child, depth + 1)
				item = item.get('/Next')

		walk(top.get('/First'), 1)
	return out


def sha(text):
	return hashlib.sha256(text.encode('utf-8')).hexdigest()


def prefixed(x, y):
	"""Is `x` just `y` after a leading number or label and a space?"""
	if not y or x == y or not x.endswith(y):
		return False
	return x[:len(x) - len(y)][-1:].isspace()


def cmd_g4(a_pdf, b_pdf):
	ea = outline_entries(a_pdf)
	eb = outline_entries(b_pdf)
	la = [d for d, _ in ea]
	lb = [d for d, _ in eb]
	first = None
	for i in range(min(len(la), len(lb))):
		if la[i] != lb[i]:
			first = i + 1
			break
	if first is None and len(la) != len(lb):
		first = min(len(la), len(lb)) + 1
	ca = {}
	cb = {}
	for _, t in ea:
		ca[sha(t)] = ca.get(sha(t), 0) + 1
	for _, t in eb:
		cb[sha(t)] = cb.get(sha(t), 0) + 1
	miss_a = sum(max(0, n - cb.get(k, 0)) for k, n in ca.items())
	miss_b = sum(max(0, n - ca.get(k, 0)) for k, n in cb.items())
	n = min(len(ea), len(eb))
	pos = 0
	pre = 0
	for i in range(n):
		ta, tb = ea[i][1], eb[i][1]
		if ta != tb:
			pos += 1
			if prefixed(ta, tb) or prefixed(tb, ta):
				pre += 1
	print('g4 outline a %d b %d' % (len(ea), len(eb)))
	print('g4 levels %s first-diff %s' % ('equal' if first is None else 'differs', 'none' if first is None else first))
	titles_equal = miss_a == 0 and miss_b == 0
	print('g4 titles %s unmatched a %d b %d' % ('equal' if titles_equal else 'differs', miss_a, miss_b))
	print('g4 titles position differ %d of %d' % (pos, n))
	print('g4 titles prefix-only %d' % pre)
	same = len(ea) == len(eb) and first is None and titles_equal and pos == 0
	print('g4 verdict %s' % ('same' if same else 'differs'))
	return 0 if same else 1


INFO_KEYS = (('title', '/Title'), ('author', '/Author'), ('subject', '/Subject'), ('keywords', '/Keywords'))


def info_of(path):
	"""The sha256 of each Info entry among Title, Author, Subject and Keywords, or None where it is absent."""
	out = {}
	with pdf_open(path) as pdf:
		info = pdf.trailer.get('/Info')
		for word, key in INFO_KEYS:
			v = None if info is None else info.get(key)
			out[word] = None if v is None else sha(str(v))
	return out


def cmd_g5(a_pdf, b_pdf):
	ia = info_of(a_pdf)
	ib = info_of(b_pdf)
	same = True
	for word, _ in INFO_KEYS:
		x, y = ia[word], ib[word]
		if x is None and y is None:
			v = 'absent-both'
		elif x == y:
			v = 'equal'
		else:
			v = 'differs'
			same = False
		print('g5 %s %s' % (word, v))
	print('g5 verdict %s' % ('same' if same else 'differs'))
	return 0 if same else 1


def read_ppm(path):
	import numpy
	with open(path, 'rb') as f:
		data = f.read()
	m = re.match(rb'P6\s+(\d+)\s+(\d+)\s+(\d+)\s', data)
	if not m:
		raise OSError('ppm')
	w, h = int(m.group(1)), int(m.group(2))
	return numpy.frombuffer(data, dtype=numpy.uint8, offset=m.end()).reshape(h, w, 3)


def rasterise(pdf, root):
	"""The paths of the pages of `pdf` at 50 dpi, in page order."""
	r = subprocess.run(['pdftoppm', '-r', '50', pdf, root], capture_output=True)
	if r.returncode != 0:
		raise OSError('pdftoppm')
	found = {}
	for f in glob.glob(glob.escape(root) + '-*.ppm'):
		m = re.search(r'-(\d+)\.ppm$', f)
		if m:
			found[int(m.group(1))] = f
	return [found[k] for k in sorted(found)]


def cmd_g7(a_pdf, b_pdf):
	import numpy
	base = os.environ.get('GATE_SCRATCH', '')
	if not base or not os.path.isdir(base):
		print('g7 error scratch')
		return 2
	with tempfile.TemporaryDirectory(dir=base) as d:
		pa = rasterise(a_pdf, os.path.join(d, 'a'))
		pb = rasterise(b_pdf, os.path.join(d, 'b'))
		n = min(len(pa), len(pb))
		fr = []
		for i in range(n):
			ra, rb = read_ppm(pa[i]), read_ppm(pb[i])
			if ra.shape != rb.shape:
				fr.append(1.0)
			else:
				diff = numpy.abs(ra.astype(numpy.int16) - rb.astype(numpy.int16)).max(axis=2)
				fr.append(float((diff > 16).mean()))
			os.remove(pa[i])
			os.remove(pb[i])
	print('g7 compared %d of a %d b %d' % (n, len(pa), len(pb)))
	for i, f in enumerate(fr):
		print('g7 page %d %.4f' % (i + 1, f))
	print('g7 max %s' % ('none' if not fr else '%.4f' % max(fr)))
	print('g7 median %s' % ('none' if not fr else '%.4f' % statistics.median(fr)))
	return 0


def char_counts(pdf):
	"""The count of non-space characters on each page, after NFKC."""
	return [len(''.join(unicodedata.normalize('NFKC', t).split())) for t in page_texts(pdf)]


def cmd_g8(a_pdf, b_pdf):
	ca = char_counts(a_pdf)
	cb = char_counts(b_pdf)
	n = min(len(ca), len(cb))
	out = []
	empty = []
	worst = 0.0
	for i in range(n):
		a, b = ca[i], cb[i]
		dev = 0.0 if a == b else (1.0 if a == 0 else abs(b - a) / a)
		worst = max(worst, dev)
		if dev > 0.01:
			out.append(i + 1)
		if a > 0 and b == 0:
			empty.append(i + 1)
	print('g8 pages a %d b %d' % (len(ca), len(cb)))
	print('g8 out-of-tolerance %d pages %s' % (len(out), ranges(out)))
	for i in out[:64]:
		print('g8 page %d a %d b %d' % (i, ca[i - 1], cb[i - 1]))
	print('g8 max-deviation %.4f' % min(worst, 9.9999))
	print('g8 empty %d pages %s' % (len(empty), ranges(empty)))
	same = len(ca) == len(cb) and not out and not empty
	print('g8 verdict %s' % ('same' if same else 'differs'))
	return 0 if same else 1


def digest(path):
	"""The sha256 of a file's bytes and its length, or None for a file that is absent or empty."""
	try:
		with open(path, 'rb') as f:
			data = f.read()
	except OSError:
		return None
	return (hashlib.sha256(data).hexdigest(), len(data)) if data else None


def relation(x, y):
	if x is None or y is None:
		return 'absent'
	return 'equal' if x[0] == y[0] else 'differs'


def cmd_g12(cli1, cli2, door1, door2, door3):
	c1, c2, d1, d2, d3 = (digest(p) for p in (cli1, cli2, door1, door2, door3))
	rows = (
		('cli-twice',			relation(c1, c2)),
		('door-one-instance',	relation(d1, d2)),
		('door-two-instances',	relation(d1, d3)),
	)
	for word, v in rows:
		print('g12 %s %s' % (word, v))
	print('g12 cli-door %s' % relation(c1, d1))
	print('g12 size cli %s door %s' % ('none' if c1 is None else c1[1], 'none' if d1 is None else d1[1]))
	same = all(v == 'equal' for _, v in rows)
	print('g12 verdict %s' % ('same' if same else 'differs'))
	return 0 if same else 1


# ── G9 ──────────────────────────────────────────────────────────────────────

TAG_PAGE	= re.compile(rb'<page\b')
TAG_BLOCK	= re.compile(rb'<block\b[^>]*>')
TAG_LINE	= re.compile(rb'<line\b[^>]*>')
TAG_WORD	= re.compile(rb'<word\b[^>]*>')
ATTR		= re.compile(rb'(xMin|yMin|xMax|yMax)="([-0-9.]+)"')


def boxes(tag):
	"""(xMin, yMin, xMax, yMax) of one tag, from its own attributes."""
	a = {k: float(v) for k, v in ATTR.findall(tag)}
	return a[b'xMin'], a[b'yMin'], a[b'xMax'], a[b'yMax']


def layout_pages(pdf):
	"""Per page, a dict of counts and extents and the list of its lines, from `pdftotext -bbox-layout`."""
	r = subprocess.run(['pdftotext', '-bbox-layout', pdf, '-'], capture_output=True)
	if r.returncode != 0:
		raise OSError('pdftotext')
	pages = []
	for chunk in TAG_PAGE.split(r.stdout)[1:]:
		blocks = TAG_BLOCK.split(chunk)
		lines = [boxes(t) for t in TAG_LINE.findall(chunk)]
		heights = [b[3] - b[1] for b in (boxes(t) for t in TAG_WORD.findall(chunk))]
		# The pitch: the median gap from one line to the next in reading order, leaving out the gaps of
		# more than two and a half word heights, which are spaces between paragraphs and blocks.
		hm = statistics.median(heights) if heights else 0.0
		gaps = [b[1] - a[1] for a, b in zip(lines, lines[1:]) if 0 < b[1] - a[1] <= 2.5 * hm]
		pages.append({
			'lines':	len(lines),
			'blocks':	len(blocks) - 1,
			'words':	len(heights),
			'sizes':	len({round(h * 2) for h in heights}),
			'left':		min((b[0] for b in lines), default=0.0),
			'right':	max((b[2] for b in lines), default=0.0),
			'top':		min((b[1] for b in lines), default=0.0),
			'bottom':	max((b[3] for b in lines), default=0.0),
			'pitch':	statistics.median(gaps) if gaps else 0.0,
			'height':	statistics.median(heights) if heights else 0.0,
			'rows':		lines,
		})
	return pages


def counts_per_page(pdf, n):
	"""(images, links) on each of the `n` pages."""
	r = subprocess.run(['pdfimages', '-list', pdf], capture_output=True)
	imgs = [0] * n
	if r.returncode == 0:
		for line in r.stdout.decode('utf-8', 'replace').splitlines()[2:]:
			t = line.split()
			if t and t[0].isdigit() and 1 <= int(t[0]) <= n:
				imgs[int(t[0]) - 1] += 1
	links = []
	with pdf_open(pdf) as doc:
		for page in doc.pages:
			ann = page.obj.get('/Annots')
			links.append(0 if ann is None else sum(1 for x in ann if str(x.get('/Subtype')) == '/Link'))
	return imgs, links


COUNTS	= ('lines', 'blocks', 'words', 'sizes')
EXTENTS	= ('left', 'right', 'top', 'bottom', 'height')


def cmd_g9(a_pdf, b_pdf):
	pa = layout_pages(a_pdf)
	pb = layout_pages(b_pdf)
	ia, la = counts_per_page(a_pdf, len(pa))
	ib, lb = counts_per_page(b_pdf, len(pb))
	n = min(len(pa), len(pb))
	rows = []		# (page, [(metric, a, b)]) for each page that differs
	for i in range(n):
		a, b = pa[i], pb[i]
		a['images'], a['links'], b['images'], b['links'] = ia[i], la[i], ib[i], lb[i]
		d = []
		for m in COUNTS + ('images', 'links'):
			if a[m] != b[m]:
				d.append((m, a[m], b[m], '%d'))
		for m in EXTENTS:
			if abs(a[m] - b[m]) > 0.5:
				d.append((m, a[m], b[m], '%.2f'))
		if abs(a['pitch'] - b['pitch']) > 0.1:
			d.append(('pitch', a['pitch'], b['pitch'], '%.2f'))
		if d:
			rows.append((i, d))
	idx = [i + 1 for i, _ in rows]
	print('g9 pages differ %d pages %s' % (len(idx), ranges(idx)))
	for i, d in rows[:12]:
		for m, x, y, f in d:
			print(('g9 page %d %s a ' + f + ' b ' + f) % (i + 1, m, x, y))
	for i, _ in rows[:3]:
		shown = 0
		for j, (u, v) in enumerate(zip(pa[i]['rows'], pb[i]['rows'])):
			if max(abs(u[1] - v[1]), abs(u[0] - v[0]), abs((u[2] - u[0]) - (v[2] - v[0]))) > 0.5:
				print('g9 line %d %d y a %.2f b %.2f x a %.2f b %.2f w a %.2f b %.2f'
					% (i + 1, j + 1, u[1], v[1], u[0], v[0], u[2] - u[0], v[2] - v[0]))
				shown += 1
				if shown >= 40:
					break
	return 0


CHECKS = {
	'compare':	(2, cmd_compare, 'g3'),
	'g3':		(2, cmd_compare, 'g3'),
	'pages':	(1, cmd_pages, 'g3'),
	'g1':		(2, cmd_g1, 'g1'),
	'g2':		(2, cmd_g2, 'g2'),
	'g4':		(2, cmd_g4, 'g4'),
	'g5':		(2, cmd_g5, 'g5'),
	'g7':		(2, cmd_g7, 'g7'),
	'g8':		(2, cmd_g8, 'g8'),
	'g12':		(5, cmd_g12, 'g12'),
	'g9':		(2, cmd_g9, 'g9'),
}


def main(argv):
	check = CHECKS.get(argv[1]) if len(argv) > 1 else None
	if check is None:
		print('g3 error usage')
		return 2
	if len(argv) != 2 + check[0]:
		print('%s error usage' % check[2])
		return 2
	try:
		return check[1](*argv[2:])
	except Exception:
		# No message and no traceback: either could carry a path or a line of the document.
		print('%s error unreadable' % check[2])
		return 2


if __name__ == '__main__':
	sys.exit(main(sys.argv))
