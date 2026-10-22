// A simulated tab for the writer core: a Map-backed `localStorage` that can be made to run out
// of quota, a send function that records every post and answers as told, a console per host
// (so a wrap on one cannot leak to the next), and timers that can be fired at once while still
// recording the delay that was asked for.
//
// This is the part of Daimond's `debugshare.test.mjs` harness the core needs. What it dropped is
// the fake DOM and the app's globals (`DaimondSync`, `DaimondIdentity`), which belong to Daimond.
import { createWriter } from '../writer.js';

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function until(cond, ms) {
	const end = Date.now() + (ms || 4000);
	while (Date.now() < end) { if (cond()) return true; await sleep(2); }
	return cond();
}

export function makeHost(cfg) {
	cfg = cfg || {};
	const store = cfg.store || new Map();
	let writes = 0;
	const storage = {
		getItem: (k) => (store.has(k) ? store.get(k) : null),
		setItem: (k, v) => {
			writes += 1;
			if (cfg.quota && cfg.quota(k, writes)) {
				const e = new Error('QuotaExceededError');
				e.name = 'QuotaExceededError';
				throw e;
			}
			store.set(k, String(v));
		},
		removeItem: (k) => store.delete(k),
	};
	const logged = [];
	const console_ = {};
	['log', 'info', 'debug', 'warn', 'error'].forEach((lvl) => {
		console_[lvl] = function () { logged.push(lvl + ':' + [].slice.call(arguments).join(' ')); };
	});
	const posts = [];
	let halted = false;
	const timerDelays = [];
	const setT = cfg.fastTimers
		? (fn, ms) => { timerDelays.push(ms); return setTimeout(fn, 1); }
		: (fn, ms) => { timerDelays.push(ms); return setTimeout(fn, ms); };
	// What the app's send function would do: one POST, answered with a status.
	const send = (rows) => {
		if (halted) return new Promise(() => {});
		const rec = { rows: rows.map((r) => Object.assign({}, r)), status: 200, at: Date.now() };
		rec.status = cfg.respond ? cfg.respond(posts.length, rec) : 200;
		posts.push(rec);
		if (cfg.reply === 'bare')  return Promise.resolve(undefined);
		if (cfg.reply === 'throw') return Promise.reject(new Error('network down'));
		return Promise.resolve({ ok: rec.status >= 200 && rec.status < 300, status: rec.status });
	};
	const host = { storage, setTimeout: setT, clearTimeout, console: console_, now: () => Date.now(), document: null };
	return {
		store, host, posts, logged, timerDelays, send, console: console_,
		halt: () => { halted = true; },
		landed: () => [].concat.apply([], posts.filter((p) => p.status >= 200 && p.status < 300).map((p) => p.rows)),
	};
}

export function makeWriter(h, over) {
	let on = true;
	const state = { on: () => on, set: (v) => { on = !!v; } };
	const w = createWriter(Object.assign({
		host:    h.host,
		keys:    { outbox: 'lens-outbox', seq: 'lens-seq', postAt: 'lens-lastpost' },
		device:  () => 'devAAAA1111222233334444',
		build:   () => 'build0123456789abcdef0123456789',
		enabled: () => on,
		send:    h.send,
		ownPath: '/lens/post',
	}, over || {}));
	w.state = state;
	return w;
}

export const parse = (rows) => rows.map((r) => JSON.parse(r.data));
