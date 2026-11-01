// pad.test.mjs -- `../pad.mjs` against `fe2o3_crypto/tests/data/padme_vectors.txt`, the vectors
// the Rust `tests/pad.rs` checks, made by an oracle written from arXiv 1806.03160 alone.
//
//   node test/pad.test.mjs
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { PREFIX_LEN, framedLen, pad, padRule, padmeLen, unpad } from '../pad.mjs';

let passes = 0, failures = 0;
function check(name, cond, detail) {
	if (cond) { passes += 1; console.log('  ok   ' + name); }
	else { failures += 1; console.log('  FAIL ' + name + (detail ? ' -- ' + detail : '')); }
}
function throws(fn) { try { fn(); return false; } catch (e) { return true; } }

const DATA = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'tests', 'data');
const lines = readFileSync(join(DATA, 'padme_vectors.txt'), 'utf8').split('\n').filter((l) => l && !l.startsWith('#'));

{
	let seen = 0, bad = 0, first = '';
	for (const l of lines) {
		const f = l.split(' ');
		if (f[0] !== 'padme') continue;
		seen += 1;
		const got = padmeLen(BigInt(f[1]));
		if (got !== BigInt(f[2])) { bad += 1; first = first || l + ' got ' + got; }
		if (BigInt(f[2]) <= BigInt(Number.MAX_SAFE_INTEGER) && padmeLen(Number(f[1])) !== Number(f[2])) { bad += 1; first = first || l + ' (Number)'; }
	}
	check('padmeLen matches every padme vector (' + seen + ')', seen >= 1000 && bad === 0, bad + ' bad, first ' + first);
}

{
	let seen = 0, bad = 0, first = '';
	for (const l of lines) {
		const f = l.split(' ');
		if (f[0] !== 'frame') continue;
		seen += 1;
		const rule = padRule({ rule: f[1], floor: Number(f[2]), block: Number(f[3]) });
		const n = Number(f[4]), want = Number(f[5]);
		let ok = framedLen(rule, n) === want;
		if (ok && n < 5000) {
			const body = Uint8Array.from({ length: n }, (_, i) => (i * 7 + 1) & 0xff);
			const fr = pad(rule, body);
			const b = unpad(fr);
			ok = fr.length === want && b.length === n && b.every((x, i) => x === body[i]);
		}
		if (!ok) { bad += 1; first = first || l + ' got ' + framedLen(rule, n); }
	}
	check('framedLen, pad and unpad match every frame vector (' + seen + ')', seen >= 1000 && bad === 0, bad + ' bad, first ' + first);
}

{
	const rule = padRule();
	const fr = pad(rule, Uint8Array.of(1, 2, 3));
	check('the default rule frames 3 bytes as 00000003 010203 and zeros to the 256-byte floor', fr.length === 256 && fr[3] === 3 && fr[4] === 1 && fr[6] === 3 && fr.slice(7).every((x) => x === 0));
	const lie = Uint8Array.from(fr); lie[3] = 200; lie[2] = 1;
	check('unpad refuses a prefix longer than the frame', throws(() => unpad(lie)));
	const dirty = Uint8Array.from(fr); dirty[255] = 1;
	check('unpad refuses a fill that is not all zeros', throws(() => unpad(dirty)));
	check('unpad refuses a frame shorter than its prefix', throws(() => unpad(new Uint8Array(PREFIX_LEN - 1))));
	check('padRule refuses an unknown rule and an empty block', throws(() => padRule({ rule: 'nope' })) && throws(() => padRule({ rule: 'block', block: 0 })));
}

console.log('\npad: ' + passes + ' ok, ' + failures + ' failed');
process.exit(failures ? 1 : 0);
