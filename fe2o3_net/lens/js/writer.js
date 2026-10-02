// writer.js -- the browser half of the debug lens: a durable outbox of small self-describing
// events, drained through a send function the app supplies.
//
// Extracted from Daimond's `www/js/debugshare.js` (2026-10-02) so that Daimond and Oxegen share
// one writer. It keeps Daimond's behaviour exactly, and everything the app alone knows is a
// parameter: where state is kept, how a post is made, who the device is, which names and strings
// are secret, which extra lane of rows drains behind the events. The Settings flag, the snapshot
// and telemetry lanes, the beat and the screen stay in the app.
//
// THE ENVELOPE. Every event is one row, `{ ts, tag: 'ev <kind>', data: <JSON, 360 bytes at most> }`,
// and its JSON begins `{ v, d, n, b, t }`: schema version, a short device id, a PERSISTED per-device
// sequence number, the build id, and the wall clock. `n` makes redelivery idempotent for the
// reader. `fe2o3_net::lens::Row` writes the same envelope in Rust and `lens::Sink` stores the row.
//
// THE OUTBOX. Rows are held in memory and in `storage` (5,000 at most, the oldest dropped with one
// `feed.drop` naming how many). A row leaves only when a post is known to have landed, so an
// outage, a reload or a killed tab loses nothing. A failed post backs the next attempt off,
// doubling to five minutes.
//
// ONE POST CLOCK. The receiving sink refuses a second post inside its floor (two seconds in
// Daimond's gateway). `lastPostAt` is stamped by every post of every lane, persisted, and
// consulted by every drain that starts, so the gap holds across lanes and across a reload. A 429
// that arrives anyway doubles the gap and is announced once per burst as `feed.throttled`.
//
// SELF-PROTECTION. Every entry point is wrapped. An exception inside the feed queues one
// `feed.fault` and turns collection off for the session, and the drainer keeps running so the
// fault itself reaches the sink. The caller's own code path continues: the app never breaks
// because the feed did.
//
// NO GLOBALS. The writer touches no `window`, `document`, `localStorage` or `fetch` that it was not
// handed in `cfg.host`, which is what lets one module serve a page, a test and a worker.
import {
	byteLen,
	createRedactor,
	createScrubber,
} from './redact.js';

export {
	SECRET_RE,
	SECRET_RE_LOOSE,
	byteLen,
	createRedactor,
	createScrubber,
	fingerprint,
	scrubDeep,
	scrubText,
	secretName,
} from './redact.js';

// ── The caps ─────────────────────────────────────────────────────────

// Mirrored from Daimond's gateway handler with a margin, so a post is never refused for size and
// a row is never clipped mid-payload. A caller overrides any of them through `cfg.caps`.
const CAPS = {
	maxEventBytes:   360,			// an event's JSON; the sink's field cap is 400
	maxRowsPost:     400,			// rows in one post; the sink refuses past 1,000
	maxBodyBytes:    200 * 1024,	// one post's body; the sink refuses past 256 KiB
	outboxCap:       5000,			// events held before the oldest are dropped
	gapMs:           3000,			// the minimum gap between ANY two posts, all lanes
	backoffMaxMs:    300000,		// the ceiling on repeated-failure backoff
	persistMs:       250,			// trailing debounce on writing the outbox out
	errorsPerMin:    20,			// error-capture rate cap, with a `dropped` count
	errWindowMs:     60000,
	consolePerMin:   60,			// distinct console lines admitted per window
	consoleWindowMs: 60000,
	consoleDedupeMs: 60000,			// an identical (level, message) inside this is one event
	maxMsgChars:     200,			// an error or console message, clipped
	maxSrcChars:     60,			// and where it came from
	maxDevChars:     12,			// envelope fields are bounded so they never crowd out the payload
	maxBuildChars:   24,
	unloadGraceMs:   2000,			// a status 0 this soon after pagehide is an abandoned call
};

const LEVELS = ['log', 'info', 'debug', 'warn', 'error'];

// A message that looks like it carries a secret VALUE. A name sitting against a value, or a
// provider-key-shaped run, is treated as live; a message that merely mentions a token is not.
const SECRET_TEXT_RE  = /(?:api[_-]?key|token|secret|passphrase|password|master[_-]?key|mnemonic|private[_-]?key|salt|sealed|wrapped)["'\s]*[:=]/i;
const SECRET_SHAPE_RE = /\b(?:sk|pk|ghp|xox[abps])[-_][A-Za-z0-9_-]{12,}/;

const ENVELOPE = { v: 1, d: 1, n: 1, b: 1, t: 1 };

/// Does this console message look like it carries a secret value?
export function looksSecret(msg) {
	return SECRET_TEXT_RE.test(msg) || SECRET_SHAPE_RE.test(msg);
}

/// Creates a writer. `cfg`:
///
///   host      `{ storage, setTimeout, clearTimeout, console, now, document, Error }`, each defaulting to
///             the global of that name where one exists. `storage` is `getItem`/`setItem`.
///   keys      `{ outbox, seq, postAt }`, the storage key names.
///   device    `() -> string`, the device id (clipped to 12 characters in the envelope).
///   build     `() -> string`, the build id (24 characters).
///   enabled   `() -> bool`, the gate; while it answers false nothing is collected or sent.
///   send      `(rows, { device }) -> Promise`, how one post is made. It may resolve a Response-like
///             `{ ok, status }`, a bare `{ status }`, or nothing (delivered); a rejection is a failure.
///   redact    a `createRedactor` value, applied to every event before it is cut.
///   scrub     a `createScrubber` value, applied to free text before it is cut.
///   extra     `(kind) -> object | null`, fields added after the envelope for that kind.
///   lane      `{ pending() -> bool, next() -> rows }`, a second lane drained after the outbox, on the
///             same post clock. Its posts may be lost without harm: the next one replaces them.
///   ownPath   the path of the feed's own endpoint, which `noteFetchFail` never reports.
///   skipFrames  substrings of stack frames that are the capture's own, skipped when naming a source.
///   after     `(kind, payload)`, run inside the guard after an accepted event.
///   onFault   `(error)`, run when the feed faults, so the app can stop its own timers.
///   caps      overrides for any entry of `CAPS`.
export function createWriter(cfg) {
	cfg = cfg || {};
	const host    = cfg.host || {};
	const caps    = Object.assign({}, CAPS, cfg.caps || {});
	const keys    = Object.assign({ outbox: 'lens-outbox', seq: 'lens-seq', postAt: 'lens-lastpost' }, cfg.keys || {});
	const redactor = cfg.redact || createRedactor();
	const scrub   = cfg.scrub || createScrubber();
	const storage = host.storage !== undefined ? host.storage : (typeof localStorage !== 'undefined' ? localStorage : null);
	const setT    = host.setTimeout || (typeof setTimeout !== 'undefined' ? setTimeout : null);
	const clearT  = host.clearTimeout || (typeof clearTimeout !== 'undefined' ? clearTimeout : function () {});
	const now     = host.now || Date.now;
	let doc       = host.document !== undefined ? host.document : (typeof document !== 'undefined' ? document : null);
	const skip    = (cfg.skipFrames || []).concat(['writer.js']);
	const gate    = typeof cfg.enabled === 'function' ? cfg.enabled : function () { return true; };

	// ── State ──
	let outbox        = [];
	let persistTimer  = null;
	let failStreak    = 0;		// consecutive failed event posts, for the backoff
	let dropOwed      = 0;		// events overflowed but not yet reported by feed.drop
	let dropping      = false;	// re-entry guard: a feed.drop must not recurse
	let feedOff       = false;	// the feed threw; collection is off for this session
	let draining      = false;
	let errWindowAt   = 0, errCount = 0, errDropped = 0;
	// The console lane: what is held for dedupe, what this window admitted, what it dropped.
	let conHeld       = {};
	let conWindowAt   = 0, conCount = 0, conDropped = 0;
	let conTimer      = null;
	let inConsole     = false, conFlushing = false;
	// The post clock, and what the sink has said about it.
	let lastPostAt    = ms(read(keys.postAt));
	let throttleStreak = 0, throttledCount = 0, postFailCount = 0, throttleBurst = false;
	let pagehideAt    = 0;

	// ── Storage, guarded ──
	function read(k)  { try { return storage ? storage.getItem(k) : null; } catch (e) { return null; } }
	function write(k, v) { try { if (storage) storage.setItem(k, v); } catch (e) { /* a flag nobody misses */ } }

	// A plain positive-millisecond number, or 0. Bignum-safe: a stamp that went through a sink can
	// arrive as an object with `toNumber`.
	function ms(v) {
		const n = (v && typeof v.toNumber === 'function') ? v.toNumber() : Number(v);
		return (isFinite(n) && n > 0) ? n : 0;
	}

	// ── Text ──

	// A string clipped to `n` characters. THE SCRUB HAPPENS BEFORE THE CUT: a cut destroys the very
	// shape the scrubber matches on, so a key beginning at character 190 of a line would otherwise
	// reach the egress seam as a prefix too short for any rule to know and every reader still can.
	// ANSI colouring (fe2o3's `err!` wraps a wasm-side error in it) is stripped as noise.
	function clip(s, n) {
		const v = scrub.text(String(s == null ? '' : s))
			.replace(/\u001b\[[0-9;]*m/g, '')
			.replace(/\[[0-9]{1,2}(;[0-9]{1,2})*m/g, '')
			.replace(/\s+/g, ' ')
			.trim();
		return v.length > n ? v.slice(0, n) : v;
	}

	function device() { try { return String((cfg.device && cfg.device()) || ''); } catch (e) { return ''; } }
	function build()  { try { return String((cfg.build && cfg.build()) || ''); } catch (e) { return ''; } }
	function shortDevice() { return clip(device(), caps.maxDevChars); }
	function buildTag()    { return clip(build() || '', caps.maxBuildChars); }

	// JSON, or a throw. Deliberately not swallowed: a payload that will not serialise is a fault.
	function str(o) { return JSON.stringify(o); }

	/// The event's JSON, cut to `maxEventBytes`. The LARGEST string field goes first and only far
	/// enough to fit, so a short tool name survives a long error message being trimmed; `tr:1` marks
	/// that something was cut. The envelope is never sacrificed: an event with no payload left still
	/// says which device, which build and where in the sequence it sits.
	function fit(obj) {
		// THE EGRESS SEAM. Nothing reaches the wire as an `ev` row except through here, so a payload
		// is scrubbed once, at full length, BEFORE the trimming loop below cuts it.
		obj = scrub.deep(obj);
		let s = str(obj);
		const max = caps.maxEventBytes;
		if (byteLen(s) <= max) return s;
		const o = {};
		Object.keys(obj).forEach(function (k) { o[k] = obj[k]; });
		o.tr = 1;
		for (let guard = 0; guard < 32; guard++) {
			s = str(o);
			const over = byteLen(s) - max;
			if (over <= 0) return s;
			let bigK = '', bigN = 0;
			Object.keys(o).forEach(function (k) {
				if (ENVELOPE[k] === 1 || k === 'tr') return;
				// Only strings are candidates, so a capability triple of booleans is never trimmed.
				if (typeof o[k] !== 'string') return;
				const n = byteLen(o[k]);
				if (n > bigN) { bigN = n; bigK = k; }
			});
			if (!bigK) break;
			// Characters, not bytes: a multi-byte string loses at least `over` bytes this way, never
			// fewer, so the loop always converges.
			const keep = o[bigK].length - over - 1;
			if (keep > 0) o[bigK] = o[bigK].slice(0, keep);
			else delete o[bigK];
		}
		s = str(o);
		if (byteLen(s) <= max) return s;
		// Nothing left but the envelope, and it is still worth sending.
		return str({ v: o.v, d: o.d, n: o.n, b: o.b, t: o.t, tr: 1 });
	}

	// ── The outbox ──

	// The next sequence number for this device, persisted BEFORE the event is queued. Monotonic
	// across reloads, so a redelivered post is harmless: the reader keys on `(device, n)`.
	function nextSeq() {
		let n = 0;
		try { n = parseInt(read(keys.seq) || '0', 10) || 0; } catch (e) { n = 0; }
		n = (n > 0 ? n : 0) + 1;
		write(keys.seq, String(n));
		return n;
	}

	// The outbox as the last session left it. A parse failure yields an empty queue rather than a
	// throw: a corrupt outbox must not stop the app booting. Only well-formed rows survive.
	function loadOutbox() {
		let rows = [];
		try { rows = JSON.parse(read(keys.outbox) || '[]') || []; } catch (e) { rows = []; }
		if (!Array.isArray(rows)) return [];
		return rows.filter(function (r) {
			return r && typeof r.tag === 'string' && typeof r.data === 'string';
		}).slice(-caps.outboxCap).map(function (r) {
			return { ts: ms(r.ts) || now(), tag: r.tag, data: r.data };
		});
	}
	outbox = loadOutbox();

	function persistNow() {
		try { if (persistTimer) clearT(persistTimer); } catch (e) { /* no timer */ }
		persistTimer = null;
		try { if (storage) storage.setItem(keys.outbox, JSON.stringify(outbox)); }
		catch (e) {
			// Quota. Half an outbox that persists beats a whole one that does not: the oldest half
			// goes, the same rule the overflow cap follows, AND IT IS COUNTED, so a reader who sees a
			// run of missing sequence numbers can tell a full device from a sink losing them.
			const cut = Math.ceil(outbox.length / 2);
			try {
				outbox.splice(0, cut);
				storage.setItem(keys.outbox, JSON.stringify(outbox));
			} catch (e2) { /* memory-only from here; the queue still drains */ }
			if (cut > 0 && !dropping) {
				dropping = true;
				try { emit('feed.drop', { count: cut, why: 'quota' }); }
				catch (e3) { /* a drop that cannot be reported is still a drop */ }
				finally { dropping = false; }
			}
		}
	}

	// A trailing debounce, so a burst of three hundred events costs one serialisation. What it
	// loses in the window would have been REDELIVERED, not dropped, since `n` makes a repeat idempotent.
	function persistSoon() {
		if (persistTimer) return;
		try { persistTimer = setT(persistNow, caps.persistMs); }
		catch (e) { persistNow(); }
	}

	// Append one finished row and kick the drainer. Overflow drops the OLDEST (a live device's
	// recent history is what a reader wants) and records the count in one `feed.drop`.
	function pushRow(tag, data) {
		outbox.push({ ts: now(), tag: tag, data: data });
		if (outbox.length > caps.outboxCap) {
			const cut = outbox.length - caps.outboxCap;
			outbox.splice(0, cut);
			dropOwed += cut;
			if (!dropping) {
				dropping = true;
				try { emit('feed.drop', { count: dropOwed }); dropOwed = 0; }
				finally { dropping = false; }
			}
		}
		persistSoon();
		if (gate()) drain();
	}

	// ── Events ──

	// Build, redact, fit and queue one event. The envelope wins over a payload key of the same
	// name, so a caller cannot overwrite `n` or `t` by accident. Throws on a payload that will not
	// serialise; only `event` calls this from outside.
	function emit(kind, payload) {
		const ev = { v: 1, d: shortDevice(), n: nextSeq(), b: buildTag(), t: now() };
		const more = cfg.extra ? cfg.extra(kind) : null;
		if (more && typeof more === 'object') {
			Object.keys(more).forEach(function (k) { if (ENVELOPE[k] !== 1 && more[k] !== undefined) ev[k] = more[k]; });
		}
		if (payload && typeof payload === 'object') {
			Object.keys(payload).forEach(function (k) {
				if (ENVELOPE[k] === 1) return;
				if (payload[k] === undefined) return;
				ev[k] = payload[k];
			});
		}
		pushRow('ev ' + kind, fit(redactor.value(ev)));
		return true;
	}

	/// THE ONE SEAM. A no-op when the gate is shut or the feed has faulted; never throws into its
	/// caller.
	function event(kind, payload) {
		if (!gate() || feedOff || !kind) return false;
		try {
			const ok = emit(kind, payload);
			if (cfg.after) cfg.after(kind, payload);
			return ok;
		} catch (e) { fault(e); return false; }
	}

	// The feed itself threw. One `feed.fault` is queued and collection stops for the session, but
	// the DRAINER is left running so the fault reaches the sink rather than sitting in a queue
	// nobody empties. Idempotent: a second fault says nothing, since the first already did.
	function fault(e) {
		if (feedOff) return;
		feedOff = true;
		try { if (cfg.onFault) cfg.onFault(e); } catch (e2) { /* the hook is not worth a second fault */ }
		try {
			const row = {
				v: 1, d: shortDevice(), n: nextSeq(), b: buildTag(), t: now(),
				msg: clip((e && e.message) || e || 'feed fault', caps.maxMsgChars),
			};
			pushRow('ev feed.fault', fit(redactor.value(row)));
		} catch (e2) { /* a feed that cannot report its own fault is simply off */ }
	}

	// ── Errors and the console ──

	// One argument of a console call as a short string. Only primitives and an Error's `message`
	// are read: the name-based redactor cannot see into a stringified object, so an object is named
	// and never unpacked.
	function argWord(a) {
		try {
			if (a == null) return String(a);
			if (typeof a === 'string') return a;
			if (typeof a === 'number' || typeof a === 'boolean') return String(a);
			if (a.message) return String(a.message);
			if (Array.isArray(a)) return '[array:' + a.length + ']';
			return '[' + ((a.constructor && a.constructor.name) || 'object') + ']';
		} catch (e) { return '[unreadable]'; }
	}

	function noteError(msg, where) {
		if (!gate() || feedOff) return;
		const t = now();
		if (t - errWindowAt >= caps.errWindowMs) { errWindowAt = t; errCount = 0; }
		if (errCount >= caps.errorsPerMin) { errDropped += 1; return; }
		errCount += 1;
		const p = { msg: clip(msg, caps.maxMsgChars) };
		if (where) p.at = clip(where, 80);
		if (errDropped) { p.dropped = errDropped; errDropped = 0; }
		event('error', p);
	}

	// `file:line` for the frame that logged, from a thrown-away stack, only for a line actually
	// being ADMITTED, so the cost is bounded by `consolePerMin`. Empty where there is no usable stack.
	function stackSrc() {
		let st = '';
		// The host's own `Error` is read at call time, so a host with no usable stack can say so.
		try { const E = host.Error || Error; st = (new E()).stack || ''; } catch (e) { return ''; }
		const lines = String(st).split('\n');
		for (let i = 0; i < lines.length && i < 12; i++) {
			if (skip.some(function (s) { return lines[i].indexOf(s) !== -1; })) continue;
			const m = /([A-Za-z0-9_.-]+\.(?:js|mjs)):(\d+)(?::\d+)?/.exec(lines[i]);
			if (m) return clip(m[1] + ':' + m[2], caps.maxSrcChars);
		}
		return '';
	}

	// Where a console line came from: the stack frame where there is one, else the bracketed
	// prefix the app puts on its own logs (`[sync]`), the next most useful thing to sort by.
	function srcOf(args) {
		const src = stackSrc();
		if (src) return src;
		try {
			const a0 = (args && args.length) ? args[0] : '';
			if (typeof a0 === 'string') {
				const m = /^\s*(\[[^\]]{1,40}\])/.exec(a0);
				if (m) return clip(m[1], caps.maxSrcChars);
			}
		} catch (e) { /* an argument that will not be read has no source */ }
		return '';
	}

	// Emit every held line whose dedupe window has closed, or all of them when `force` is set (what
	// a beat and `pagehide` do, so nothing waits in memory for a repeat that never comes). Guarded,
	// because `event` reaches `drain` and anything down there that logs would re-enter mid-walk.
	function flushConsole(t, force) {
		if (typeof t === 'boolean') { force = t; t = 0; }
		if (conFlushing) return;
		conFlushing = true;
		try {
			t = t || now();
			const ks = Object.keys(conHeld);
			for (let i = 0; i < ks.length; i++) {
				const h = conHeld[ks[i]];
				if (!force && (t - h.at) < caps.consoleDedupeMs) continue;
				delete conHeld[ks[i]];
				const p = { lvl: h.lvl, msg: h.msg };
				if (h.src) p.src = h.src;
				if (h.x > 1) p.x = h.x;		// the whole window in one row
				event('console', p);
			}
		} finally { conFlushing = false; }
	}

	function scheduleConsoleFlush() {
		if (conTimer) return;
		try {
			conTimer = setT(function () {
				conTimer = null;
				try { flushConsole(now(), true); } catch (e) { fault(e); }
			}, caps.consoleDedupeMs);
		} catch (e) { /* no timer: the app's beat and `pagehide` still flush */ }
	}

	// One console call. A repeat of a held line costs a counter; a new line costs an entry, a stack
	// sample and one of the window's slots. Over the cap the line is counted into `conDropped`.
	function noteConsole(lvl, args) {
		if (!gate() || feedOff) return;
		const t = now();
		flushConsole(t);
		let msg = '';
		try {
			const parts = [];
			for (let i = 0; i < args.length && i < 4; i++) parts.push(argWord(args[i]));
			msg = clip(parts.join(' '), caps.maxMsgChars);
		} catch (e) { return; }
		if (!msg) return;
		// `clip` has taken the VALUES out by content, which keeps the line readable. These stay
		// behind it for the cases a content rule cannot reach: a name against a value too short to
		// have a shape (`api_key: hunter2`), and whatever the caller's string test calls secret.
		if (looksSecret(msg)) msg = redactor.fingerprint(msg);
		else {
			const hit = redactor.text(msg);
			if (hit) msg = hit;
		}
		const key = lvl + '|' + msg;
		const held = conHeld[key];
		if (held) { held.x += 1; return; }
		if (t - conWindowAt >= caps.consoleWindowMs) { conWindowAt = t; conCount = 0; }
		if (conCount >= caps.consolePerMin) { conDropped += 1; return; }
		conCount += 1;
		conHeld[key] = { lvl: lvl, msg: msg, src: srcOf(args), at: t, x: 1 };
		scheduleConsoleFlush();
	}

	/// A gateway call that did not answer 2xx, or threw. Path, status and elapsed ms only, NEVER the
	/// request body. The feed's own posts are not failures: a 429 on our own endpoint is feed
	/// health, and a `fetch.fail` about it would be the feed describing itself to itself. An
	/// ABORTED request (a status 0 while the page is going away) is marked rather than dropped.
	function noteFetchFail(path, status, lapsed, err) {
		if (!gate() || feedOff) return;
		const clean = String(path || '').split('?')[0];
		if (cfg.ownPath && clean === cfg.ownPath) return;
		const p = { path: clip(clean, 64), status: status | 0, ms: lapsed | 0 };
		if (err) p.err = clip(err, 120);
		if (!p.status && unloading()) p.aborted = 1;
		event('fetch.fail', p);
	}

	// Is the page on its way out? Hidden, or it said `pagehide` within the grace window.
	function unloading() {
		try { if (doc && doc.visibilityState === 'hidden') return true; } catch (e) { /* no document */ }
		return !!(pagehideAt && (now() - pagehideAt) < caps.unloadGraceMs);
	}

	/// Wraps every console method on `c`: the original runs first and unconditionally, then the
	/// line is captured. `error` keeps its `error` event as well as its `console` one: the first is
	/// the rate-capped fault trail a reader scans first, the second the transcript of what was
	/// printed. One re-entry guard serves all five levels, since anything below that logs would
	/// otherwise call straight back in.
	function captureConsole(c) {
		c = c || host.console;
		if (!c) return;
		LEVELS.forEach(function (lvl) {
			const orig = c[lvl];
			if (typeof orig !== 'function' || orig._lens) return;
			const wrapped = function () {
				try { orig.apply(c, arguments); } catch (e) { /* the app's own logging comes first */ }
				if (inConsole) return;
				inConsole = true;
				try {
					noteConsole(lvl, arguments);
					if (lvl === 'error') {
						const parts = [];
						for (let i = 0; i < arguments.length && i < 4; i++) parts.push(argWord(arguments[i]));
						noteError(parts.join(' '), 'console.error');
					}
				} catch (e) { /* never from the app's own logging */ }
				inConsole = false;
			};
			wrapped._lens = true;
			c[lvl] = wrapped;
		});
	}

	/// Hooks an uncaught error and an unhandled rejection on `win`, both inert while the gate is shut.
	function captureErrors(win) {
		try {
			win.addEventListener('error', function (e) {
				try {
					let where = '';
					if (e && e.filename) where = String(e.filename).replace(/^https?:\/\/[^/]+/, '') + ':' + (e.lineno || 0);
					noteError((e && e.message) || 'error', where);
				} catch (e2) { fault(e2); }
			});
		} catch (e) { /* no window */ }
		try {
			win.addEventListener('unhandledrejection', function (e) {
				try {
					const r = e && e.reason;
					noteError((r && r.message) || argWord(r) || 'rejection', 'unhandledrejection');
				} catch (e2) { fault(e2); }
			});
		} catch (e) { /* no window */ }
	}

	/// The outbox is written on a trailing debounce, so the page going away is the one moment it
	/// must be written NOW. It is also the moment a request in flight is about to fail for no
	/// reason, so the time is remembered for `noteFetchFail`. `d` is the document, for visibility.
	function captureLifecycle(win, d) {
		if (d !== undefined) doc = d;
		try {
			win.addEventListener('pagehide', function () {
				pagehideAt = now();
				try { flushConsole(now(), true); } catch (e) { /* best effort */ }
				try { persistNow(); } catch (e) { /* best effort */ }
			});
			win.addEventListener('visibilitychange', function () {
				try {
					if (doc && doc.visibilityState !== 'hidden') return;
					try { flushConsole(now(), true); } catch (e) { /* best effort */ }
					persistNow();
				} catch (e) { /* best effort */ }
			});
		} catch (e) { /* no window */ }
	}

	// ── The post clock and the drain ──

	function gapMs() {
		const wait = caps.gapMs * Math.pow(2, throttleStreak);
		return wait > caps.backoffMaxMs ? caps.backoffMaxMs : wait;
	}

	function backoffMs(n) {
		if (n != null) failStreak = n;
		const k = failStreak > 0 ? failStreak : 1;
		const wait = gapMs() * Math.pow(2, k - 1);
		return wait > caps.backoffMaxMs ? caps.backoffMaxMs : wait;
	}

	function gapLeft() {
		if (!lastPostAt) return 0;
		const left = (lastPostAt + gapMs()) - now();
		return left > 0 ? left : 0;
	}

	// One post's worth of events from the FRONT of the outbox, never splitting one, bounded by both
	// the row cap and the body cap.
	function eventBatch() {
		const out = [];
		let bytes = 0;
		for (let i = 0; i < outbox.length && out.length < caps.maxRowsPost; i++) {
			const r = outbox[i];
			const rowBytes = r.tag.length + r.data.length + 48;		// JSON overhead per row
			if (out.length && (bytes + rowBytes) > caps.maxBodyBytes) break;
			out.push({ ts: r.ts, tag: r.tag, data: r.data });
			bytes += rowBytes;
		}
		return out;
	}

	/// The body a sink such as `fe2o3_net::lens::Sink::post` takes: `{ v: 1, device, rows }`.
	function postBody(rows) {
		return JSON.stringify({ v: 1, device: device(), rows: rows });
	}

	// What one post's answer means for the clock: a 429 doubles the gap and is announced once per
	// burst, any other failure is counted, and a post that landed clears both. The counters do not
	// overlap, so a beat's `throttled` and `postFail` read as the two separate things they are.
	function settle(ok, status) {
		if (ok) {
			throttleStreak = 0;
			throttleBurst  = false;
		} else if (status === 429) {
			throttledCount += 1;
			throttleStreak += 1;
			if (!throttleBurst) {
				throttleBurst = true;
				try { event('feed.throttled', { n: throttledCount, gap: gapMs() }); }
				catch (e) { /* the announcement is not worth a fault */ }
			}
		} else {
			postFailCount += 1;
		}
		return { ok: ok, status: status };
	}

	/// Makes one post through the caller's `send`. The promise resolves to `{ ok, status }`: whether
	/// the post LANDED, which a second lane ignores and the event lane depends on. A thrown send and
	/// a non-2xx answer are the same thing here, and both mean "still ours".
	function post(rows) {
		// Stamped BEFORE the request, not after it: the sink's window starts when the post arrives,
		// and a slow reply must not buy a second one.
		lastPostAt = now();
		write(keys.postAt, String(lastPostAt));
		let p;
		try { p = Promise.resolve(cfg.send(rows, { device: device() })); }
		catch (e) { return Promise.resolve(settle(false, 0)); }
		return p.then(function (r) {
			// A send that resolves nothing counts as delivered: the alternative is an outbox that
			// never empties.
			if (!r) return settle(true, 0);
			const status = r.status | 0;
			if (typeof r.ok === 'boolean') return settle(r.ok, status);
			return settle(!(status >= 400), status);
		}, function () { return settle(false, 0); });
	}

	function pending() {
		if (!gate()) return false;
		return outbox.length > 0 || !!(cfg.lane && cfg.lane.pending());
	}

	/// Drains the outbox one batch every `gapMs`, then the caller's lane, so the sink's rate floor is
	/// never met. Stops the moment the gate shuts. A drain entered any way but by its own pacing
	/// timer first asks the post clock what is owed: the old drainer paced itself perfectly while it
	/// ran and asked nothing when it started again, which is how a boot restore drew a 429.
	function drain() {
		if (draining) return;
		draining = true;
		step(false);

		function step(paced) {
			if (!pending()) { draining = false; return; }
			if (!paced) {
				const owed = gapLeft();
				if (owed > 0) { setT(function () { step(true); }, owed); return; }
			}
			// EVENTS FIRST, and they are the only DURABLE lane: the rows stay in the outbox until the
			// post is known to have landed, so an outage costs a retry and not the events that explain
			// it. The batch is spliced off by LENGTH, never by identity: nothing else removes from the
			// front.
			if (outbox.length) {
				const batch = eventBatch();
				post(batch).then(function (res) {
					let wait = gapMs();
					if (res.ok) {
						outbox.splice(0, batch.length);
						failStreak = 0;
						persistNow();
					} else {
						failStreak += 1;
						wait = backoffMs();
					}
					if (!pending()) { draining = false; return; }
					setT(function () { step(true); }, wait);
				});
				return;
			}
			const rows = cfg.lane ? cfg.lane.next() : null;
			if (!rows) { draining = false; return; }
			post(rows).then(function () {
				if (!pending()) { draining = false; return; }
				setT(function () { step(true); }, gapMs());
			});
		}
	}

	/// The gate has shut: nothing more may leave this device, so the outbox and everything held for
	/// the console are dropped, and the failure counters start again. The post clock is kept.
	function reset() {
		outbox = [];
		failStreak = 0;
		dropOwed = 0;
		conHeld = {};
		conDropped = 0;
		try { if (conTimer) clearT(conTimer); } catch (e) { /* no timer */ }
		conTimer = null;
		persistNow();
	}

	return {
		event:          event,
		fault:          fault,
		pushRow:        pushRow,
		post:           post,
		postBody:       postBody,
		drain:          drain,
		reset:          reset,
		persistNow:     persistNow,
		noteError:      noteError,
		noteConsole:    function (lvl, args) { noteConsole(lvl, args || []); },
		flushConsole:   flushConsole,
		noteFetchFail:  noteFetchFail,
		captureConsole: captureConsole,
		captureErrors:  captureErrors,
		captureLifecycle: captureLifecycle,
		clip:           clip,
		fit:            fit,
		eventBatch:     eventBatch,
		outbox:         function () { return outbox.slice(); },
		depth:          function () { return outbox.length; },
		ok:             function () { return !feedOff; },
		device:         device,
		gapMs:          gapMs,
		gapLeft:        gapLeft,
		backoffMs:      backoffMs,
		takeConsoleDropped: function () { const n = conDropped; conDropped = 0; return n; },
		health:         function () {
			return { throttled: throttledCount, postFail: postFailCount, cdrop: conDropped, gap: gapMs(), lastPostAt: lastPostAt };
		},
	};
}
