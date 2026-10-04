// clip.test.mjs -- `clipChars` and `soundText` of `../redact.js`: a string is cut at a character, never inside one.
//
// JSON.stringify writes half of a surrogate pair as a `\udXXX` escape, which the peer's strict reader
// (`fe2o3_net::lens::chunk`) refuses and so covers the whole bundle. No row may hold half a pair, so a
// page clips with the one and mends what arrives broken with the other.
//
//   node test/clip.test.mjs
import * as redact from '../redact.js';
import * as writer from '../writer.js';
import { check, finish, part, section } from './harness.mjs';
import { makeHost, makeWriter } from './host.mjs';

const E = '\u{1F600}';
// Half a pair, as the text holds it, and as JSON.stringify would write it.
const lone = (s) => /[\uD800-\uDBFF](?![\uDC00-\uDFFF])/.test(s) || /(?:^|[^\uD800-\uDBFF])[\uDC00-\uDFFF]/.test(s);
const escaped = (s) => /\\ud[89a-f][0-9a-f]{2}/i.test(JSON.stringify(s));

section('clip: a string is cut at a character');
await part(async () => {
	// An emoji (two units) at each offset round the cut at forty, which falls inside it at one of them.
	let naive = false, bad = '', short = '';
	for (let pad = 36; pad <= 43; pad++) {
		const s = 'x'.repeat(pad) + E.repeat(4) + ' the rest';
		const c = redact.clipChars(s, 40);
		naive = naive || lone(s.slice(0, 40));
		if (lone(c) || escaped(c) || !s.startsWith(c)) bad += ' ' + pad;
		if (c.length < 39 || c.length > 40) short += ' ' + pad + ':' + c.length;
	}
	check('the cut this test makes does split a pair when it is made by unit', naive);
	check('no cut leaves half of a pair, and each keeps what it can', bad === '' && short === '', 'bad' + bad + ' short' + short);
	check('a string within the limit is returned as it is', redact.clipChars('short', 40) === 'short' && redact.clipChars(E, 2) === E);
	check('a pair that fits whole is kept', redact.clipChars('a' + E + 'b', 3) === 'a' + E);
	check('a cut at nothing is nothing', redact.clipChars('abc', 0) === '');
});

section('sound: a lone half becomes U+FFFD');
await part(async () => {
	const cases = [ [ 'a\uD83Db', 'a�b' ], [ '\uDE00 hi', '� hi' ], [ 'ends\uD83D', 'ends�' ], [ 'two\uD83D\uD83Dhalves', 'two��halves' ] ];
	check('each half that has no other half is replaced, and nothing else is', cases.every(([ s, want ]) => redact.soundText(s) === want),
		cases.map(([ s ]) => escape(redact.soundText(s))).join(' '));
	check('and what it returns holds no half to be written as an escape', cases.every(([ s ]) => !lone(redact.soundText(s)) && !escaped(redact.soundText(s))));
	check('a whole pair, and plain text, are left as they are', redact.soundText(E + ' ok') === E + ' ok' && redact.soundText('plain, é') === 'plain, é');
});

section('writer: the core hands both on, and its own clip cuts at a character');
await part(async () => {
	check('a page needs one import', typeof writer.clipChars === 'function' && writer.clipChars === redact.clipChars && writer.soundText === redact.soundText);
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	// 199 units, then a pair: the writer's clip at 200 falls between its halves.
	w.noteError('word '.repeat(39) + 'abcd' + E + ' tail', '');
	const row = w.outbox()[0];
	check('an error message clipped at 200 holds no half of a pair', !lone(JSON.parse(row.data).msg) && !/\\ud[89a-f][0-9a-f]{2}/i.test(row.data), row.data.slice(-40));
	h.halt();
});

finish('clip');
