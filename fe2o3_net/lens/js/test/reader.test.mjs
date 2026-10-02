// reader.test.mjs -- the node half of the lens, `../reader.mjs`.
//
// What is ported, and from where. Daimond's `dev/lens.mjs` is the reader, and its tests are
// `dev/lens.test.mjs` (the ledger join, run unchanged through `test/adapter/lens_daimond.mjs`
// by `test/adapter/run_lens.mjs`) and the scrubber section already carried by
// `redact.test.mjs`. Everything else here is new: the core takes the generic half of
// `lens.mjs` out of the app, and the tests pin it on the line T5a writes (`lens::Sink`:
// `{"bt","device","ts","tag","data"}`, `data` a string, one object a line, rotated as
// `<key>.ndjson.<stamp>[-n]`), on Daimond's text blocks, and on the two guarantees the plan
// asks of a reader: a row is scrubbed again on ingest, and a digest is never over 2 KB.
//
//   node test/reader.test.mjs
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import * as RD from '../reader.mjs';
import { createRedactor, createScrubber } from '../redact.js';
import { check, finish, part, section } from './harness.mjs';

const {
	capBytes, createReader, digest, eachBlock, emit, lineBlocks, mirrorFiles, parseArgs, parseLine,
	rangeAdd, rangeCount, rangeHas, rangeMissing, readLines, readNdjson, safeName, session, sinceMs,
	tagFamily, timeline, writeJson,
} = RD;

// ── Fixtures ─────────────────────────────────────────────────────────

const ROOT = fs.mkdtempSync(path.join(os.tmpdir(), 'lens-reader-test-'));
let seq = 0;
const fresh = () => { const d = path.join(ROOT, 'h' + (++seq)); fs.mkdirSync(d, { recursive: true }); return d; };
const put = (home, rel, text) => {
	const f = path.join(home, rel);
	fs.mkdirSync(path.dirname(f), { recursive: true });
	fs.writeFileSync(f, text);
	return f;
};
const add = (f, text) => fs.appendFileSync(f, text);

// What `lens::Entry::line` writes, in its key order.
const L = (bt, device, ts, tag, data) => JSON.stringify({ bt, device, ts, tag, data }) + '\n';
// The `data` of an `ev` row: the envelope `{v,d,n,b,t}` then the payload.
const evData = (d, n, t, payload) => JSON.stringify(Object.assign({ v: 1, d, n, b: 'b1', t }, payload || {}));
const EV = (bt, device, n, kind, payload, ts) =>
	L(bt, device, ts || bt - 5, 'ev ' + kind, evData(device, n, ts || bt - 5, payload));

// A deterministic run of characters, so a fixture is built and never typed as a credential.
const fake = (n, seed, alpha) => {
	alpha = alpha || 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
	let s = '', h = (seed * 2654435761) >>> 0;
	for (let i = 0; i < n; i++) { h = (h * 1103515245 + 12345) >>> 0; s += alpha[(h >>> 8) % alpha.length]; }
	return s;
};
const stripe = () => ['sk', 'test', fake(24, 7)].join('_');
const jwt    = () => 'eyJ' + fake(12, 8) + '.' + fake(14, 9) + '.' + fake(12, 10);
const awsId  = () => 'AKIA' + fake(16, 11, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789');
const b64    = (o) => Buffer.from(JSON.stringify(o), 'utf8').toString('base64');

// Every file under a directory, relative.
function walk(dir, base) {
	const out = [];
	for (const n of fs.readdirSync(dir, { withFileTypes: true })) {
		const p = path.join(dir, n.name);
		if (n.isDirectory()) out.push(...walk(p, base || dir)); else out.push(p);
	}
	return out;
}
const holds = (dir, needle) => walk(dir).some((f) => fs.readFileSync(f, 'utf8').includes(needle));

const NOW = 1_800_000_000_000;

// ═════════════════════════════════════════════════════════════════════
section('arguments and time');
await part(() => {
	const a = parseArgs(['turns', '--since', '5m', '--json', '--device', 'arg']);
	check('parseArgs: command, valued flags and bare flags',
		a.cmd === 'turns' && a.flags.since === '5m' && a.flags.device === 'arg' && a.bool.json === true);
	check('parseArgs: no command means digest', parseArgs([]).cmd === 'digest');
	check('parseArgs: a flag followed by a flag is a bare flag',
		parseArgs(['x', '--json', '--limit', '3']).bool.json === true && parseArgs(['x', '--json', '--limit', '3']).flags.limit === '3');

	const real = Date.now;
	Date.now = () => NOW;
	try {
		check('sinceMs: undefined and empty take the default', sinceMs(undefined, 7) === 7 && sinceMs('', 8) === 8 && sinceMs(null, 9) === 9);
		check('sinceMs: all is zero', sinceMs('all') === 0);
		check('sinceMs: bare numbers are seconds, so 90 and 90s agree', sinceMs('90') === NOW - 90_000 && sinceMs('90s') === NOW - 90_000);
		check('sinceMs: minutes, hours, days and weeks',
			sinceMs('5m') === NOW - 300_000 && sinceMs('2h') === NOW - 7_200_000
			&& sinceMs('7d') === NOW - 604_800_000 && sinceMs('1w') === NOW - 604_800_000);
		check('sinceMs: a fraction and a capital', sinceMs('1.5H') === NOW - 5_400_000);
		check('sinceMs: an ISO instant', sinceMs('2026-09-01T00:00:00Z') === Date.parse('2026-09-01T00:00:00Z'));
		// `new Date()` does not read the stubbed `Date.now`, so today is the real one.
		const mid = new Date(); mid.setHours(0, 0, 0, 0);
		check('sinceMs: today is local midnight', sinceMs('today') === mid.getTime());
		check('sinceMs: nonsense takes the default', sinceMs('soonish', 42) === 42);
	} finally { Date.now = real; }
});

section('integer range sets, which keep redelivery cheap');
await part(() => {
	const rs = [];
	check('rangeAdd: a new number is new', rangeAdd(rs, 5) === true && rangeHas(rs, 5));
	check('rangeAdd: a seen number is not', rangeAdd(rs, 5) === false);
	rangeAdd(rs, 7); rangeAdd(rs, 6);
	check('rangeAdd: neighbours merge into one range', JSON.stringify(rs) === '[[5,7]]');
	rangeAdd(rs, 1); rangeAdd(rs, 2); rangeAdd(rs, 10);
	check('rangeCount counts every number covered', rangeCount(rs) === 6, JSON.stringify(rs));
	check('rangeMissing names the gaps in 1..n', rangeMissing(rs, 12) === '3-4,8-9,11-12', rangeMissing(rs, 12));
	check('rangeMissing says how many more it left out', /\+\d+ more$/.test(rangeMissing([[2, 2], [4, 4], [6, 6]], 20, 2)));
	check('safeName keeps a name legal', safeName('a/b c.d') === 'a_b_c_d' && safeName('') === 'unknown' && safeName('x'.repeat(100)).length === 64);
});

// ═════════════════════════════════════════════════════════════════════
section('the line T5a writes');
await part(() => {
	const good = L(1700000000000, 'dev1', 1699999999990, 'ev turn.end', '{"v":1,"d":"dev1","n":4,"t":1}');
	const p = parseLine(good);
	check('parseLine reads bt, device, ts, tag and data', p && p.bt === 1700000000000 && p.device === 'dev1'
		&& p.ts === 1699999999990 && p.tag === 'ev turn.end' && p.data === '{"v":1,"d":"dev1","n":4,"t":1}');
	check('parseLine: data stays a string, not parsed', typeof p.data === 'string');
	check('parseLine ignores the trailing newline', parseLine(good.trimEnd()) && parseLine(good.trimEnd()).tag === 'ev turn.end');

	const odd = L(5, 'd"e\\v', 6, 'tag with  two spaces', 'line\nbreak   and é and 😀 "q"');
	const q = parseLine(odd);
	check('parseLine: quotes, escapes, line breaks and astral characters survive', q && q.device === 'd"e\\v'
		&& q.tag === 'tag with  two spaces' && q.data === 'line\nbreak   and é and 😀 "q"');

	check('parseLine: a missing data is empty', parseLine('{"bt":1,"device":"d","ts":2,"tag":"t"}').data === '');
	check('parseLine: a torn line is null', parseLine(good.slice(0, 40)) === null);
	check('parseLine: an empty line is null', parseLine('') === null && parseLine('   ') === null);
	check('parseLine: an array, a string and a number are null', parseLine('[1,2]') === null && parseLine('"x"') === null && parseLine('7') === null);
	check('parseLine: a line with no tag is null', parseLine('{"bt":1,"device":"d","ts":2,"data":"x"}') === null);
	check('parseLine: a non-numeric receive time is null', parseLine('{"bt":"x","device":"d","ts":2,"tag":"t","data":""}') === null);
	check('parseLine: data that is an object is null (T5a always writes a string)',
		parseLine('{"bt":1,"device":"d","ts":2,"tag":"t","data":{"a":1}}') === null);

	const home = fresh();
	const f = put(home, 'x.ndjson', L(1, 'a', 1, 't', 'one') + 'not json at all\n' + L(2, 'a', 2, 't', 'two') + good.slice(0, 30));
	const r = readLines(f);
	check('readLines: skips the malformed line and a torn tail, and counts them',
		r.rows.length === 2 && r.bad === 2 && r.rows[1].data === 'two', JSON.stringify({ n: r.rows.length, bad: r.bad }));
	check('readLines: a file that is not there is empty', readLines(path.join(home, 'nope')).rows.length === 0);
	check('readNdjson (archive records) skips a torn tail too', readNdjson(put(home, 'y.ndjson', '{"a":1}\n{"b":\n{"c":3}\n{"d"')).length === 2);
});

await part(() => {
	const rows = [
		{ bt: 10, device: 'a', ts: 1, tag: 't', data: 'x', raw: 'r1' }, { bt: 10, device: 'a', ts: 2, tag: 't', data: 'y', raw: 'r2' },
		{ bt: 10, device: 'b', ts: 3, tag: 't', data: 'z', raw: 'r3' }, { bt: 11, device: 'b', ts: 4, tag: 't', data: 'w', raw: 'r4' },
	];
	const bs = lineBlocks(rows);
	check('lineBlocks: consecutive rows of one receive time and device are one block',
		bs.length === 3 && bs[0].rows.length === 2 && bs[1].device === 'b' && bs[1].bt === 10 && bs[2].bt === 11);
	check('lineBlocks: a block carries the rows as tag, data and the raw line', bs[0].rows[0].tag === 't' && bs[0].rows[0].raw === 'r1');
});

section('tags: the three families a row can belong to');
await part(() => {
	const e = tagFamily('ev turn.end');
	check('tagFamily: ev <kind>', e.fam === 'ev' && e.kind === 'turn.end');
	const d = tagFamily('ds telemetry abc123 3/12');
	check('tagFamily: ds <kind> <id> i/N', d.fam === 'ds' && d.kind === 'telemetry' && d.id === 'abc123' && d.i === 3 && d.n === 12);
	check('tagFamily: ds snapshot', tagFamily('ds snapshot s-9 1/1').kind === 'snapshot');
	check('tagFamily: any other tag is a row, with single spaces kept', tagFamily('election peer').fam === 'row');
	check('tagFamily: a malformed ds tag is a row', tagFamily('ds telemetry id').fam === 'row' && tagFamily('ds telemetry id x/y').fam === 'row');
	check('tagFamily: the gateway\'s own marker is a row', tagFamily('(malformed row)').fam === 'row');
});

section('the file set: live, rotated, nested, and Daimond\'s text logs');
await part(() => {
	const home = fresh();
	const m = path.join(home, 'mirror');
	for (const n of [
		'b.ndjson', 'a.ndjson', 'a.ndjson.20260902T000000Z', 'a.ndjson.20260901T000000Z-1', 'a.ndjson.20260901T000000Z',
		'client/s1.ndjson', 'peer/p.ndjson', 'peer/p.ndjson.20260801T000000Z', 'legacy.log', 'legacy.log.1', 'a.ndjson.1',
		'readme.txt', 'a.ndjson.20260902T000000Z.tmp',
	]) put(home, 'mirror/' + n, '');
	const names = mirrorFiles(m).map((f) => path.relative(m, f.file));
	const at = (n) => names.indexOf(n);
	check('mirrorFiles: a rotated file comes before the live one it came from',
		at('a.ndjson.20260901T000000Z') < at('a.ndjson.20260901T000000Z-1') && at('a.ndjson.20260901T000000Z-1') < at('a.ndjson.20260902T000000Z')
		&& at('a.ndjson.20260902T000000Z') < at('a.ndjson'), names.join(' '));
	check('mirrorFiles: every rotated file precedes every live one', Math.max(...['a.ndjson.20260902T000000Z', 'peer/p.ndjson.20260801T000000Z', 'legacy.log.1'].map(at))
		< Math.min(...['a.ndjson', 'b.ndjson', 'client/s1.ndjson', 'peer/p.ndjson', 'legacy.log'].map(at)));
	check('mirrorFiles: a pre-stamp .ndjson.1 is kept, oldest', at('a.ndjson.1') >= 0 && at('a.ndjson.1') < at('a.ndjson.20260901T000000Z'));
	check('mirrorFiles: nested directories are walked', at('client/s1.ndjson') >= 0 && at('peer/p.ndjson') >= 0);
	check('mirrorFiles: a stray file and a temporary are not lens files', at('readme.txt') < 0 && at('a.ndjson.20260902T000000Z.tmp') < 0);
	const fmt = Object.fromEntries(mirrorFiles(m).map((f) => [path.relative(m, f.file), f.fmt]));
	check('mirrorFiles: .log and .log.1 are Daimond\'s text blocks, .ndjson the line', fmt['legacy.log'] === 'text' && fmt['legacy.log.1'] === 'text' && fmt['a.ndjson'] === 'lines');
	check('mirrorFiles: a missing directory is empty', mirrorFiles(path.join(home, 'nowhere')).length === 0);
});

// ═════════════════════════════════════════════════════════════════════
section('ingest: a mirror becomes an archive');
await part(() => {
	const home = fresh();
	const R = createReader({ home, now: () => NOW });
	put(home, 'mirror/k1.ndjson',
		EV(1000, 'devA', 1, 'turn.end', { turn: 't1', r: 3 }, 995)
		+ L(1000, 'devA', 991, 'election peer', 'x  y')
		+ L(1000, 'devA', 992, 'ev console', 'not json {'));
	const st = R.pull();
	check('pull: counts the event, the generic row and the malformed event', st.events === 1 && st.rows === 2, JSON.stringify(st));
	const recs = readNdjson(R.eventsFile('devA'));
	check('an ev row is filed as {src,device,bt,ts,kind,ev}, ts from the envelope',
		JSON.stringify(recs[0]) === JSON.stringify({ src: 'ev', device: 'devA', bt: 1000, ts: 995, kind: 'turn.end', ev: { v: 1, d: 'devA', n: 1, b: 'b1', t: 995, turn: 't1', r: 3 } }),
		JSON.stringify(recs[0]));
	check('a generic row is kept verbatim as {src:row,tag,data}', recs[1].src === 'row' && recs[1].tag === 'election peer' && recs[1].data === 'x  y');
	check('an ev row whose data is no JSON is kept and marked malformed', recs[2].malformed === true && recs[2].data === 'not json {');
	check('pull: the archive and the state exist', fs.existsSync(R.dirs.state) && fs.existsSync(R.dirs.archive));

	const again = R.pull();
	check('a second pull over the same mirror ingests nothing', again.events === 0 && again.rows === 0 && readNdjson(R.eventsFile('devA')).length === 3, JSON.stringify(again));
	const R2 = createReader({ home, now: () => NOW });
	check('a new reader on the same home remembers (state is on disk)', R2.pull().events === 0 && readNdjson(R2.eventsFile('devA')).length === 3);
});

await part(() => {
	const home = fresh();
	const R = createReader({ home, now: () => NOW });
	// The same (d,n) arrives in the live file and again in a rotated copy: one record.
	// The client retries a post whose reply it lost, so the same (d,n) arrives again at a later receive time.
	put(home, 'mirror/k.ndjson', EV(5000, 'devA', 7, 'boot', { a: 1 }) + EV(5000, 'devA', 8, 'beat', {}));
	put(home, 'mirror/k.ndjson.20260101T000000Z', EV(5100, 'devA', 7, 'boot', { a: 1 }) + EV(4000, 'devA', 6, 'beat', {}));
	const st = R.pull();
	check('events dedupe on (d,n), whichever file redelivers them', st.events === 3 && st.dupEvents === 1, JSON.stringify(st));
	// A device that is re-keyed writes under a new envelope id into the same file name: its n starts again.
	put(home, 'mirror/k2.ndjson', L(6000, 'devA', 5995, 'ev boot', evData('devB', 7, 5995, {})));
	check('the dedupe axis is the envelope device, so a re-keyed device\'s n=7 is new', R.pull().events === 1);
});

await part(() => {
	const home = fresh();
	const R = createReader({ home, now: () => NOW });
	// A torn tail, all in one batch (one receive time): the rest of the line arrives on the next pull.
	const l1 = EV(9000, 'devA', 1, 'a', {}), l2 = EV(9000, 'devA', 2, 'b', {}), l3 = EV(9000, 'devA', 3, 'c', {}), l4 = EV(9000, 'devA', 4, 'd', {});
	const f = put(home, 'mirror/k.ndjson', l1 + l2 + l3.slice(0, 35));
	const s1 = R.pull();
	check('torn tail: the whole lines are ingested and the torn one is counted', s1.events === 2 && s1.bad === 1, JSON.stringify(s1));
	fs.writeFileSync(f, l1 + l2 + l3 + l4);
	const s2 = R.pull();
	const kinds = readNdjson(R.eventsFile('devA')).map((r) => r.kind);
	check('torn tail: the completed line and the next one are ingested once, and nothing twice',
		s2.events === 2 && kinds.join() === 'a,b,c,d', kinds.join());
});

await part(() => {
	const home = fresh();
	const R = createReader({ home, now: () => NOW });
	// Two files of ONE device an hour apart, the later one sorting first: a device-wide watermark would
	// call the earlier file's rows old and drop them.
	put(home, 'mirror/a-sess.ndjson', L(NOW, 'dev', NOW - 5, 'tick', 'late one'));
	put(home, 'mirror/b-sess.ndjson', L(NOW - 3_600_000, 'dev', NOW - 3_600_005, 'tick', 'early one'));
	const st = R.pull();
	check('a device with several session files keeps the rows of the older one', st.rows === 2, JSON.stringify(st));
});

await part(() => {
	const home = fresh();
	const R = createReader({ home, now: () => NOW });
	const rot = put(home, 'mirror/k.ndjson.20260101T000000Z', EV(100, 'devA', 1, 'a', {}));
	put(home, 'mirror/k.ndjson', EV(900_000_000, 'devA', 2, 'b', {}));
	const s1 = R.pull();
	check('a rotated file is read, then sealed', s1.events === 2 && (s1.sealed || 0) === 0, JSON.stringify(s1));
	const s2 = R.pull();
	check('the next pull does not read a sealed file again', s2.sealed === 1 && s2.events === 0, JSON.stringify(s2));
	add(rot, EV(899_999_000, 'devA', 3, 'c', {}));
	const s3 = R.pull();
	check('a sealed file that has changed size is read again', s3.events === 1 && s3.sealed === 0, JSON.stringify(s3));
});

section('ingest: Daimond\'s text blocks reach the same archive');
await part(() => {
	const rows = [
		{ ts: 995, tag: 'ev turn.end', data: evData('devA', 1, 995, { turn: 't1' }) },
		{ ts: 991, tag: 'election peer', data: 'x  y' },
	];
	const hN = fresh(), hT = fresh();
	put(hN, 'mirror/acc-devA.ndjson', rows.map((r) => L(1000, 'devA', r.ts, r.tag, r.data)).join(''));
	put(hT, 'mirror/acc-devA.log', '===== 1000 account=acc device=devA rows=2 =====\n' + rows.map((r) => '  ' + r.ts + '  ' + r.tag + '  ' + r.data).join('\n') + '\n');
	const a = createReader({ home: hN, now: () => NOW }), b = createReader({ home: hT, now: () => NOW });
	a.pull(); b.pull();
	const ea = readNdjson(a.eventsFile('devA')), eb = readNdjson(b.eventsFile('devA'));
	check('the same rows as a block and as lines give the same archive records', ea.length === 2 && JSON.stringify(ea) === JSON.stringify(eb), JSON.stringify(eb));
	let seen = [];
	eachBlock(path.join(hT, 'mirror/acc-devA.log'), (blk) => seen.push(blk));
	check('eachBlock: header fields, and a tag and data split at the first double space',
		seen.length === 1 && seen[0].bt === 1000 && seen[0].account === 'acc' && seen[0].device === 'devA'
		&& seen[0].rows[1].tag === 'election peer' && seen[0].rows[1].data === 'x  y', JSON.stringify(seen));
	check('a text block already ingested is not ingested again', b.pull().skipped >= 1 && readNdjson(b.eventsFile('devA')).length === 2);
});

section('ingest: chunk sets assemble into telemetry ticks and snapshots');
await part(() => {
	const home = fresh();
	const R = createReader({
		home, now: () => NOW,
		tick:    (b) => ({ iso: b.iso || null, stats: b.stats || null, ledger: b.ledger || [] }),
		prepare: (b) => { if (b.config) b.config = createRedactor({ loose: true }).value(b.config); return b; },
		index:   (b) => ({ chats: Array.isArray(b.chats) ? b.chats.length : 0 }),
	});
	const bundle = { ts: 5000, iso: '1970', stats: { models: [{ model: 'm', usd: 1 }] }, ledger: [{ t: 1, m: 'm' }] };
	const enc = b64(bundle), third = Math.ceil(enc.length / 3);
	const part_ = (i) => enc.slice((i - 1) * third, i * third);
	const B = NOW - 100_000;
	const f = put(home, 'mirror/k.ndjson',
		L(B, 'devA', 1, 'ds telemetry T1 1/3', part_(1)) + L(B, 'devA', 2, 'ds telemetry T1 2/3', part_(2)));
	let st = R.pull();
	check('two of three chunks: the set is assembling, nothing filed yet',
		st.chunks === 2 && readNdjson(R.telFile('devA')).length === 0 && R.pendingSets().length === 1 && R.pendingSets()[0].have === 2, JSON.stringify(st));
	add(f, L(B + 100, 'devA', 3, 'ds telemetry T1 3/3', part_(3)) + L(B + 100, 'devA', 3, 'ds telemetry T1 3/3', part_(3)));
	st = R.pull();
	const tel = readNdjson(R.telFile('devA'));
	check('the last chunk completes it: one tick, the hook\'s fields, ts from the bundle',
		st.telemetry === 1 && tel.length === 1 && tel[0].id === 'T1' && tel[0].ts === 5000 && tel[0].ledger[0].t === 1 && tel[0].bt === B + 100, JSON.stringify(tel));
	check('a chunk delivered twice is counted once and then as a duplicate', st.chunks === 1 && st.dupChunks >= 1, JSON.stringify(st));
	check('the parts file is gone and nothing is left assembling', R.pendingSets().length === 0 && !fs.existsSync(R.partsFile('devA', 'telemetry', 'T1')));
	add(f, L(B + 200, 'devA', 4, 'ds telemetry T1 2/3', part_(2)));
	st = R.pull();
	check('a chunk of a set already filed (a retried post) opens no phantom set', st.dupChunks === 1 && R.pendingSets().length === 0 && readNdjson(R.telFile('devA')).length === 1, JSON.stringify(st));

	// A snapshot: its config goes through the prepare hook, its body is filed beside the index.
	const snap = { ts: 6000, iso: 'x', config: { passphrase: 'zebra horse battery', theme: 'dark' }, chats: [1, 2, 3], note: 'key ' + awsId() };
	put(home, 'mirror/k2.ndjson', L(B + 300, 'devA', 1, 'ds snapshot S1 1/1', b64(snap)));
	R.pull();
	const idx = readNdjson(R.snapIndexFile('devA'));
	check('a snapshot is indexed complete, with the hook\'s facts', idx.length === 1 && idx[0].complete === true && idx[0].chats === 3 && idx[0].id === 'S1', JSON.stringify(idx));
	const body = JSON.parse(fs.readFileSync(path.join(R.dirs.snaps, idx[0].body), 'utf8'));
	check('the snapshot body is filed with its config redacted by name', body.config.theme === 'dark' && !JSON.stringify(body.config).includes('battery'), JSON.stringify(body.config));
	check('and its free text scrubbed by content', !JSON.stringify(body).includes(awsId()), JSON.stringify(body.note));
});

await part(() => {
	const home = fresh();
	let now = NOW;
	const R = createReader({ home, now: () => now, pendingTtlMs: 3_600_000 });
	put(home, 'mirror/k.ndjson', L(now - 10, 'devA', 1, 'ds snapshot LOST 1/3', 'AAAA'));
	let st = R.pull();
	check('an incomplete set waits', st.chunks === 1 && R.pendingSets().length === 1 && st.gapped === 0);
	now += 3_600_001;
	st = R.pull();
	const idx = readNdjson(R.snapIndexFile('devA'));
	check('past the time-to-live it is closed off as a gap the index keeps', st.gapped === 1 && R.pendingSets().length === 0
		&& idx.length === 1 && idx[0].gap === true && idx[0].complete === false && idx[0].missing === '2-3', JSON.stringify(idx));
	put(home, 'mirror/k2.ndjson', L(now, 'devA', 2, 'ds snapshot LOST 2/3', 'BBBB'));
	st = R.pull();
	check('a straggler of a closed set does not reopen it', R.pendingSets().length === 0 && st.dupChunks === 1, JSON.stringify(st));
	put(home, 'mirror/k3.ndjson', L(now, 'devA', 3, 'ds snapshot BAD 1/1', 'not base64 of json!!'));
	st = R.pull();
	const idx2 = readNdjson(R.snapIndexFile('devA')).filter((r) => r.id === 'BAD');
	check('a set that will not decode is recorded broken and does not stop the pull', st.broken === 1 && idx2.length === 1 && !!idx2[0].broken, JSON.stringify(st));
});

section('ingest: a secret in a row is scrubbed again on the way to the archive');
await part(() => {
	const home = fresh();
	const sk = stripe(), tok = jwt(), key = awsId(), phrase = 'correct horse battery staple';
	const R = createReader({
		home, now: () => NOW,
		scrub:  createScrubber(),
		redact: createRedactor({ deny: ['draft'], test: (s) => s.includes(phrase), head: 0 }),
	});
	put(home, 'mirror/k.ndjson',
		EV(1000, 'devA', 1, 'console', { lvl: 'error', msg: 'POST failed with ' + sk + ' in the header' })
		+ EV(1000, 'devA', 2, 'state', { passphrase: 'hunter2hunter2', draft: 'abc', ok: 'fine', note: phrase })
		+ L(1000, 'devA', 3, 'election peer', 'token seen ' + tok)
		+ L(1000, 'devA', 4, 'ds snapshot S 1/1', b64({ ts: 1, text: 'id ' + key }))
		+ L(1000, 'devA', 5, 'election peer', 'said ' + phrase + ' aloud'));
	check('control: the mirror does hold every plant', holds(R.dirs.mirror, sk) && holds(R.dirs.mirror, tok) && holds(R.dirs.mirror, 'hunter2hunter2') && holds(R.dirs.mirror, phrase));
	R.pull();
	check('no plant reaches the archive (sk_test_, JWT, a named field, the caller\'s string)',
		!holds(R.dirs.archive, sk) && !holds(R.dirs.archive, tok) && !holds(R.dirs.archive, 'hunter2hunter2') && !holds(R.dirs.archive, phrase));
	check('and not the AWS id inside a snapshot body (the third place a row is filed), and not the phrase in a generic row', !holds(R.dirs.archive, key) && !holds(R.dirs.pending, key));
	const recs = readNdjson(R.eventsFile('devA'));
	check('a scrubbed value leaves a marker that names its shape', /stripe/.test(recs[0].ev.msg) && /jwt/.test(recs[2].data), recs[0].ev.msg);
	check('the deny-list and the by-name guard fingerprint, and the rest of the row stays legible',
		recs[1].ev.ok === 'fine' && recs[1].ev.draft !== 'abc' && recs[1].ev.passphrase !== 'hunter2hunter2' && recs[1].ev.v === 1 && recs[1].ev.n === 2, JSON.stringify(recs[1].ev));
	const before = fs.readFileSync(R.eventsFile('devA'), 'utf8');
	const R3 = createReader({ home, now: () => NOW });
	R3.appendLine(path.join(home, 'again.ndjson'), JSON.parse(before.split('\n')[0]));
	check('scrubbing is idempotent: running it over its own output changes nothing',
		fs.readFileSync(path.join(home, 'again.ndjson'), 'utf8') === before.split('\n')[0] + '\n');
});

await part(() => {
	const home = fresh();
	const R = createReader({ home, now: () => NOW });
	const planted = stripe();
	put(home, 'mirror/k.ndjson', L(1, 'devA', 1, 'election peer', 'plain ' + planted));
	R.pull();
	check('with no scrubber given, the default scrubber still runs (the guard is not opt-in)', !holds(R.dirs.archive, planted));
});

section('ingest: the pull command');
await part(() => {
	const home = fresh();
	const mirror = path.join(home, 'mirror');
	const writer = (n, text) => ['node', '-e', `require('fs').mkdirSync(${JSON.stringify(mirror)},{recursive:true});require('fs').writeFileSync(${JSON.stringify(path.join(mirror, n))},${JSON.stringify(text)})`];
	const R = createReader({ home, now: () => NOW, pull: [writer('one.ndjson', L(1, 'devA', 1, 'election peer', 'one')), writer('two.ndjson', L(2, 'devB', 2, 'election peer', 'two'))] });
	const st = R.pull();
	check('a list of argv commands is run in order, and what they fetch is ingested', st.pull === 'ok' && st.rows === 2, JSON.stringify(st));
	check('--no-pull skips the command and still ingests the mirror',
		createReader({ home: fresh(), now: () => NOW, pull: [['node', '-e', 'process.exit(9)']] }).pull({ noPull: true }).pull === 'skipped');

	const h2 = fresh();
	const bad = createReader({ home: h2, now: () => NOW, pull: [['node', '-e', 'process.stderr.write("no route\\n");process.exit(3)']] });
	put(h2, 'mirror/k.ndjson', L(1, 'devA', 1, 'election peer', 'kept'));
	const sb = bad.pull();
	check('a failing command is reported, and the mirror is still ingested', /^failed: /.test(sb.pull) && sb.rows === 1, JSON.stringify(sb));
	check('a missing program is a failure too, not a throw', /^failed: /.test(createReader({ home: fresh(), pull: [['no-such-program-xyz']] }).pull().pull));
	check('no command at all is skipped', createReader({ home: fresh() }).pull().pull === 'skipped');
	let called = 0;
	const fn = createReader({ home: fresh(), pull: () => { called += 1; } });
	check('a function pull is called once', fn.pull().pull === 'ok' && called === 1);
	check('a function that throws is a failure', /^failed: boom/.test(createReader({ home: fresh(), pull: () => { throw new Error('boom'); } }).pull().pull));
	check('a lone argv array is one command', createReader({ home: fresh(), pull: ['node', '-e', '0'] }).pull().pull === 'ok');
});

// ═════════════════════════════════════════════════════════════════════
section('filters: since, device, tag');
await part(() => {
	const home = fresh();
	let names = { dev1aaaa: 'argonaut' };
	const R = createReader({ home, now: () => NOW, nameFor: (d) => names[d] || '' });
	put(home, 'mirror/k.ndjson',
		EV(1000, 'dev1aaaa', 1, 'boot', {}, 1000) + EV(2000, 'dev1aaaa', 2, 'screen', { view: 'home' }, 2000)
		+ EV(3000, 'dev2bbbb', 1, 'boot', {}, 3000) + L(4000, 'dev2bbbb', 4000, 'election peer', 'handoff'));
	R.pull();
	// A device with only a telemetry file and one with only a snapshot index are devices too, and a
	// row from one that sorts first by name must not come first by time.
	put(home, 'archive/dev0zzzz.telemetry.ndjson', JSON.stringify({ id: 't', device: 'dev0zzzz', bt: 2500, ts: 2500 }) + '\n');
	put(home, 'archive/dev3cccc.snapshots.ndjson', JSON.stringify({ id: 's', device: 'dev3cccc', bt: 1500, ts: 1500 }) + '\n');
	check('archiveDevices lists devices by file name, sorted, whichever of the three files they have',
		JSON.stringify(R.archiveDevices()) === '["dev0zzzz","dev1aaaa","dev2bbbb","dev3cccc"]', JSON.stringify(R.archiveDevices()));
	check('loadTelemetry: the telemetry of every device, and loadSnapshotIndex its own file',
		R.loadTelemetry(null, 0).length === 1 && R.loadSnapshotIndex(null).length === 1 && R.loadSnapshotIndex('dev3')[0].id === 's');
	check('loadEvents: all, in time order', R.loadEvents(null, 0).map((r) => r.ts).join() === '1000,2000,3000,4000');
	put(home, 'archive/dev0zzzz.events.ndjson', JSON.stringify({ src: 'row', device: 'dev0zzzz', bt: 2500, ts: 2500, tag: 'x', data: '' }) + '\n');
	check('loadEvents: one time order across devices, not file order', R.loadEvents(null, 0).map((r) => r.ts).join() === '1000,2000,2500,3000,4000');
	check('loadTelemetry: sorted by time across devices too', (() => {
		put(home, 'archive/dev1aaaa.telemetry.ndjson', JSON.stringify({ id: 'a', device: 'dev1aaaa', bt: 3000, ts: 3000 }) + '\n' + JSON.stringify({ id: 'b', device: 'dev1aaaa', bt: 1000, ts: 1000 }) + '\n');
		return R.loadTelemetry(null, 0).map((r) => r.ts).join() === '1000,2500,3000';
	})());
	// The next checks count the four ingested rows and the one added above.
	check('loadEvents: since is a floor', R.loadEvents(null, 2600).map((r) => r.ts).join() === '3000,4000');
	check('--device by id prefix', R.loadEvents('dev1', 0).length === 2 && R.loadEvents('dev2b', 0).length === 2);
	check('--device by id prefix both ways: a longer id still names the device', R.wantDevice('dev1aaaa', 'dev1aaaa-and-more') === true);
	check('--device by roster name, case-insensitive substring', R.wantDevice('dev1aaaa', 'ARGO') === true && R.wantDevice('dev2bbbb', 'argo') === false);
	check('--device that matches nothing finds nothing', R.loadEvents('zzz', 0).length === 0);
	check('no filter means every device', R.wantDevice('x', null) === true && R.wantDevice('x', '') === true);
	const c = R.coverage(null);
	check('coverage: the earliest ev is the tick coverage, the earliest row of any kind the row coverage',
		c.ticks === 1000 && c.rows === 1000 && c.byDevice.dev2bbbb.ticks === 3000 && c.byDevice.dev2bbbb.rows >= 1 , JSON.stringify(c));

	const rows = R.loadEvents(null, 0);
	check('filterRows: by kind (the ev kind or the tag)', RD.filterRows(rows, { kind: 'boot' }).length === 2 && RD.filterRows(rows, { kind: 'election peer' }).length === 1);
	check('filterRows: by a pattern over the body', RD.filterRows(rows, { grep: 'home' }).length === 1 && RD.filterRows(rows, { grep: /handoff/ }).length === 1);
	check('filterRows: by window and device together', RD.filterRows(rows, { since: 1500, device: 'dev2' }).length === 2);
	check('filterRows: no criteria keeps everything and copies nothing it should not', RD.filterRows(rows, {}).length === rows.length && rows.length === 5);
});

// ═════════════════════════════════════════════════════════════════════
section('the timeline: client rows, frames and the peer log, joined');
await part(() => {
	const ev = (device, bt, ts, kind, p) => ({ src: 'ev', device, bt, ts, kind, ev: Object.assign({ v: 1, d: device, n: ts, t: ts }, p) });
	const row = (device, bt, ts, tag, o) => ({ src: 'row', device, bt, ts, tag, data: JSON.stringify(o) });
	const recs = [
		ev('pageA', 2000, 1990, 'screen', { sid: 'S1', view: 'join' }),
		row('peer', 2001, 2001, 'ev frame.in', { run: 'R1', conn: 7, sid: 'S1', op: 'Hello' }),
		row('peer', 2003, 2003, 'ev log', { run: 'R1', conn: 7, msg: 'bound' }),
		row('peer', 2002, 2002, 'ev frame.out', { run: 'R1', conn: 7, sid: 'S1', op: 'Bound' }),
		ev('pageB', 2002, 2000, 'screen', { sid: 'S2', view: 'home' }),
		row('peer', 2002, 2002.5, 'ev frame.in', { run: 'R1', conn: 8, sid: 'S2', op: 'Hello' }),
		row('peer', 1500, 1500, 'ev frame.in', { run: 'R0', conn: 7, sid: 'S0', op: 'Hello' }),
		row('peer', 2004, 2004, 'ev log', { run: 'R1', msg: 'tick' }),
		row('peer', 2005, 2005, 'ev log', 'not an object'),
	];
	const tl = timeline(recs);
	check('timeline: every record, ordered by receive time and then the client\'s own clock',
		tl.length === 9 && tl.every((e, i) => i === 0 || e.bt > tl[i - 1].bt || (e.bt === tl[i - 1].bt && e.ts >= tl[i - 1].ts)), tl.map((e) => e.bt + '/' + e.ts).join(' '));
	check('timeline: run, conn and sid come from the ev payload or from the row\'s JSON data',
		tl.find((e) => e.label === 'screen' && e.device === 'pageA').sid === 'S1' && tl.find((e) => e.label === 'ev frame.out').conn === 7 && tl.find((e) => e.label === 'ev frame.out').run === 'R1');
	check('timeline: a row whose data is not an object has no ids and does not throw', tl[tl.length - 1].sid == null && tl[tl.length - 1].conn == null);
	check('timeline: the label is the ev kind, or the tag of a row', tl.find((e) => e.device === 'pageA').label === 'screen' && tl.find((e) => e.sid === 'S0').label === 'ev frame.in');

	const s1 = session(tl, { sid: 'S1' });
	check('session by sid: the page\'s own rows, its frames both ways, and the peer log line of its connection',
		s1.length === 4 && s1.map((e) => e.label).join() === 'screen,ev frame.in,ev frame.out,ev log', s1.map((e) => e.label).join());
	check('session by sid: another page\'s rows and an older run\'s conn 7 are not in it', !s1.some((e) => e.sid === 'S2' || e.run === 'R0'));
	const c7 = session(tl, { conn: 7, run: 'R1' });
	check('session by conn and run: the connection\'s rows, and the page rows of the sid it carried', c7.length === 4);
	check('session by conn alone matches any run', session(tl, { conn: 7 }).length === 5);
	check('session by run: that process\'s rows only', session(tl, { run: 'R1' }).every((e) => e.run === 'R1') && session(tl, { run: 'R1' }).length === 5);
	check('session of an id nothing carries is empty', session(tl, { sid: 'nope' }).length === 0);
	check('session with no key is empty, not everything', session(tl, {}).length === 0);
	const custom = timeline([{ src: 'ev', device: 'p', bt: 1, ts: 1, kind: 'k', ev: { session: 'X', c: 3, proc: 'P' } }], { fields: { sid: 'session', conn: 'c', run: 'proc' } });
	check('timeline: the field names are the caller\'s to give', custom[0].sid === 'X' && custom[0].conn === 3 && custom[0].run === 'P');
});

// ═════════════════════════════════════════════════════════════════════
section('digest: at most 2 KB, enforced here');
await part(() => {
	const utf8 = (s) => Buffer.byteLength(s, 'utf8');
	const lines = ['title line'];
	for (let i = 0; i < 400; i++) lines.push('line ' + i + ' ' + 'x'.repeat(60));
	lines.push('last line');
	const out = capBytes(lines, 2048);
	check('capBytes: under the cap whatever the input', utf8(out) <= 2048, String(utf8(out)));
	check('capBytes: the first and the last line survive, and the cut says what it withheld',
		out.split('\n')[0] === 'title line' && out.endsWith('last line') && /withheld/.test(out), out.split('\n').slice(-3).join(' | '));
	check('capBytes: the title is always kept', out.startsWith('title line\n'));
	check('capBytes: a short list is returned whole', capBytes(['a', 'b'], 100) === 'a\nb');
	const wide = capBytes(['t', 'é'.repeat(5000), '😀'.repeat(2000)], 300);
	check('capBytes: one enormous line is clipped to fit, on a character boundary',
		utf8(wide) <= 300 && !wide.includes('�') && Buffer.from(wide, 'utf8').toString('utf8') === wide, String(utf8(wide)));
	check('capBytes: a tiny cap still holds', utf8(capBytes(['abcdefghij', 'klm'], 5)) <= 5 && utf8(capBytes(['x'], 0)) === 0);
	check('capBytes: no lines, no output', capBytes([], 100) === '');

	// An oversized archive, through the reader: 60 devices, thousands of rows, errors with long multibyte text.
	const recs = [];
	for (let i = 0; i < 6000; i++) {
		const d = 'device' + String(i % 60).padStart(4, '0') + 'abcdef';
		recs.push(i % 5 === 0
			? { src: 'ev', device: d, bt: 1000 + i, ts: 1000 + i, kind: 'fetch.fail', ev: { msg: 'request éè failed ' + i + ' ' + 'z'.repeat(300) } }
			: { src: 'ev', device: d, bt: 1000 + i, ts: 1000 + i, kind: 'screen', ev: { view: 'v' + (i % 40) } });
	}
	const d1 = digest(recs, { now: 1000 + 6000, title: 'lens' });
	check('digest: an oversized archive is under 2 KB', utf8(d1) <= 2048, String(utf8(d1)));
	check('digest: it still has its title, and counts the devices it saw', d1.startsWith('lens') && /60 device\(s\)/.test(d1), d1);
	const d2 = digest(recs, { now: 7000, title: 'lens', max: 1_000_000, extra: () => Array.from({ length: 80 }, (_, i) => 'extra line ' + i + ' ' + 'q'.repeat(50)) });
	check('digest: a caller cannot raise the cap above 2 KB', utf8(d2) <= 2048);
	const d3 = digest(recs, { now: 7000, title: 'lens', max: 600 });
	check('digest: a caller can lower it', utf8(d3) <= 600 && utf8(d3) > 0, String(utf8(d3)));
	const d4 = digest(recs, { now: 7000, title: 'lens', extra: () => Array.from({ length: 80 }, (_, i) => 'extra line ' + i + ' ' + 'q'.repeat(50)) });
	check('digest: a caller\'s extra lines cannot break the cap either, and the cut is announced',
		utf8(d4) <= 2048 && /withheld/.test(d4) && d4.startsWith('lens') && d4.split('\n')[1].startsWith('…'), d4.split('\n').slice(0, 3).join(' | '));

	const small = [
		{ src: 'ev', device: 'dev1aaaa', bt: 1000, ts: 1000, kind: 'boot', ev: {} },
		{ src: 'ev', device: 'dev1aaaa', bt: 2000, ts: 2000, kind: 'fetch.fail', ev: { msg: '/api/sync 409' } },
		{ src: 'row', device: 'peer', bt: 2500, ts: 2500, tag: 'election peer', data: 'x' },
	];
	const s = digest(small, { now: 3000, title: 'lens' });
	check('digest of a small archive says the devices, the counts and the error', /2 device/.test(s) && /dev1aaa/.test(s) && /\/api\/sync 409/.test(s) && /x1 fetch\.fail|fetch\.fail/.test(s), s);
	check('digest of nothing says so and is short', /0 device/.test(digest([], { now: 1, title: 'lens' })) && utf8(digest([], { now: 1, title: 'lens' })) < 200);
	const withExtra = digest(small, { now: 3000, title: 'lens', extra: () => ['EXTRA'] });
	check('digest: a caller\'s extra lines come after the device block and before the error list',
		withExtra.indexOf('dev1aaa') < withExtra.indexOf('EXTRA') && withExtra.indexOf('EXTRA') < withExtra.lastIndexOf('fetch.fail'), withExtra);
});

section('output helpers');
await part(() => {
	const real = console.log;
	let seen = '';
	console.log = (s) => { seen += s + '\n'; };
	try { emit(['a', 'b', 'c', 'd'], 3); emit(['x'], 0); emit(['p', 'q'], 5); } finally { console.log = real; }
	check('emit: past the cap it says how many lines it withheld', seen.split('\n')[0] === 'a' && /… 2 more line\(s\)/.test(seen), seen);
	check('emit: no cap prints everything, a short list prints whole', /^x$/m.test(seen) && /^p\nq$/m.test(seen));
	check('labels: num, hm, ago, covLabel, utcLabel, sinceLabel, windowLabel, dev7, idsMatch',
		RD.num(1500) === '2k' && RD.num(2_500_000) === '2.50M' && RD.num(3e9) === '3.00G' && RD.num('x') === '0'
		&& RD.utcLabel(Date.UTC(2026, 8, 1, 12, 30)) === '2026-09-01 12:30Z' && RD.utcLabel(0) === '?' && RD.hm(0) === '--:--:--'
		&& RD.dev7('abcdefghij') === 'abcdefg' && RD.dev7() === '?' && RD.idsMatch('abcdef0123456789zz', 'abcdef0123456789') && !RD.idsMatch('abc', 'xyz')
		&& RD.sinceLabel(0) === 'the start of the archive' && RD.windowLabel(5, 10).includes('archive start') && RD.windowLabel(10, 5) === RD.sinceLabel(10));
	const realNow = Date.now; Date.now = () => NOW;
	try { check('ago: seconds, minutes, hours, days', RD.ago(NOW - 30_000) === '30s' && RD.ago(NOW - 600_000) === '10m' && RD.ago(NOW - 7_200_000) === '2h' && RD.ago(NOW - 5 * 86_400_000) === '5d' && RD.ago(0) === 'never');
		check('covLabel: the clock inside the day, the date beyond it', RD.covLabel(NOW - 3_600_000).endsWith('Z') && RD.covLabel(NOW - 3_600_000).length === 6 && RD.covLabel(NOW - 3 * 86_400_000).length === 17 && RD.covLabel(0) === '?');
	} finally { Date.now = realNow; }
	const home = fresh();
	const f = path.join(home, 'x', 'state.json');
	writeJson(f, { a: 1 });
	check('writeJson goes through a temporary file and leaves none', JSON.parse(fs.readFileSync(f, 'utf8')).a === 1 && !fs.existsSync(f + '.tmp'));
	check('no process or environment is read by the core: a reader built with no env reads no env', (() => {
		const save = process.env.DAIMOND_LENS_HOME; process.env.DAIMOND_LENS_HOME = '/nonexistent/should-not-be-read';
		try { return createReader({ home: fresh() }).dirs.home.indexOf('nonexistent') < 0; } finally { if (save === undefined) delete process.env.DAIMOND_LENS_HOME; else process.env.DAIMOND_LENS_HOME = save; }
	})());
});

fs.rmSync(ROOT, { recursive: true, force: true });
finish('reader');
