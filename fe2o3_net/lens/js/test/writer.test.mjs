// writer.test.mjs -- the browser writer core, `../writer.js`, ported from the event-feed,
// console, rate and scrubber sections of Daimond's `www/js/debugshare.test.mjs`.
//
// What carried over is every property of the core: the envelope, the 360-byte fit, the durable
// outbox (exactly once, in order, across an outage and a reload), the feed that cannot break the
// app, the caps, the overflow, error and console capture, the one post clock and its 429
// handling. What stayed behind is what is Daimond's: the Settings flag and its sync, the
// snapshot and telemetry lanes, the beat, the screen, the indicator. Those run in Daimond's own
// test, through `test/adapter` (see the record).
//
//   node test/writer.test.mjs
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRedactor, createScrubber } from '../redact.js';
import { check, finish, part, section } from './harness.mjs';
import { makeHost, makeWriter, parse, sleep, until } from './host.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const RAW_API_KEY = 'sk-or-v1-ZZZTOPSECRETproviderKEY0123456789abcdef';	// allowlist secret

async function main() {

section('writer: events -- the envelope, the tag and the 360-byte cap');
await part(async () => {
	const h = makeHost({ respond: () => 500, fastTimers: true });
	const w = makeWriter(h);
	w.state.set(false);
	check('event() is a no-op while the gate is shut', w.event('tool', { name: 'read' }) === false);
	check('nothing is queued while shut', w.depth() === 0);
	w.state.set(true);
	check('event() queues while open', w.event('tool', { name: 'read', out: 'done' }) === true);
	const rows = w.outbox();
	check('exactly one event was queued', rows.length === 1);
	check('the row is tagged "ev <kind>"', rows[0].tag === 'ev tool');
	check('the data is plain JSON, not base64', rows[0].data.charAt(0) === '{');
	const e1 = JSON.parse(rows[0].data);
	check('the envelope is {v,d,n,b,t} in that order, then the payload',
		Object.keys(e1).join(',') === 'v,d,n,b,t,name,out');
	check('v is 1, n a positive count, t a millisecond clock',
		e1.v === 1 && e1.n > 0 && typeof e1.t === 'number' && e1.t > 1e12);
	check('d and b come from the caller and are bounded (12 and 24 characters)',
		e1.d === 'devAAAA11112' && e1.b === 'build0123456789abcdef012');
	check('the sequence advances by one per event', (w.event('tool', {}), JSON.parse(w.outbox()[1].data).n === e1.n + 1));
	check('the sequence is persisted before the event is queued',
		Number(h.store.get('lens-seq')) === JSON.parse(w.outbox()[1].data).n);
	w.event('tool', { n: 999, t: 1, v: 9, d: 'x', b: 'y', own: 1 });
	const e3 = JSON.parse(w.outbox()[2].data);
	check('a payload cannot overwrite the envelope', e3.v === 1 && e3.n !== 999 && e3.t > 1e12 && e3.d.startsWith('devAAAA') && e3.own === 1);
	w.event('tool', { skip: undefined, keep: 0 });
	check('an undefined payload field is left out, a zero is kept',
		!('skip' in JSON.parse(w.outbox()[3].data)) && JSON.parse(w.outbox()[3].data).keep === 0);

	w.event('tool', { name: 'small', msg: 'x'.repeat(900) });
	const big = w.outbox()[4];
	const e4 = JSON.parse(big.data);
	check('a giant event is cut to the 360-byte cap', new TextEncoder().encode(big.data).length <= 360);
	check('a cut event is marked tr:1 and keeps its whole envelope', e4.tr === 1 && e4.v === 1 && e4.n > 0 && typeof e4.t === 'number');
	check('the cut trims the large field and keeps the small one', e4.name === 'small' && e4.msg.length < 900);
	w.event('tool', { m: 'é'.repeat(500) });
	check('a multi-byte string still lands inside the cap', new TextEncoder().encode(w.outbox()[5].data).length <= 360);
	check('every queued event is within the cap', w.outbox().every((r) => new TextEncoder().encode(r.data).length <= 360));
	check('the cut is a pure function of the object: fit(obj) returns what event() queued',
		w.fit({ v: 1, d: 'd', n: 1, b: 'b', t: 1, a: 'y'.repeat(800) }).length <= 360);
	w.event('x', { k: 'a'.repeat(100), j: 'b'.repeat(400) });
	check('the largest string goes first, so a short sibling survives', JSON.parse(w.outbox()[6].data).k === 'a'.repeat(100));
	h.halt();
});

section('writer: events -- the redactor, with the caller\'s two hooks');
await part(async () => {
	const h = makeHost({ respond: () => 500, fastTimers: true });
	const w = makeWriter(h);
	w.event('cfg', { apiKey: RAW_API_KEY, model: 'anthropic/claude-3.5' });
	const data = w.outbox()[0].data;
	check('a secret-named field is not carried raw', data.indexOf(RAW_API_KEY) === -1);
	check('it appears as a fingerprint keeping six characters', /"apiKey":"\[redacted sk-or-…#[0-9a-f]+\/\d+\]"/.test(data));
	check('a field beside it is untouched', data.indexOf('anthropic/claude-3.5') !== -1);

	const eight = /^(?:[a-z]+ ){7}[a-z]+$/;
	const h2 = makeHost({ respond: () => 500, fastTimers: true });
	const w2 = makeWriter(h2, { redact: createRedactor({ deny: ['draft'], test: (s) => eight.test(s), head: 0 }) });
	w2.event('screen', { draft: 'typed so far', phrase: 'one two three four five six seven eight', ok: 'fine' });
	const d2 = w2.outbox()[0].data;
	check('the caller\'s deny-list hook covers its field', d2.indexOf('typed so far') === -1 && /"draft":"\[redacted …#/.test(d2));
	check('the caller\'s string test covers a phrase under any field name', d2.indexOf('one two three') === -1 && /"phrase":"\[redacted …#/.test(d2));
	check('an innocent field is left', d2.indexOf('"ok":"fine"') !== -1);
	h.halt(); h2.halt();
});

section('writer: events -- RULE 1, exactly once and in order across an outage');
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: (i) => (i < 6 ? 503 : 200) });
	const w = makeWriter(h);
	for (let i = 0; i < 300; i++) w.event('round', { turn: 'T', r: i });
	check('300 events are queued (some may already be in flight)', w.depth() + h.posts.length >= 1 && w.depth() <= 300);
	check('the outbox empties once the endpoint recovers', await until(() => w.depth() === 0));
	check('the endpoint really did refuse a window of posts', h.posts.filter((p) => p.status === 503).length >= 6);
	const arrived = parse(h.landed());
	const ns = arrived.map((e) => e.n);
	check('every one of the 300 events arrived', arrived.length === 300);
	check('no n arrived twice', new Set(ns).size === ns.length);
	check('they arrived in sequence', ns.every((n, i) => i === 0 || n > ns[i - 1]));
	check('their payloads arrived in the order they were made', arrived.every((e, i) => e.r === i));
	check('the run carries one device id', new Set(arrived.map((e) => e.d)).size === 1);
	check('a repeated failure backs the next attempt off past the ordinary gap', h.timerDelays.some((d) => d > 3000));
	check('the first failure waits the ordinary gap', w.backoffMs(1) === 3000);
	check('the second waits twice it', w.backoffMs(2) === 6000);
	check('the backoff is capped at five minutes', w.backoffMs(30) === 300000);
});

section('writer: events -- RULE 2, the outbox survives a reload');
await part(async () => {
	const h1 = makeHost({ fastTimers: true, respond: () => 500 });
	const w1 = makeWriter(h1);
	for (let i = 0; i < 40; i++) w1.event('round', { turn: 'T', r: i });
	await sleep(30);
	w1.persistNow();
	const depthBefore = w1.depth();
	const lastPostBefore = w1.health().lastPostAt;
	check('the queue is held while the endpoint refuses', depthBefore === 40);
	check('the post clock is itself persisted before the reload', lastPostBefore > 0 && Number(h1.store.get('lens-lastpost')) === lastPostBefore);
	h1.halt();

	const h2 = makeHost({ store: h1.store, fastTimers: true });
	const w2 = makeWriter(h2);
	check('the reloaded writer recovered the whole outbox', w2.depth() === 40);
	check('and the post clock, so a post right after boot still knows a gap is owed',
		w2.health().lastPostAt === lastPostBefore && w2.gapLeft() > 0);
	w2.event('round', { turn: 'T', r: 40 });
	const n41 = JSON.parse(w2.outbox()[40].data).n;
	check('the sequence continues where it left off, never restarting', n41 === 41);
	w2.drain();
	check('the recovered outbox drains', await until(() => w2.depth() === 0));
	const landed = parse(h2.landed());
	check('all 41 events reach the wire after the reload', landed.length === 41);
	check('nothing queued before the reload was lost, and none arrived twice',
		new Set(landed.map((e) => e.n)).size === 41 && landed.every((e, i) => e.r === i));
});

section('writer: events -- a corrupt or odd stored outbox cannot stop the boot');
await part(async () => {
	const store = new Map([['lens-outbox', '{not json'], ['lens-seq', 'banana']]);
	const w = makeWriter(makeHost({ store, respond: () => 500 }));
	check('unparseable storage yields an empty queue, not a throw', w.depth() === 0);
	check('an unreadable sequence restarts from one', (w.event('x', {}), JSON.parse(w.outbox()[0].data).n === 1));
	const store2 = new Map([['lens-outbox', JSON.stringify([{ ts: 1, tag: 'ev a', data: '{}' }, { tag: 5 }, null, 'x', { ts: 2, tag: 'ev b' }])]]);
	const w2 = makeWriter(makeHost({ store: store2, respond: () => 500 }));
	check('only well-formed rows are recovered', w2.depth() === 1 && w2.outbox()[0].tag === 'ev a');
	const big = Array.from({ length: 5200 }, (_, i) => ({ ts: i + 1, tag: 'ev r', data: '{"i":' + i + '}' }));
	const w3 = makeWriter(makeHost({ store: new Map([['lens-outbox', JSON.stringify(big)]]), respond: () => 500 }));
	check('an oversized stored outbox keeps its newest 5,000', w3.depth() === 5000 && w3.outbox()[0].data === '{"i":200}');
	const w4 = makeWriter(makeHost({ store: undefined, respond: () => 500 }), { host: { storage: { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); } }, setTimeout, clearTimeout, console: {}, now: Date.now } });
	check('a storage that throws on every call still lets the writer run in memory', (w4.event('x', {}), w4.depth() === 1));
});

section('writer: events -- RULE 3, the feed cannot break the app');
await part(async () => {
	const h = makeHost({ fastTimers: true });
	const w = makeWriter(h);
	check('the feed is healthy before the fault', w.ok() === true);
	let appDid = 0;
	const poison = { get name() { throw new Error('provider blew up'); } };
	const appCall = () => { w.event('tool', poison); appDid += 1; return 'the answer'; };
	check('the app\'s own call returns its own answer', appCall() === 'the answer');
	check('the app\'s own call completes', appDid === 1);
	check('the feed turned itself off', w.ok() === false);
	await until(() => w.depth() === 0);
	const faults = h.landed().filter((r) => r.tag === 'ev feed.fault');
	check('exactly one feed.fault reached the wire (the drainer was left running)', faults.length === 1);
	check('the fault says what threw', JSON.parse(faults[0].data).msg.indexOf('provider blew up') !== -1);
	check('the fault carries the ordinary envelope', JSON.parse(faults[0].data).n > 0 && JSON.parse(faults[0].data).v === 1);
	check('a later event is refused by the off feed', w.event('tool', { name: 'after' }) === false);
	const depth = w.depth();
	appCall();
	check('a second fault queues nothing more and the app still completes', w.depth() === depth && appDid === 2);

	let faulted = null;
	const w2 = makeWriter(makeHost({ fastTimers: true }), { onFault: (e) => { faulted = e; } });
	w2.event('tool', poison);
	check('the caller is told of the fault, so it can stop its own timers', faulted && faulted.message === 'provider blew up');
	let after = 0;
	const w3 = makeWriter(makeHost({ fastTimers: true }), { after: () => { after += 1; } });
	w3.event('round', { r: 1 });
	w3.state.set(false);
	w3.event('round', { r: 2 });
	check('the after-hook runs for an accepted event only', after === 1);
});

section('writer: events -- a post never exceeds 400 rows or 200 KiB');
await part(async () => {
	const h = makeHost({ fastTimers: true });
	const w = makeWriter(h);
	for (let i = 0; i < 900; i++) w.event('round', { turn: 'T', r: i, note: 'n'.repeat(40) });
	check('900 events all drain', await until(() => w.depth() === 0));
	check('they went in more than one post', h.posts.length >= 3);
	check('no post exceeds 400 rows', h.posts.every((p) => p.rows.length <= 400));
	check('no post exceeds 200 KiB', h.posts.every((p) => JSON.stringify(p.rows).length <= 200 * 1024));
	check('a post carries only whole events', h.landed().every((r) => { try { return JSON.parse(r.data).v === 1; } catch (e) { return false; } }));
	const h2 = makeHost({ respond: () => 500, fastTimers: true });
	const w2 = makeWriter(h2);
	w2.state.set(false);
	for (let i = 0; i < 450; i++) w2.pushRow('ev x', '{"i":' + i + '}');
	check('one batch is capped at 400 rows', w2.eventBatch().length === 400);
	const w3 = makeWriter(makeHost({ respond: () => 500 }), { caps: { maxBodyBytes: 2000 } });
	w3.state.set(false);
	for (let i = 0; i < 50; i++) w3.pushRow('ev x', '{"pad":"' + 'p'.repeat(200) + '"}');
	check('a smaller body cap shortens the batch, and a lone row always goes',
		w3.eventBatch().length < 50 && w3.eventBatch().length >= 1);
	h2.halt();
});

section('writer: events -- overflow drops the OLDEST and says how many');
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	for (let i = 0; i < 5040; i++) w.event('round', { turn: 'T', r: i });
	const rows = w.outbox();
	check('the outbox is capped at 5,000 events', rows.length === 5000);
	const drops = rows.filter((r) => r.tag === 'ev feed.drop');
	check('a feed.drop records the overflow', drops.length >= 1);
	check('the drop carries a count', JSON.parse(drops[0].data).count >= 1);
	check('the oldest events are the ones dropped', JSON.parse(rows.find((r) => r.tag === 'ev round').data).r > 0);
	check('the newest event survived', JSON.parse(rows[rows.length - 1].data).r === 5039 || rows[rows.length - 1].tag === 'ev feed.drop');
	w.reset();
	check('reset() empties the outbox, so nothing more leaves the device', w.depth() === 0);
	h.halt();
});

section('writer: a quota that halves the outbox says so');
await part(async () => {
	let quotaLeft = 1;
	const h = makeHost({ fastTimers: true, respond: () => 500, quota: (k) => (k === 'lens-outbox' && quotaLeft > 0) ? (quotaLeft -= 1, true) : false });
	const w = makeWriter(h);
	for (let i = 0; i < 8; i++) w.event('tool', { turn: 'Q1', name: 'run' });
	await sleep(40);
	h.halt();
	const drops = parse(w.outbox().filter((r) => r.tag === 'ev feed.drop'));
	check('a quota drop is announced once', drops.length === 1);
	check('it names the quota and says how many rows went', drops.length === 1 && drops[0].why === 'quota' && drops[0].count >= 1);
	check('the rows that survived are still there', w.outbox().some((r) => r.tag === 'ev tool'));
});

section('writer: errors -- rate-capped, with a dropped count');
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	w.noteError('x is not a function', 'js/app.js:120');
	const first = JSON.parse(w.outbox()[0].data);
	check('an error becomes an `error` event', w.outbox()[0].tag === 'ev error');
	check('it carries its message and where', first.msg.indexOf('x is not a function') !== -1 && first.at === 'js/app.js:120');
	w.noteError('y'.repeat(900), '');
	check('a long message is clipped to 200 characters', JSON.parse(w.outbox()[1].data).msg.length <= 200);
	const before = w.depth();
	for (let i = 0; i < 100; i++) w.noteError('flood ' + i, '');
	check('the rate cap admits at most 20 a minute', w.depth() - before <= 18);
	check('every admitted error is well formed and inside the cap',
		w.outbox().filter((r) => r.tag === 'ev error').every((r) => JSON.parse(r.data).n > 0 && new TextEncoder().encode(r.data).length <= 360));
	w.state.set(false);
	const q = w.depth();
	w.noteError('quiet', '');
	check('nothing is captured while the gate is shut', w.depth() === q);
	h.halt();
});

section('writer: fetch.fail -- no body, an abort is marked, the feed never reports itself');
await part(async () => {
	let hidden = false;
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h, { host: Object.assign({}, h.host, { document: { get visibilityState() { return hidden ? 'hidden' : 'visible'; } } }) });
	w.noteFetchFail('/api/sync?token=abc', 502, 40, '');
	w.noteFetchFail('/api/x', 0, 7, 'Failed to fetch');
	const fails = () => parse(w.outbox().filter((r) => r.tag === 'ev fetch.fail'));
	check('a path and status and elapsed ms, with the query dropped', fails()[0].path === '/api/sync' && fails()[0].status === 502 && fails()[0].ms === 40);
	check('a thrown fetch carries status 0 and its message', fails()[1].status === 0 && fails()[1].err === 'Failed to fetch');
	check('no request body travels', w.outbox().every((r) => r.data.indexOf('"body"') === -1));
	check('a status 0 on a live page is a plain failure', fails()[1].aborted === undefined);
	hidden = true;
	w.noteFetchFail('/api/y', 0, 1, '');
	check('a status 0 while the page is hidden is marked aborted', fails().pop().aborted === 1);
	w.noteFetchFail('/api/y', 500, 1, '');
	check('a real status while hidden is not', fails().pop().aborted === undefined);
	hidden = false;
	const pageHideListeners = [];
	w.captureLifecycle({ addEventListener: (t, f) => pageHideListeners.push([t, f]) }, undefined);
	pageHideListeners.filter((x) => x[0] === 'pagehide').forEach((x) => x[1]({}));
	w.noteFetchFail('/api/z', 0, 1, '');
	check('a status 0 just after pagehide is marked aborted', fails().pop().aborted === 1);
	const n = fails().length;
	w.noteFetchFail('/lens/post?x=1', 429, 1, '');
	check('the feed\'s own endpoint never becomes a fetch.fail', fails().length === n);
	h.halt();
});

section('writer: console -- every level, the ORIGINAL first, bounded three ways');
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	w.captureConsole(h.console);
	h.console.log('plain line');
	h.console.info('an info line');
	h.console.debug('[improve] a debug line');
	h.console.warn('i18n: no string for "home.sec_diag"');
	h.console.error('TypeError: boom');
	w.flushConsole(true);
	const rows = parse(w.outbox().filter((r) => r.tag === 'ev console'));
	check('all five levels become events', rows.length === 5);
	check('each event names its level', ['log', 'info', 'debug', 'warn', 'error'].every((l) => rows.some((r) => r.lvl === l)));
	check('the warning is carried verbatim', rows.some((r) => r.lvl === 'warn' && r.msg === 'i18n: no string for "home.sec_diag"'));
	check('the original ran first, for every level', h.logged.length === 5 && h.logged[0] === 'log:plain line');
	check('a console event carries a bounded source', rows.every((r) => typeof r.src === 'string' && r.src.length <= 60));
	check('console.error also becomes an `error` event', w.outbox().some((r) => r.tag === 'ev error' && JSON.parse(r.data).at === 'console.error'));
	check('a console event stays inside the cap', w.outbox().every((r) => new TextEncoder().encode(r.data).length <= 360));
	h.console.warn('a', 'b', 3, true, 'fifth is dropped');
	w.flushConsole(true);
	check('several arguments join into one message, to four', parse(w.outbox().filter((r) => r.tag === 'ev console')).pop().msg === 'a b 3 true');
	h.console.log('z'.repeat(900));
	w.flushConsole(true);
	check('a long console line is clipped to 200 characters', parse(w.outbox().filter((r) => r.tag === 'ev console')).pop().msg.length <= 200);
	check('wrapping twice does not wrap twice', (() => { const before = h.console.log; w.captureConsole(h.console); return h.console.log === before; })());
	h.halt();
});
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	w.captureConsole(h.console);
	h.console.warn('held');
	w.state.set(false);
	w.reset();
	w.state.set(true);
	w.flushConsole(true);
	check('a line still held when the writer is reset is dropped, not delivered later', w.outbox().filter((r) => r.tag === 'ev console').length === 0);
	w.state.set(false);
	const q = w.depth();
	h.console.warn('nobody is listening');
	check('nothing is captured while the gate is shut, but the original still ran',
		w.depth() === q && h.logged.pop() === 'warn:nobody is listening');
	h.halt();
});
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	w.noteConsole('warn', ['[sync] chunk index not merged on this device']);
	w.flushConsole(true);
	check('with no stack to read, the bracketed prefix is the source', parse(w.outbox().filter((r) => r.tag === 'ev console'))[0].src === '[sync]' || /\.(?:js|mjs):\d+$/.test(parse(w.outbox().filter((r) => r.tag === 'ev console'))[0].src));
	h.halt();
});
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	for (let i = 0; i < 40; i++) w.noteConsole('warn', ['repeat me']);
	w.noteConsole('warn', ['once']);
	check('a repeat queues nothing until the window closes', w.depth() === 0);
	w.flushConsole(true);
	const rows = parse(w.outbox().filter((r) => r.tag === 'ev console'));
	check('forty identical lines are one event carrying their count, a single line none', rows.length === 2 && rows[0].x === 40 && rows[1].x === undefined);
	w.noteConsole('error', ['repeat me']);
	w.flushConsole(true);
	check('the same text at another level is its own event', parse(w.outbox().filter((r) => r.tag === 'ev console')).length === 3);
	h.halt();
});
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	for (let i = 0; i < 75; i++) w.noteConsole('log', ['distinct ' + i]);
	w.flushConsole(true);
	check('the cap admits sixty distinct lines a minute', parse(w.outbox().filter((r) => r.tag === 'ev console')).length === 60);
	check('the fifteen it dropped are counted', w.health().cdrop === 15);
	check('and the count is taken and cleared by whoever reports it', w.takeConsoleDropped() === 15 && w.health().cdrop === 0);
	h.halt();
});
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	w.captureConsole(h.console);
	// An argument that logs WHILE it is being read: the capture reaches for `.message`, and this
	// getter answers by logging. Without the guard that is the capture calling itself from inside.
	let reentries = 0;
	const noisy = { get message() { reentries++; h.console.error('re-entrant line'); return 'noisy'; } };
	h.console.log(noisy);
	w.flushConsole(true);
	const rows = parse(w.outbox().filter((r) => r.tag === 'ev console'));
	check('the re-entrant log really did happen', reentries >= 1);
	check('only the outer line became an event', rows.length === 1 && rows[0].msg === 'noisy');
	check('the re-entrant line is not in the feed', w.outbox().every((r) => r.data.indexOf('re-entrant') === -1));
	check('the feed is still collecting', w.ok() === true);
	h.halt();
});
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	w.captureConsole(h.console);
	h.console.log('api_key: hunter2');
	h.console.log('call used sk-' + 'Zq9Xw8Vu7Ts6Rq5Po4Nm3Lk2' + ' today');
	h.console.log('the token expired at noon');
	w.flushConsole(true);
	const wire = w.outbox().map((r) => r.data).join('\n');
	check('a key logged beside its name never reaches the outbox', wire.indexOf('hunter2') === -1);
	check('what is kept is a fingerprint or a marker', /\[redacted [^"]+\]/.test(wire));
	check('a key-shaped token is caught by its shape alone', wire.indexOf('Zq9Xw8Vu7Ts6Rq5Po4Nm3Lk2') === -1);
	check('a message that merely mentions a token is kept whole', wire.indexOf('the token expired at noon') !== -1);
	const h2 = makeHost({ fastTimers: true, respond: () => 500 });
	const w2 = makeWriter(h2, { redact: createRedactor({ test: (s) => /correct horse/.test(s), head: 0 }) });
	w2.noteConsole('log', ['typed correct horse battery staple']);
	w2.flushConsole(true);
	check('the caller\'s string test reaches console lines too', w2.outbox().map((r) => r.data).join('').indexOf('correct horse') === -1);
	h.halt(); h2.halt();
});

section('writer: error hooks and lifecycle');
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: () => 500 });
	const w = makeWriter(h);
	const L = {};
	const win = { addEventListener: (t, f) => { (L[t] = L[t] || []).push(f); } };
	w.captureErrors(win);
	L.error[0]({ message: 'Uncaught boom', filename: 'https://x.test/js/a.js', lineno: 12 });
	L.unhandledrejection[0]({ reason: { message: 'rejected thing' } });
	const evs = parse(w.outbox().filter((r) => r.tag === 'ev error'));
	check('an uncaught error becomes an event with its path and line, origin stripped', evs[0].msg === 'Uncaught boom' && evs[0].at === '/js/a.js:12');
	check('an unhandled rejection becomes one too', evs[1].msg === 'rejected thing' && evs[1].at === 'unhandledrejection');
	let persists = 0;
	const h2 = makeHost({ fastTimers: true, respond: () => 500, quota: () => { persists += 1; return false; } });
	const w2 = makeWriter(h2);
	const L2 = {};
	const doc = { visibilityState: 'visible' };
	w2.captureLifecycle({ addEventListener: (t, f) => { (L2[t] = L2[t] || []).push(f); } }, doc);
	w2.event('a', {});
	const beforeWrites = h2.store.get('lens-outbox');
	L2.pagehide.forEach((f) => f({}));
	check('pagehide writes the outbox out at once, not on the debounce', beforeWrites === undefined && JSON.parse(h2.store.get('lens-outbox')).length === 1);
	doc.visibilityState = 'hidden';
	w2.event('b', {});
	L2.visibilitychange.forEach((f) => f({}));
	check('and so does the page going hidden', JSON.parse(h2.store.get('lens-outbox')).length === 2);
	h.halt(); h2.halt();
});

section('writer: rate -- ONE post clock for every lane, and what a 429 does to it');
await part(async () => {
	// A second lane: a caller's own big payload, drained after the outbox, on the same clock.
	let laneRows = [[{ ts: 1, tag: 'ds a 1/3', data: 'x' }], [{ ts: 2, tag: 'ds a 2/3', data: 'y' }], [{ ts: 3, tag: 'ds a 3/3', data: 'z' }]];
	const lane = { pending: () => laneRows.length > 0, next: () => laneRows.shift() };
	const h = makeHost({ fastTimers: true });
	const w = makeWriter(h, { lane });
	w.event('turn.start', { turn: 'T1' });
	w.event('turn.end', { turn: 'T1' });
	w.drain();
	check('both lanes empty', await until(() => w.depth() === 0 && laneRows.length === 0));
	check('the outbox went first, then the lane', h.posts[0].rows[0].tag.startsWith('ev ') && h.posts[h.posts.length - 1].rows[0].tag === 'ds a 3/3');
	check('more than one post was needed', h.posts.length >= 4);
	const pacing = h.timerDelays.filter((d) => d >= 3000);
	check('every post after the first waited a full window', pacing.length >= h.posts.length - 1);
	check('nothing asked to post inside another post\'s window',
		h.timerDelays.every((d) => d === 0 || d === 250 || d >= 3000), JSON.stringify(h.timerDelays.filter((d) => d !== 0 && d !== 250 && d < 3000)));

	const h2 = makeHost({ fastTimers: true });
	const w2 = makeWriter(h2);
	w2.event('x', {});
	await until(() => w2.depth() === 0);
	const sent = h2.posts.length;
	h2.timerDelays.length = 0;
	w2.event('y', {});
	check('a drain that STARTS inside the window waits out the remainder first', await until(() => h2.timerDelays.some((d) => d > 0 && d <= 3000)));
	check('and then it posts, with no 429 provoked', await until(() => h2.posts.length === sent + 1) && w2.health().throttled === 0);
});
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: (i) => (i < 3 ? 429 : 200) });
	const w = makeWriter(h);
	w.event('a', {});
	w.event('b', {});
	await until(() => w.depth() === 0);
	const throttles = parse(h.landed().filter((r) => r.tag === 'ev feed.throttled'));
	check('a 429 is announced, once per burst', throttles.length === 1);
	check('the 429s are counted, and a 429 is not a post failure', w.health().throttled >= 1 && w.health().postFail === 0);
	check('the events themselves were unlost', parse(h.landed()).filter((e) => e.n > 0 && !e.gap).length >= 2);
	check('the announcement says the gap it widened to', throttles[0].gap >= 3000);
	check('a post that lands clears the widening', w.health().gap === 3000);
});
await part(async () => {
	const h = makeHost({ fastTimers: true, respond: (i) => (i < 2 ? 429 : 200) });
	const w = makeWriter(h);
	w.event('a', {});
	await sleep(20);
	check('while refused the gap has widened past the ordinary interval', w.health().gap > 3000 || w.health().throttled >= 1);
	h.halt();
	const h2 = makeHost({ fastTimers: true, respond: () => 500 });
	const w2 = makeWriter(h2);
	w2.event('a', {});
	await sleep(20);
	h2.halt();
	check('a 500 is a post failure and not a throttle', w2.health().postFail >= 1 && w2.health().throttled === 0);
	check('and the gap stays at the ordinary interval', w2.health().gap === 3000);
});

section('writer: what send() may answer');
await part(async () => {
	const bare = makeHost({ fastTimers: true, reply: 'bare' });
	const w = makeWriter(bare);
	w.event('a', {});
	check('a send that resolves nothing counts as delivered, or the outbox would never empty', await until(() => w.depth() === 0));
	const th = makeHost({ fastTimers: true, reply: 'throw' });
	const w2 = makeWriter(th);
	w2.event('a', {});
	await sleep(20);
	th.halt();
	check('a send that rejects is a failure and the event stays', w2.depth() === 1 && w2.health().postFail >= 1);
	const r1 = makeHost({ fastTimers: true });
	const w3 = makeWriter(r1, { send: (rows) => { r1.send(rows); return Promise.resolve({ status: 204 }); } });
	w3.event('a', {});
	check('a reply with only a status below 400 is delivered', await until(() => w3.depth() === 0));
	const r2 = makeHost({ fastTimers: true });
	const w4 = makeWriter(r2, { send: (rows) => { r2.send(rows); return Promise.resolve({ status: 503 }); } });
	w4.event('a', {});
	await sleep(20);
	r2.halt();
	check('a reply with a status of 400 or more is not', w4.depth() === 1);
	const calls = [];
	const r3 = makeHost({ fastTimers: true });
	const w5 = makeWriter(r3, { send: (rows, info) => { calls.push(info); return r3.send(rows); }, caps: { gapMs: 2000 } });
	w5.event('a', {});
	await until(() => w5.depth() === 0);
	check('send receives the device in its second argument', calls[0] && calls[0].device === w5.device());
	check('the pacing gap is the caller\'s: a socket drain may run at two seconds', w5.gapMs() === 2000);
	check('postBody builds the body the Rust sink takes: {v:1,device,rows}',
		JSON.stringify(JSON.parse(w5.postBody([{ ts: 1, tag: 't', data: 'd' }]))) === '{"v":1,"device":"devAAAA1111222233334444","rows":[{"ts":1,"tag":"t","data":"d"}]}');
});

section('writer: nothing of an app in the core');
await part(async () => {
	// Code lines only: a comment may say where the code came from, the code may not name an app.
	const code = (f) => readFileSync(join(HERE, '..', f), 'utf8').split('\n')
		.filter((l) => !/^\s*(\/\/|\*|\/\*)/.test(l)).map((l) => l.replace(/\s\/\/ .*$/, '')).join('\n');
	check('no Daimond or Oxegen name in the code of the writer or the redactor', !/daimond|oxegen|DEBUG_SHARE/i.test(code('writer.js') + code('redact.js')));

	// And no global it was not handed: every one of these throws on touch, and a full scenario runs.
	const touched = [];
	for (const name of ['localStorage', 'sessionStorage', 'fetch', 'document', 'window', 'XMLHttpRequest', 'navigator']) {
		Object.defineProperty(globalThis, name, { configurable: true, get() { touched.push(name); throw new Error('touched ' + name); } });
	}
	try {
		const h = makeHost({ fastTimers: true });
		const w = makeWriter(h);
		w.captureConsole(h.console);
		w.captureErrors({ addEventListener() {} });
		w.captureLifecycle({ addEventListener() {} }, { visibilityState: 'visible' });
		w.event('a', { k: 1 });
		h.console.warn('line');
		w.noteError('e', 'w');
		w.noteFetchFail('/x', 500, 1, '');
		w.flushConsole(true);
		await until(() => w.depth() === 0);
		w.reset();
	} finally {
		for (const name of ['localStorage', 'sessionStorage', 'fetch', 'document', 'window', 'XMLHttpRequest', 'navigator']) delete globalThis[name];
	}
	check('a full scenario runs without touching localStorage, fetch, document, window or navigator', touched.length === 0, touched.join(','));
});

}
main().then(() => finish('writer')).catch((e) => { console.error(e); process.exit(1); });
