// pad.mjs -- the JavaScript twin of `fe2o3_crypto::pad` (fe2o3_crypto/src/pad.rs).
//
// `padmeLen` is Padmé, Algorithm 1 of Nikitin et al., "Reducing Metadata Leakage from Encrypted
// Files and Communication with PURBs" (PoPETs 2019, arXiv 1806.03160, §4.4). A frame is
// `u32 big-endian body length ‖ body ‖ zeros`, its whole length rounded by a rule and never below
// the rule's floor; `unpad` needs no rule. Pad before sealing and unpad after opening. Both
// twins are held to `fe2o3_crypto/tests/data/padme_vectors.txt`.

export const PREFIX_LEN = 4; // fixed: the frame format, a u32 big-endian body length
const U32_MAX = 0xffffffff; // fixed: the prefix is a u32
const U64_MAX = (1n << 64n) - 1n; // fixed: the Rust twin's u64 range

function big(x, what) {
	if (typeof x === 'bigint') return x;
	if (Number.isSafeInteger(x) && x >= 0) return BigInt(x);
	throw new RangeError(what + ' must be a non-negative safe integer or a BigInt, not ' + x);
}

// A BigInt in gives a BigInt out; a Number in gives a Number, refused past 2^53 − 1.
function back(v, asBig, what) {
	if (v > U64_MAX) throw new RangeError(what + ' overflows 64 bits');
	if (asBig) return v;
	if (v > BigInt(Number.MAX_SAFE_INTEGER)) throw new RangeError(what + ' is past 2^53 − 1; pass a BigInt');
	return Number(v);
}

function bitLen(v) { return v === 0n ? 0n : BigInt(v.toString(2).length); }

// Padmé's padded length for `len` bytes; below 2 the length stands, as in the Rust twin.
export function padmeLen(len) {
	const asBig = typeof len === 'bigint';
	const l = big(len, 'len');
	if (l < 2n) return back(l, asBig, 'padmeLen');
	const e = bitLen(l) - 1n; // ⌊log₂ L⌋, the exponent
	const s = bitLen(e); // ⌊log₂ E⌋ + 1, the bits that hold E
	const m = (1n << (e - s)) - 1n; // low bits forced to zero
	return back((l + m) & ~m, asBig, 'padmeLen(' + l + ')');
}

// A rule as configured: `{ rule: 'padme' | 'pow2' | 'block', floor, block }`, the shape of the
// Rust `PadConfig` (defaults padme, 256, 1 MiB). `block` is read only by `block`.
export const PAD_DEFAULT = Object.freeze({ rule: 'padme', floor: 256, block: 1048576 });

export function padRule(cfg = PAD_DEFAULT) {
	const c = { ...PAD_DEFAULT, ...cfg };
	if (c.rule !== 'padme' && c.rule !== 'pow2' && c.rule !== 'block') {
		throw new RangeError('The pad rule ' + JSON.stringify(c.rule) + ' is not one of padme, pow2 or block');
	}
	if (c.rule === 'block' && big(c.block, 'block') === 0n) {
		throw new RangeError("The pad rule 'block' needs 'block' of at least one byte");
	}
	return Object.freeze(c);
}

// The rounded length of a frame of `len` bytes, the prefix included.
export function round(rule, len) {
	const asBig = typeof len === 'bigint';
	const l = big(len, 'len');
	const floor = big(rule.floor, 'floor');
	let got;
	if (rule.rule === 'padme') {
		got = big(padmeLen(l), 'padme');
	} else if (rule.rule === 'pow2') {
		got = 1n;
		while (got < l) got <<= 1n;
	} else if (rule.rule === 'block') {
		const b = big(rule.block, 'block');
		if (b === 0n) throw new RangeError('A block pad rule needs a block of at least one byte');
		got = ((l + b - 1n) / b) * b;
	} else {
		throw new RangeError('Unknown pad rule ' + JSON.stringify(rule.rule));
	}
	return back(got > floor ? got : floor, asBig, 'round(' + l + ')');
}

// The length of the frame `pad` makes for a body of `bodyLen` bytes.
export function framedLen(rule, bodyLen) {
	const n = big(bodyLen, 'bodyLen');
	if (n > BigInt(U32_MAX)) throw new RangeError('A body of ' + n + " bytes is over the frame's u32 length prefix");
	const got = round(rule, n + BigInt(PREFIX_LEN));
	return typeof bodyLen === 'bigint' ? got : Number(got);
}

// `body` (a Uint8Array) framed and padded: its length as a big-endian u32, the body, then zeros.
export function pad(rule, body) {
	const total = framedLen(rule, body.length);
	const out = new Uint8Array(total);
	new DataView(out.buffer).setUint32(0, body.length, false);
	out.set(body, PREFIX_LEN);
	return out;
}

// The body inside a frame `pad` made, as a view onto it. Throws when the frame is shorter than
// its prefix, the prefix claims more than the frame holds, or the fill is not all zeros.
export function unpad(frame) {
	if (frame.length < PREFIX_LEN) {
		throw new RangeError('A padded frame of ' + frame.length + ' bytes is shorter than its ' + PREFIX_LEN + '-byte length prefix');
	}
	const n = new DataView(frame.buffer, frame.byteOffset, frame.byteLength).getUint32(0, false);
	const end = PREFIX_LEN + n;
	if (end > frame.length) throw new RangeError('A padded frame of ' + frame.length + ' bytes claims a body of ' + n + ' bytes');
	for (let i = end; i < frame.length; i++) {
		if (frame[i] !== 0) throw new RangeError("A padded frame's fill after its " + n + '-byte body is not all zeros');
	}
	return frame.subarray(PREFIX_LEN, end);
}
