#!/usr/bin/env python3
"""Size of a wasm binary, taken apart the way the Austenite size metric needs it.

For each `.wasm` given: raw bytes, gzip -9 and brotli (quality 11); the byte split between the
code, data and custom sections; the bytes of every font embedded in the data segments, found by
walking each sfnt table directory rather than by guessing; and, when the binary keeps its `name`
section, code size attributed to Rust modules.

    tools/wasm_measure.py pkg/oxedyne_fe2o3_austenite_bg.wasm
    tools/wasm_measure.py --compare ~/.../typst_ts_web_compiler_bg.wasm pkg/..._bg.wasm
    tools/wasm_measure.py --limit-mb 10 --json out.json pkg/..._bg.wasm

The comparison is like for like: typst.ts ships its fonts separately, so every row is also given
as total minus embedded fonts.  `--limit-mb` is the proposed pass line (owner decision 1): the
first binary's raw size must not exceed it, and the exit status says whether it did.

A release build strips the `name` section, so attribution needs a measurement build that keeps
it, for instance `wasm-pack build --profiling --target web --features wasm` or a raw
`cargo build --release --target wasm32-unknown-unknown --features wasm`.  Without names the
attribution table is omitted and the report says so.

Standard library only: no dependency is installed for this.  Brotli is reported when the
`brotli` module is importable and marked unavailable otherwise, never estimated.
"""

import argparse
import gzip
import json
import os
import sys

try:
	import brotli	# optional; the size is reported as unavailable without it
except ImportError:
	brotli = None

SECTION_NAMES = {
	0: 'custom', 1: 'type', 2: 'import', 3: 'function', 4: 'table', 5: 'memory', 6: 'global',
	7: 'export', 8: 'start', 9: 'element', 10: 'code', 11: 'data', 12: 'datacount', 13: 'tag',
}

SFNT_MAGICS = (b'\x00\x01\x00\x00', b'OTTO', b'true', b'typ1')


class Reader:
	def __init__(self, buf, pos=0, end=None):
		self.buf = buf
		self.pos = pos
		self.end = len(buf) if end is None else end

	def byte(self):
		if self.pos >= self.end:
			raise ValueError('read past the end at byte %d' % self.pos)
		b = self.buf[self.pos]
		self.pos += 1
		return b

	def uleb(self):
		result = 0
		shift = 0
		while True:
			b = self.byte()
			result |= (b & 0x7f) << shift
			if b & 0x80 == 0:
				return result
			shift += 7

	def sleb(self):
		result = 0
		shift = 0
		while True:
			b = self.byte()
			result |= (b & 0x7f) << shift
			shift += 7
			if b & 0x80 == 0:
				if b & 0x40:
					result -= 1 << shift
				return result

	def bytes(self, n):
		if self.pos + n > self.end:
			raise ValueError('a %d-byte field overruns its section at byte %d' % (n, self.pos))
		out = self.buf[self.pos:self.pos + n]
		self.pos += n
		return out

	def name(self):
		n = self.uleb()
		return self.bytes(n).decode('utf-8', 'replace')

	def const_expr(self):
		# The offset expression of an active data segment: a constant or a global read, then `end`.
		# Returns the constant, or None when the offset is not a plain constant.
		value = None
		while True:
			op = self.byte()
			if op == 0x0b:
				return value
			if op in (0x41, 0x42):
				value = self.sleb()
			elif op == 0x23:
				self.uleb()
			elif op == 0x44:
				self.bytes(8)
			elif op == 0x43:
				self.bytes(4)
			else:
				raise ValueError('unexpected opcode 0x%02x in a constant expression' % op)


def sections(buf):
	if buf[:4] != b'\x00asm':
		raise ValueError('not a wasm binary (no \\0asm magic)')
	r = Reader(buf, 8)
	out = []
	while r.pos < r.end:
		sid = r.byte()
		size = r.uleb()
		start = r.pos
		name = SECTION_NAMES.get(sid, 'unknown%d' % sid)
		if sid == 0:
			sub = Reader(buf, start, start + size)
			name = 'custom:' + sub.name()
		out.append((sid, name, start, size))
		r.pos = start + size
	return out


def imported_functions(buf, start, size):
	r = Reader(buf, start, start + size)
	count = 0
	for _ in range(r.uleb()):
		r.name()
		r.name()
		kind = r.byte()
		if kind == 0x00:
			r.uleb()
			count += 1
		elif kind == 0x01:
			r.byte()
			limits(r)
		elif kind == 0x02:
			limits(r)
		elif kind == 0x03:
			r.byte()
			r.byte()
		elif kind == 0x04:
			r.byte()
			r.uleb()
		else:
			raise ValueError('unknown import kind %d' % kind)
	return count


def limits(r):
	flags = r.byte()
	r.uleb()
	if flags & 0x01:
		r.uleb()


def code_bodies(buf, start, size):
	r = Reader(buf, start, start + size)
	out = []
	for _ in range(r.uleb()):
		n = r.uleb()
		out.append(n)
		r.pos += n
	return out


def data_segments(buf, start, size):
	r = Reader(buf, start, start + size)
	out = []
	for _ in range(r.uleb()):
		flags = r.uleb()
		addr = None
		if flags == 0:
			addr = r.const_expr()
		elif flags == 2:
			r.uleb()
			addr = r.const_expr()
		n = r.uleb()
		out.append((r.pos, n, addr))
		r.pos += n
	return out


def function_names(buf, start, size):
	r = Reader(buf, start, start + size)
	r.name()
	names = {}
	while r.pos < r.end:
		sub = r.byte()
		n = r.uleb()
		end = r.pos + n
		if sub == 1:
			for _ in range(r.uleb()):
				idx = r.uleb()
				names[idx] = r.name()
		r.pos = end
	return names


def sfnt_length(buf, at, end):
	"""The byte length of an sfnt font starting at `at`, or None when the bytes there are not one.

	The table directory is checked, not trusted: a table count in a sane range, printable tags,
	and a `head` table present; the length is the furthest table end, padded to four bytes."""
	if at + 12 > end:
		return None
	num = int.from_bytes(buf[at + 4:at + 6], 'big')
	if num < 4 or num > 64 or at + 12 + 16 * num > end:
		return None
	furthest = 0
	tags = set()
	for i in range(num):
		rec = at + 12 + 16 * i
		tag = buf[rec:rec + 4]
		if not all(0x20 <= c < 0x7f for c in tag):
			return None
		tags.add(bytes(tag))
		off = int.from_bytes(buf[rec + 8:rec + 12], 'big')
		ln = int.from_bytes(buf[rec + 12:rec + 16], 'big')
		if off < 12 + 16 * num or at + off + ln > end:
			return None
		furthest = max(furthest, off + ln)
	if b'head' not in tags or not (b'cmap' in tags or b'CFF ' in tags or b'glyf' in tags):
		return None
	return (furthest + 3) & ~3


def memory_image(buf, segments):
	"""The initial linear memory the active segments build.  An optimiser splits the data at runs
	of zeros, so a font spans many segments; it is only whole again in the image."""
	placed = [(addr, pos, n) for (pos, n, addr) in segments if addr is not None]
	if not placed:
		return b''
	lo = min(a for (a, _, _) in placed)
	hi = max(a + n for (a, _, n) in placed)
	img = bytearray(hi - lo)
	for (a, pos, n) in placed:
		img[a - lo:a - lo + n] = buf[pos:pos + n]
	return bytes(img)


def fonts_in(img):
	found = []
	end = len(img)
	i = 0
	while True:
		hits = [h for h in (img.find(m, i, end) for m in SFNT_MAGICS) if h >= 0]
		if not hits:
			return found
		h = min(hits)
		ln = sfnt_length(img, h, end)
		if ln:
			found.append((h, ln))
			i = h + ln
		else:
			i = h + 1


def demangle(sym):
	"""Legacy Rust mangling (`_ZN3foo3bar17h0123456789abcdefE`) to a path; anything else unchanged."""
	if not sym.startswith('_ZN'):
		return sym
	i = 3
	parts = []
	while i < len(sym) and sym[i] != 'E':
		j = i
		while j < len(sym) and sym[j].isdigit():
			j += 1
		if j == i:
			break
		n = int(sym[i:j])
		parts.append(sym[j:j + n])
		i = j + n
	if parts and len(parts[-1]) == 17 and parts[-1].startswith('h'):
		parts.pop()
	return '::'.join(parts)


LEGACY_ESCAPES = (
	('$LT$', '<'), ('$GT$', '>'), ('$u20$', ' '), ('$RF$', '&'), ('$BP$', '*'), ('$C$', ','),
	('$u7b$', '{'), ('$u7d$', '}'), ('$u5b$', '['), ('$u5d$', ']'), ('$u27$', "'"), ('$u22$', '"'),
	('$LP$', '('), ('$RP$', ')'), ('$SP$', '@'), ('..', '::'),
)


def unescape(s):
	"""A legacy-mangled segment's escapes (`_$LT$a..b$GT$`) as the path they spell (`<a::b>`)."""
	if s.startswith('_$'):
		s = s[1:]
	for a, b in LEGACY_ESCAPES:
		s = s.replace(a, b)
	return s


def module_of(name, depth):
	"""The module a function belongs to, to `depth` path segments.  A trait impl
	(`<a::B as c::D>::f`) is charged to its self type's module."""
	s = unescape(demangle(name))
	if s.startswith('<'):
		inner = s[1:]
		for stop in (' as ', '>'):
			k = inner.find(stop)
			if k >= 0:
				inner = inner[:k]
				break
		s = inner.lstrip('&*').replace('mut ', '').replace('const ', '')
	s = s.split('<')[0]
	segs = [p for p in s.split('::') if p]
	if len(segs) <= 1:
		return '(unattributed)' if not segs else segs[0]
	return '::'.join(segs[:depth])


def measure(path, depth):
	buf = open(path, 'rb').read()
	secs = sections(buf)
	out = {
		'path':		path,
		'raw':		len(buf),
		'gzip9':	len(gzip.compress(buf, 9)),
		'brotli':	len(brotli.compress(buf, quality=11)) if brotli else None,
		'sections':	{},
	}
	n_imports = 0
	bodies = []
	segments = []
	names = None
	for (sid, name, start, size) in secs:
		out['sections'][name] = out['sections'].get(name, 0) + size
		if sid == 2:
			n_imports = imported_functions(buf, start, size)
		elif sid == 10:
			bodies = code_bodies(buf, start, size)
		elif sid == 11:
			segments = data_segments(buf, start, size)
		elif name == 'custom:name':
			names = function_names(buf, start, size)
	out['code'] = out['sections'].get('code', 0)
	out['data'] = out['sections'].get('data', 0)
	out['custom'] = sum(v for k, v in out['sections'].items() if k.startswith('custom:'))
	fonts = fonts_in(memory_image(buf, segments))
	out['fonts'] = [{'offset': h, 'bytes': ln} for (h, ln) in fonts]
	out['font_bytes'] = sum(ln for (_, ln) in fonts)
	out['raw_minus_fonts'] = out['raw'] - out['font_bytes']
	out['functions'] = len(bodies)
	if names:
		per = {}
		crates = {}
		for i, n in enumerate(bodies):
			name = names.get(n_imports + i, '')
			mod = module_of(name, depth)
			per[mod] = per.get(mod, 0) + n
			crate = module_of(name, 1)
			crates[crate] = crates.get(crate, 0) + n
		out['modules'] = sorted(per.items(), key=lambda kv: -kv[1])
		out['crates'] = sorted(crates.items(), key=lambda kv: -kv[1])
	else:
		out['modules'] = None
		out['crates'] = None
	return out


def mb(n):
	return '-' if n is None else '%.2f MB' % (n / 1048576.0)


def report(m, top):
	print(m['path'])
	print('  raw           %12d  %s' % (m['raw'], mb(m['raw'])))
	print('  gzip -9       %12d  %s' % (m['gzip9'], mb(m['gzip9'])))
	if m['brotli'] is None:
		print('  brotli        %12s  (python brotli module unavailable)' % '-')
	else:
		print('  brotli        %12d  %s' % (m['brotli'], mb(m['brotli'])))
	print('  code section  %12d  %s  (%d functions)' % (m['code'], mb(m['code']), m['functions']))
	print('  data section  %12d  %s' % (m['data'], mb(m['data'])))
	print('  custom        %12d  %s' % (m['custom'], mb(m['custom'])))
	print('  fonts in data %12d  %s  (%d faces)' % (m['font_bytes'], mb(m['font_bytes']), len(m['fonts'])))
	print('  raw - fonts   %12d  %s' % (m['raw_minus_fonts'], mb(m['raw_minus_fonts'])))
	if m['modules'] is None:
		print('  (no name section: build with names retained for per-module attribution)')
		return
	for title, rows in (('crate', m['crates']), ('module', m['modules'])):
		print('  code by %s (top %d):' % (title, top))
		for mod, n in rows[:top]:
			print('    %10d  %5.1f%%  %s' % (n, 100.0 * n / max(1, m['code']), mod))


def main():
	ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
	ap.add_argument('wasm', nargs='+')
	ap.add_argument('--depth', type=int, default=2, help='module path segments for attribution')
	ap.add_argument('--top', type=int, default=25)
	ap.add_argument('--json', help='also write the measurements to this file')
	ap.add_argument('--limit-mb', type=float, help='fail when the first binary is larger (raw)')
	ap.add_argument('--compare', action='store_true', help='print a side-by-side table last')
	a = ap.parse_args()
	ms = []
	for p in a.wasm:
		m = measure(p, a.depth)
		ms.append(m)
		report(m, a.top)
		print()
	if a.compare and len(ms) > 1:
		print('%-44s %10s %10s %10s %10s %10s' % ('binary', 'raw', 'gzip', 'brotli', 'fonts', 'raw-fonts'))
		for m in ms:
			print('%-44s %10s %10s %10s %10s %10s' % (
				os.path.basename(m['path'])[:44], mb(m['raw']), mb(m['gzip9']), mb(m['brotli']),
				mb(m['font_bytes']), mb(m['raw_minus_fonts'])))
		base = ms[0]
		for m in ms[1:]:
			print('  %s / %s: raw %.1f%%, raw minus fonts %.1f%%' % (
				os.path.basename(base['path']), os.path.basename(m['path']),
				100.0 * base['raw'] / max(1, m['raw']),
				100.0 * base['raw_minus_fonts'] / max(1, m['raw_minus_fonts'])))
	if a.json:
		with open(a.json, 'w') as f:
			json.dump(ms, f, indent=1)
	if a.limit_mb is not None:
		limit = a.limit_mb * 1048576.0
		ok = ms[0]['raw'] <= limit
		print('pass line: %s raw %s against %.1f MB: %s' % (
			os.path.basename(ms[0]['path']), mb(ms[0]['raw']), a.limit_mb, 'PASS' if ok else 'FAIL'))
		return 0 if ok else 1
	return 0


if __name__ == '__main__':
	sys.exit(main())
