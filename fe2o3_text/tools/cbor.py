"""A minimal std-only CBOR decoder, enough to read hayagriva's archived citationberg records."""

import struct


def decode(data):
	val, pos = _item(data, 0)
	return val


def _arg(data, pos, info):
	if info < 24:
		return info, pos
	n = {24: 1, 25: 2, 26: 4, 27: 8}[info]
	return int.from_bytes(data[pos:pos + n], "big"), pos + n


def _item(data, pos):
	b = data[pos]
	pos += 1
	major, info = b >> 5, b & 31
	if major == 7:
		if info == 20:
			return False, pos
		if info == 21:
			return True, pos
		if info in (22, 23):
			return None, pos
		if info == 25:
			return _half(data[pos:pos + 2]), pos + 2
		if info == 26:
			return struct.unpack(">f", data[pos:pos + 4])[0], pos + 4
		if info == 27:
			return struct.unpack(">d", data[pos:pos + 8])[0], pos + 8
		if info < 24:
			return info, pos
		raise ValueError("cbor: simple value %d at %d" % (info, pos))
	if info == 31:
		# Indefinite length.
		out = [] if major in (4, 2, 3) else {}
		if major in (2, 3):
			chunks = []
			while data[pos] != 0xff:
				c, pos = _item(data, pos)
				chunks.append(c)
			return (b"" if major == 2 else "").join(chunks), pos + 1
		while data[pos] != 0xff:
			if major == 4:
				v, pos = _item(data, pos)
				out.append(v)
			else:
				k, pos = _item(data, pos)
				v, pos = _item(data, pos)
				out[_key(k)] = v
		return out, pos + 1
	n, pos = _arg(data, pos, info)
	if major == 0:
		return n, pos
	if major == 1:
		return -1 - n, pos
	if major == 2:
		return bytes(data[pos:pos + n]), pos + n
	if major == 3:
		return data[pos:pos + n].decode("utf-8"), pos + n
	if major == 4:
		out = []
		for _ in range(n):
			v, pos = _item(data, pos)
			out.append(v)
		return out, pos
	if major == 5:
		out = {}
		for _ in range(n):
			k, pos = _item(data, pos)
			v, pos = _item(data, pos)
			out[_key(k)] = v
		return out, pos
	if major == 6:
		return _item(data, pos)
	raise ValueError("cbor: major %d" % major)


def _key(k):
	return tuple(k) if isinstance(k, list) else k


def _half(b):
	h = int.from_bytes(b, "big")
	s, e, m = h >> 15, (h >> 10) & 31, h & 1023
	if e == 0:
		v = m * 2.0 ** -24
	elif e == 31:
		v = float("inf") if m == 0 else float("nan")
	else:
		v = (1 + m / 1024.0) * 2.0 ** (e - 15)
	return -v if s else v
