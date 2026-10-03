#!/usr/bin/env python3
"""G3 of the gate checks: do two PDFs say the same thing on every page?

    g_checks.py compare <a.pdf> <b.pdf>
    g_checks.py pages <x.pdf>

A page is its `pdftotext -raw` text, normalised to NFKC and split on whitespace, with the tokens that
are only punctuation dropped. Page i of one PDF is compared with page i of the other by the 12-hex hash of
its first token, the 12-hex hash of its last, and the difflib ratio over its token lists. The text is read
from the pipe and never written to disk, and nothing of it is printed: every line is built from numbers,
hashes and the fixed words below, so the output is safe for a document whose text must not be logged.

`compare` prints, then exits 0 when the PDFs agree on every page and 1 when they do not:
    g3 pages a <n> b <n>
    g3 differ <n> pages <ranges|none>     (pages whose first or last hash differs, 1-based, e.g. 3-5,9)
    g3 min-similarity <d.ddd|none>        (over the pages both have)
    g3 verdict <same|differs>             (differs on a count, a hash or a similarity below 1)
`pages` prints one `g3 page <i> first <hex12> last <hex12> tokens <n>` per page.
On any failure: `g3 error <unreadable|usage>`, exit 2.
"""

import difflib
import hashlib
import subprocess
import sys
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


def main(argv):
	try:
		if len(argv) == 4 and argv[1] == 'compare':
			return cmd_compare(argv[2], argv[3])
		if len(argv) == 3 and argv[1] == 'pages':
			return cmd_pages(argv[2])
		print('g3 error usage')
		return 2
	except Exception:
		# No message and no traceback: either could carry a path or a line of the document.
		print('g3 error unreadable')
		return 2


if __name__ == '__main__':
	sys.exit(main(sys.argv))
