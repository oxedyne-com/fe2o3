#!/usr/bin/env python3
"""Writes the WebAssembly plugin fixtures for Austenite's plugin tests, std-only.

Each module is described by the table `MODULES` below: its imports, memory, functions with their
instruction bytes, exports and start function. The assembler is a hand encoder of the WebAssembly
binary format (version 1), enough for the fixtures and nothing more. Every `.wasm` is written beside a
`.wat`-like listing of its source, so a reader can see what a binary does without a disassembler.

    python3 -I fe2o3_austenite/tools/plugin/mkwasm.py [outdir]

The default outdir is `tests/fixtures/eval/plugin/` under the crate. The output is deterministic.
"""

import os
import sys

HERE	= os.path.dirname(os.path.abspath(__file__))
CRATE	= os.path.dirname(os.path.dirname(HERE))
OUT		= os.path.join(CRATE, "tests", "fixtures", "eval", "plugin")

I32, I64 = 0x7f, 0x7e

# Instructions, as (mnemonic, bytes) so the listing names them.
def uleb(n):
	out = bytearray()
	while True:
		b = n & 0x7f
		n >>= 7
		if n:
			out.append(b | 0x80)
		else:
			out.append(b)
			return bytes(out)

def sleb(n):
	out = bytearray()
	while True:
		b = n & 0x7f
		n >>= 7
		done = (n == 0 and not b & 0x40) or (n == -1 and b & 0x40)
		out.append(b if done else b | 0x80)
		if done:
			return bytes(out)

def i32_const(n):	return ("i32.const %d" % n, b"\x41" + sleb(n))
def local_get(i):	return ("local.get %d" % i, b"\x20" + uleb(i))
def call(i):		return ("call %d" % i, b"\x10" + uleb(i))
def br(d):			return ("br %d" % d, b"\x0c" + uleb(d))
def br_if(d):		return ("br_if %d" % d, b"\x0d" + uleb(d))
def load8(off=0):	return ("i32.load8_u offset=%d" % off, b"\x2d\x00" + uleb(off))
def store8(off=0):	return ("i32.store8 offset=%d" % off, b"\x3a\x00" + uleb(off))
LOOP		= ("loop", b"\x03\x40")
END			= ("end", b"\x0b")
DROP		= ("drop", b"\x1a")
ADD			= ("i32.add", b"\x6a")
NE			= ("i32.ne", b"\x47")
UNREACHABLE	= ("unreachable", b"\x00")
GROW		= ("memory.grow", b"\x40\x00")

# The two host imports, function indices 0 and 1 when a module imports both.
WRITE_ARGS	= ("typst_env", "wasm_minimal_protocol_write_args_to_buffer", [I32], [])
SEND_RESULT	= ("typst_env", "wasm_minimal_protocol_send_result_to_host", [I32, I32], [])
HOST		= [WRITE_ARGS, SEND_RESULT]
W, S		= 0, 1

def body_return(code, sum_params):
	"""Reads the arguments to address 0 and sends back the concatenation, returning `code`."""
	ins = [i32_const(0), call(W), i32_const(0)]
	if sum_params == 0:
		ins.append(i32_const(0))
	else:
		ins.append(local_get(0))
		for i in range(1, sum_params):
			ins += [local_get(i), ADD]
	ins += [call(S), i32_const(code)]
	return ins

MODULES = {
	# Returns its arguments joined; `two` takes two, `none` none.
	"echo": dict(imports=HOST, memory=(1, None), funcs=[
		("echo",	[I32], [I32], body_return(0, 1)),
		("two",		[I32, I32], [I32], body_return(0, 2)),
		("none",	[], [I32], body_return(0, 0)),
	]),
	# Returns 1 with its argument as the message.
	"err": dict(imports=HOST, memory=(1, None), funcs=[
		("fail",	[I32], [I32], body_return(1, 1)),
	]),
	# Returns 2, which the protocol does not define.
	"bad_proto": dict(imports=HOST, memory=(1, None), funcs=[
		("odd",		[I32], [I32], body_return(2, 1)),
	]),
	# Traps.
	"trap": dict(imports=HOST, memory=(1, None), funcs=[
		("boom",	[], [I32], [UNREACHABLE]),
	]),
	# A parameter that is not a 32-bit integer, and a function returning nothing.
	"nonint": dict(imports=HOST, memory=(1, None), funcs=[
		("wide",	[I64], [I32], [i32_const(0)]),
		("void",	[I32], [], []),
	]),
	# Exports no memory.
	"no_memory": dict(imports=[], memory=None, funcs=[
		("f",		[], [I32], [i32_const(0)]),
	]),
	# A byte of state at address 0: `inc` adds one, `get` returns it.
	"counter": dict(imports=HOST, memory=(1, None), funcs=[
		("inc",		[], [I32], [i32_const(0), i32_const(0), load8(), i32_const(1), ADD, store8(),
								i32_const(0), i32_const(1), call(S), i32_const(0)]),
		("get",		[], [I32], [i32_const(0), i32_const(1), call(S), i32_const(0)]),
	]),
	# Sends a result from beyond its memory.
	"oob": dict(imports=HOST, memory=(1, None), funcs=[
		("far",		[], [I32], [i32_const(0x20000), i32_const(4), call(S), i32_const(0)]),
	]),
	# Hostile: a loop that never ends.
	"spin": dict(imports=HOST, memory=(1, None), funcs=[
		("spin",	[], [I32], [LOOP, br(0), END, i32_const(0)]),
	]),
	# Hostile: grows its memory a page at a time until refused, then traps.
	"grow": dict(imports=HOST, memory=(1, None), funcs=[
		("grow",	[], [I32], [LOOP, i32_const(1), GROW, i32_const(-1), NE, br_if(0), END, UNREACHABLE]),
	]),
	# Hostile: a start function that never ends, which runs when the module loads.
	"spin_start": dict(imports=HOST, memory=(1, None), start="spin", funcs=[
		("spin",	[], [], [LOOP, br(0), END]),
		("f",		[], [I32], [i32_const(0)]),
	]),
	# Hostile: an initial memory of 32768 pages (2 GiB), above every cap.
	"huge_initial": dict(imports=HOST, memory=(32768, None), funcs=[
		("f",		[], [I32], [i32_const(0)]),
	]),
}

def vec(items):
	return uleb(len(items)) + b"".join(items)

def name(s):
	b = s.encode()
	return uleb(len(b)) + b

def section(sid, payload):
	return bytes([sid]) + uleb(len(payload)) + payload

def functype(params, results):
	return b"\x60" + vec([bytes([p]) for p in params]) + vec([bytes([r]) for r in results])

def limits(lo, hi):
	return (b"\x00" + uleb(lo)) if hi is None else (b"\x01" + uleb(lo) + uleb(hi))

def assemble(m):
	types = []
	def tyidx(params, results):
		t = functype(params, results)
		if t not in types:
			types.append(t)
		return types.index(t)
	imports = [name(mod) + name(fld) + b"\x00" + uleb(tyidx(p, r)) for (mod, fld, p, r) in m["imports"]]
	nimp = len(m["imports"])
	fidx = [uleb(tyidx(p, r)) for (_, p, r, _) in m["funcs"]]
	exports = [name(n) + b"\x00" + uleb(nimp + i) for i, (n, _, _, _) in enumerate(m["funcs"])]
	if m["memory"] is not None:
		exports.append(name("memory") + b"\x02" + uleb(0))
	codes = []
	for (_, _, _, ins) in m["funcs"]:
		body = b"\x00" + b"".join(b for (_, b) in ins) + END[1]
		codes.append(uleb(len(body)) + body)
	out = b"\x00asm\x01\x00\x00\x00"
	out += section(1, vec(types))
	if imports:
		out += section(2, vec(imports))
	out += section(3, vec(fidx))
	if m["memory"] is not None:
		out += section(5, vec([limits(*m["memory"])]))
	out += section(7, vec(exports))
	if "start" in m:
		names = [n for (n, _, _, _) in m["funcs"]]
		out += section(8, uleb(nimp + names.index(m["start"])))
	out += section(10, vec(codes))
	return out

def listing(key, m):
	ty = {I32: "i32", I64: "i64"}
	lines = ["(module $%s" % key]
	for (mod, fld, p, r) in m["imports"]:
		lines.append('  (import "%s" "%s" (func (param %s) (result %s)))' % (mod, fld,
			" ".join(ty[x] for x in p), " ".join(ty[x] for x in r)))
	if m["memory"] is not None:
		lines.append('  (memory (export "memory") %d)' % m["memory"][0])
	for (n, p, r, ins) in m["funcs"]:
		lines.append('  (func (export "%s") (param %s) (result %s)' % (n,
			" ".join(ty[x] for x in p), " ".join(ty[x] for x in r)))
		for (mn, _) in ins:
			lines.append("    " + mn)
		lines.append("  )")
	if "start" in m:
		lines.append("  (start $%s)" % m["start"])
	lines.append(")")
	return "\n".join(lines) + "\n"

def main():
	out = sys.argv[1] if len(sys.argv) > 1 else OUT
	os.makedirs(out, exist_ok=True)
	for key, m in MODULES.items():
		with open(os.path.join(out, key + ".wasm"), "wb") as f:
			f.write(assemble(m))
		with open(os.path.join(out, key + ".wat"), "w") as f:
			f.write(listing(key, m))
	print("wrote %d modules to %s" % (len(MODULES), out))

if __name__ == "__main__":
	main()
