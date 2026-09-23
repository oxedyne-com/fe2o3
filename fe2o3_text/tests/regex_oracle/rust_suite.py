#!/usr/bin/env python3
"""Converts the Rust `regex` crate's own test suite into rust_suite.txt for tests/regex.rs.

The crate's `testdata/*.toml` files state what the crate -- and so Typst's `regex(...)` -- answers
for each pattern and haystack.  Only the cases that ask what fe2o3_text's engine offers are kept:
leftmost-first searching over UTF-8 text with Unicode on, no bounds, no anchoring, no regex sets
and no byte-oriented `(?-u)` patterns.  The data is the crate's, under its MIT or Apache-2.0
licence.

Run from this directory, naming one or more of the crate's source directories; a case named in
an earlier one is not taken again from a later one.  1.11.1 still carries the AT&T "fowler"
suites that later releases dropped from the package:

    python3 rust_suite.py ~/.cargo/registry/src/*/regex-1.13.1 \\
        ~/.cargo/registry/src/*/regex-1.11.1 > rust_suite.txt
"""
import pathlib
import re
import sys
import tomllib

def esc(s):
    return s.replace('\\', '\\\\').replace('\t', '\\t').replace('\n', '\\n').replace('\r', '\\r')

def unescape(s):
    # The crate's own unescaping: \xNN bytes, \n, \t and friends.  Bytes that are not UTF-8 make
    # the case one for byte haystacks, which the caller drops.
    out = bytearray()
    i = 0
    b = s.encode('utf-8')
    while i < len(b):
        c = b[i:i+1]
        if c == b'\\' and i + 1 < len(b):
            n = b[i+1:i+2]
            if n == b'x' and i + 3 < len(b) + 1:
                out += bytes([int(b[i+2:i+4], 16)])
                i += 4
                continue
            out += {b'n': b'\n', b't': b'\t', b'r': b'\r', b'\\': b'\\', b'0': b'\0'}.get(n, b'\\' + n)
            i += 2
            continue
        out += c
        i += 1
    return out.decode('utf-8')

def unsupported_flags(rx):
    # `R` (CRLF lines) is not offered, and `-u` turns Unicode off, which this engine never does.
    for g in re.findall(r'\(\?([a-zA-Z-]+)[:)]', rx):
        on, _, off = g.partition('-')
        if 'R' in g or 'u' in off:
            return True
    return False

def emit(key, t):
    """Writes one case, or returns False when it asks for something the engine does not offer."""
    rx = t.get('regex')
    anchored = bool(t.get('anchored'))
    if (not isinstance(rx, str)
            or t.get('utf8', True) is False
            or t.get('unicode', True) is False
            or 'bounds' in t or 'line-terminator' in t
            or (anchored and len(t.get('matches', [])) > 1)
            or t.get('match-kind', 'leftmost-first') != 'leftmost-first'
            or t.get('search-kind', 'leftmost') != 'leftmost'
            or unsupported_flags(rx)):
        return False
    hay = t.get('haystack', '')
    if t.get('unescape'):
        try:
            hay = unescape(hay)
        except (UnicodeDecodeError, ValueError):
            return False
    flags = ('i' if t.get('case-insensitive') else '') + ('a' if anchored else '')
    print('T\t' + key + '\t' + (flags or '-'))
    print('P\t' + esc(rx))
    print('H\t' + esc(hay))
    if t.get('compiles', True) is False:
        print('C')
    else:
        limit = t.get('match-limit')
        print('L\t' + (str(limit) if limit is not None else '-'))
        for m in t.get('matches', []):
            if isinstance(m, dict):
                m = m['span']
            if m and isinstance(m[0], list):
                print('M\t' + ' '.join(f'{g[0]},{g[1]}' if g else '-' for g in m))
            else:
                print('M\t' + f'{m[0]},{m[1]}')
    print('E')
    return True

seen = set()
kept = skipped = 0
for root in sys.argv[1:]:
    base = pathlib.Path(root) / 'testdata'
    for path in sorted(base.rglob('*.toml')):
        rel = str(path.relative_to(base))
        # regex-lite is another crate, ASCII-only by design; its cases are not the regex crate's.
        if rel == 'regex-lite.toml':
            continue
        for t in tomllib.loads(path.read_text()).get('test', []):
            key = rel + '/' + t.get('name', '')
            if key in seen:
                continue
            seen.add(key)
            if emit(key, t):
                kept += 1
            else:
                skipped += 1
print(f'{kept} kept, {skipped} skipped', file=sys.stderr)
