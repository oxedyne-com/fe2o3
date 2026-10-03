// phrase.test.mjs -- the word-run test of `../redact.js`, `createRunGuard`.
//
// A passphrase drawn from a list of words is `n` of them in a row, wherever it turned up. This holds
// the JavaScript to `fe2o3_net/tests/data/lens_phrase.tsv`, the table the Rust `lens::Phrase` is held
// to, so the page and the peer cover the same strings: the list in its first line, then each case as
// the input, `n`, whether it is a hit and the spans found (as the text of each). Offsets are the
// language's own (UTF-16 units here, bytes in Rust), so the table compares text, not positions.
//
//   node test/phrase.test.mjs
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRunGuard } from '../redact.js';
import { createRunGuard as viaWriter } from '../writer.js';
import { check, finish, section } from './harness.mjs';

const DATA = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..', 'tests', 'data');
const table = readFileSync(join(DATA, 'lens_phrase.tsv'), 'utf8').split('\n').filter((l) => l.length);
const list = table[0].split('\t')[1].split(' ');
const cases = table.slice(1).map((l) => l.split('\t')).map(([t, n, hit, spans]) => ({ text: JSON.parse(t), n: Number(n), hit: hit === '1', spans: JSON.parse(spans) }));

section('phrase: the shared case file, ' + cases.length + ' cases');
{
	check('the table has the 36 cases Rust is tested on, and a list with hyphenated entries', cases.length === 36 && list.some((w) => w.includes('-')), cases.length + ' cases');
	let bad = 0, first = '', hits = 0, misses = 0;
	for (const c of cases) {
		const g = createRunGuard(list, c.n);
		const got = g.spans(c.text).map(([a, b]) => c.text.slice(a, b));
		const ok = JSON.stringify(got) === JSON.stringify(c.spans) && g.has(c.text) === c.hit;
		if (!ok) { bad += 1; first = first || JSON.stringify([c.text, got, c.spans]); }
		if (c.hit) hits += 1; else misses += 1;
	}
	check('every case finds the spans the table says, and a hit only where it says', bad === 0, bad + ' differ, first ' + first);
	check('the table is not one answer', hits >= 20 && misses >= 10, hits + ' hits, ' + misses + ' misses');
	const hy = cases.filter((c) => /drop-down|t-shirt|yo-yo|felt-tip/i.test(c.text));
	check('it holds the hyphenated entries a split on every non-letter cannot see', hy.length >= 5 && hy.filter((c) => c.hit).length >= 4, hy.length + ' cases');
}

section('phrase: the guard');
{
	const W = 'abacus abdomen abdominal ability able abstract absurd acid';
	const g = createRunGuard(list);
	check('the writer core hands on the same guard, so a page needs one import', viaWriter === createRunGuard);
	check('eight is the default run', g.has(W) && !g.has('abacus abdomen abdominal ability able abstract absurd'));
	check('a run of nine is one span', g.spans(W + ' down').length === 1);
	check('a word list may be a Set', createRunGuard(new Set(list)).has(W));
	check('a word list may be given in capitals', createRunGuard(list.map((w) => w.toUpperCase())).has(W));
	check('n may be 1', createRunGuard(list, 1).has('say acid now') && !createRunGuard(list, 1).has('say nothing'));
	// The list is the app's, and may not be there yet.
	let loaded = null;
	const lazy = createRunGuard(() => loaded);
	check('a list that is not there yet finds nothing', !lazy.has(W) && lazy.spans(W).length === 0 && lazy.mask(W, () => 'X') === W);
	loaded = list;
	check('and finds it once the list arrives', lazy.has(W));
	check('a list that throws finds nothing and does not throw', !createRunGuard(() => { throw new Error('no list'); }).has(W));
	check('an empty list finds nothing', !createRunGuard([]).has(W));
	check('a non-string finds nothing', !g.has(null) && !g.has(7) && g.spans(undefined).length === 0);

	const mark = (run) => '<' + run.length + '>';
	check('mask replaces each span and leaves the rest', g.mask('say ' + W + ' now, then ' + W + '.', mark) === 'say <' + W.length + '> now, then <' + W.length + '>.');
	check('mask leaves a string with no run as it is', g.mask('nothing to see here', mark) === 'nothing to see here');
	const obj = { a: W, [W]: 'k', list: ['x', 'y ' + W], n: 1, deep: { deeper: { text: W } } };
	const copy = JSON.stringify(obj);
	const out = g.deep(obj, mark);
	check('deep masks every string and every key', JSON.stringify(out) === JSON.stringify({ a: '<' + W.length + '>', ['<' + W.length + '>']: 'k', list: ['x', 'y <' + W.length + '>'], n: 1, deep: { deeper: { text: '<' + W.length + '>' } } }));
	check('and never changes what it was given', JSON.stringify(obj) === copy);
	let nest = { v: W }; for (let i = 0; i < 60; i++) nest = { n: nest };
	check('deep stops at forty levels without throwing', (() => { try { g.deep(nest, mark); return true; } catch (e) { return false; } })());
}

finish('phrase');
