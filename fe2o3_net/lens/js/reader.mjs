// reader.mjs -- the node half of the debug lens: pull a mirror, keep an archive, answer questions.
//
// Extracted from Daimond's `dev/lens.mjs` (2026-10-02) so that Daimond and Oxegen share one reader.
// What the app alone knows is a parameter of `createReader`: where the home directory is, how the
// mirror is fetched, which names and strings are secret, what a snapshot or a telemetry bundle
// holds. The views (`turns`, `errors`, `status` and the rest) stay in the app; they read the archive
// this file keeps and use the helpers it exports.
//
// THE LINE. `fe2o3_net::lens::Sink` writes one object a line, `{"bt":<recv ms>,"device":"..",
// "ts":<client ms>,"tag":"..","data":".."}`, with `data` a string (an event's JSON as text, so a
// clip never makes an invalid line). Daimond's gateway still writes the text block the line was
// taken from (`===== <recv ms> account=<a> device=<d> rows=<n> =====` and `  <ts>  <tag>  <data>`
// under it); this reader takes either, and both reach the same archive records.
//
// THE FILES. A key's live file is `<key>.ndjson`. A rotation closes it as `<key>.ndjson.<stamp>`,
// `<stamp>` being `YYYYMMDDTHHMMSSZ` and `-n` added on a taken stamp. A rotated file is closed for
// good, so once it has been read it is skipped until its size changes.
//
// THE ARCHIVE, kept under `<home>/archive/`:
//
//   <device>.events.ndjson      every `ev` row as {src:'ev',device,bt,ts,kind,ev}, every other row as
//                               {src:'row',device,bt,ts,tag,data}
//   <device>.telemetry.ndjson   one record per assembled `ds telemetry` set
//   <device>.snapshots.ndjson   one index record per `ds snapshot` set, its body in `snapshots/`
//
// DEDUPE. An event is filed once by its envelope `(d, n)`, whichever file or retry delivers it. A
// chunk of a `ds` set is filed once by its index. A line is told from a line already read by its
// receive time and a hash of its text, per stream (one key's files), so a torn tail that is
// completed on the next pull loses nothing and files nothing twice.
//
// SCRUBBING AGAIN. Every record passes the caller's redactor (by name and by string test, where
// given) and then the content scrubber on its way to disk, because the writer's own guard is the
// first line and a client bug or an older client is what the second line is for.
//
// OUTPUT IS THE BUDGET. A digest is read by a language model at every pickup, so `digest` and
// `capBytes` hold it to 2 KB inside the function, and say what they withheld.
import { execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { createScrubber } from './redact.js';

// ── Arguments ────────────────────────────────────────────────────────

export function parseArgs(argv) {
	const a = { cmd: argv[0] || 'digest', flags: {}, bool: {} };
	for (let i = 1; i < argv.length; i++) {
		const t = argv[i];
		if (!t.startsWith('--')) continue;
		const k = t.slice(2);
		const next = argv[i + 1];
		if (next !== undefined && !next.startsWith('--')) { a.flags[k] = next; i++; }
		else a.bool[k] = true;
	}
	return a;
}

/// Turn `--since` into an absolute millisecond bound. Bare numbers are seconds,
/// so `--since 90` and `--since 90s` agree.
export function sinceMs(v, dflt) {
	if (v === undefined || v === null || v === '') return dflt;
	const s = String(v).trim();
	if (s === 'all') return 0;
	if (s === 'today') { const d = new Date(); d.setHours(0, 0, 0, 0); return d.getTime(); }
	const m = s.match(/^(\d+(?:\.\d+)?)\s*([smhdw]?)$/i);
	if (m) {
		const mult = { '': 1000, s: 1000, m: 60000, h: 3600000, d: 86400000, w: 604800000 };
		return Date.now() - Number(m[1]) * mult[m[2].toLowerCase()];
	}
	const t = Date.parse(s);
	return Number.isFinite(t) ? t : dflt;
}

// ── Small file helpers ───────────────────────────────────────────────

export function ensureDir(d) { fs.mkdirSync(d, { recursive: true }); }
export function safeName(s) { return String(s || 'unknown').replace(/[^A-Za-z0-9_-]/g, '_').slice(0, 64); }

export function readJson(f, dflt) {
	try { return JSON.parse(fs.readFileSync(f, 'utf8')); } catch (e) { return dflt; }
}

export function writeJson(f, obj) {
	ensureDir(path.dirname(f));
	// Through a temporary file, so a pull killed mid-write (the timer's unit
	// stopping, an OOM) cannot leave a truncated state that loses the archive's
	// whole ingestion history.
	const tmp = f + '.tmp';
	fs.writeFileSync(tmp, JSON.stringify(obj));
	fs.renameSync(tmp, f);
}

export function readNdjson(f) {
	let txt = '';
	try { txt = fs.readFileSync(f, 'utf8'); } catch (e) { return []; }
	const out = [];
	for (const line of txt.split('\n')) {
		if (!line) continue;
		// A line half-written by a killed pull is skipped rather than fatal: the
		// archive is append-only, so the loss is one record at the tail.
		try { out.push(JSON.parse(line)); } catch (e) { /* torn tail */ }
	}
	return out;
}

// ── Integer range sets ───────────────────────────────────────────────
//
// Chunk indices and event sequence numbers both arrive almost in order with
// occasional redelivery, so a list of [lo,hi] ranges collapses to one or two
// entries and keeps state.json small where a plain set of every number seen
// would reach megabytes within a day.

export function rangeHas(rs, n) {
	for (const r of rs) if (n >= r[0] && n <= r[1]) return true;
	return false;
}

export function rangeAdd(rs, n) {
	if (rangeHas(rs, n)) return false;
	rs.push([n, n]);
	rs.sort((a, b) => a[0] - b[0]);
	for (let i = rs.length - 1; i > 0; i--) {
		if (rs[i][0] <= rs[i - 1][1] + 1) {
			rs[i - 1][1] = Math.max(rs[i - 1][1], rs[i][1]);
			rs.splice(i, 1);
		}
	}
	return true;
}

export function rangeCount(rs) {
	let n = 0;
	for (const r of rs) n += r[1] - r[0] + 1;
	return n;
}

/// The numbers in 1..n that the ranges do not cover, as a short human string.
export function rangeMissing(rs, n, cap = 6) {
	const gaps = [];
	let next = 1;
	for (const r of rs) {
		if (r[0] > next) gaps.push([next, r[0] - 1]);
		next = Math.max(next, r[1] + 1);
	}
	if (next <= n) gaps.push([next, n]);
	const shown = gaps.slice(0, cap).map(g => g[0] === g[1] ? String(g[0]) : g[0] + '-' + g[1]);
	if (gaps.length > cap) shown.push('+' + (gaps.length - cap) + ' more');
	return shown.join(',');
}

// ── Parsing what the sink wrote ──────────────────────────────────────

const HEADER_RE	= /^===== (\d+) account=(\S+) device=(\S+) rows=(\d+) =====$/;
const CHUNK_RE	= /^ds (telemetry|snapshot) (\S+) (\d+)\/(\d+)$/;
const EV_RE	= /^ev (\S+)$/;

/// One line of a sink file as `{bt, device, ts, tag, data}`, or null for anything that is not one: a
/// torn tail, a blank, a value that is not an object, a line with no tag or no receive time, or a
/// `data` that is not a string (T5a always writes a string).
export function parseLine(text) {
	const raw = String(text).trim();
	if (!raw) return null;
	let o;
	try { o = JSON.parse(raw); } catch (e) { return null; }
	if (!o || typeof o !== 'object' || Array.isArray(o)) return null;
	if (!Number.isFinite(o.bt) || typeof o.tag !== 'string') return null;
	if (o.data !== undefined && typeof o.data !== 'string') return null;
	return {
		bt:     o.bt,
		device: typeof o.device === 'string' ? o.device : '',
		ts:     Number.isFinite(o.ts) ? o.ts : o.bt,
		tag:    o.tag,
		data:   o.data === undefined ? '' : o.data,
	};
}

/// Every line of a file that parses, each with its trimmed text as `raw`, and how many non-blank lines
/// did not. A file that is not there reads as empty.
export function readLines(file) {
	let txt = '';
	try { txt = fs.readFileSync(file, 'utf8'); } catch (e) { return { rows: [], bad: 0 }; }
	const rows = [];
	let bad = 0;
	for (const line of txt.split('\n')) {
		if (!line.trim()) continue;
		const r = parseLine(line);
		if (r) { r.raw = line.trim(); rows.push(r); } else bad++;
	}
	return { rows, bad };
}

/// Group consecutive rows of one receive time and device, which is one post, into the blocks the
/// ingest works on.
export function lineBlocks(rows) {
	const out = [];
	let cur = null;
	for (const r of rows) {
		if (!cur || cur.bt !== r.bt || cur.device !== r.device) {
			cur = { bt: r.bt, device: r.device, rows: [] };
			out.push(cur);
		}
		cur.rows.push({ ts: r.ts, tag: r.tag, data: r.data, raw: r.raw });
	}
	return out;
}

/// Which family a tag belongs to: `ev <kind>`, `ds <telemetry|snapshot> <id> i/N`, or any other row.
export function tagFamily(tag) {
	const c = CHUNK_RE.exec(tag);
	if (c) return { fam: 'ds', kind: c[1], id: c[2], i: Number(c[3]), n: Number(c[4]) };
	const e = EV_RE.exec(tag);
	if (e) return { fam: 'ev', kind: e[1] };
	return { fam: 'row' };
}

/// Walk one text-block file (Daimond's gateway format), calling `onBlock({bt, account, device,
/// rows})` in file order. Rows are `{ts, tag, data}`; a row the gateway marked malformed comes
/// through with tag `(malformed row)` rather than being dropped, since the client having sent
/// something unexpected is itself a finding. Tag and data are separated by the FIRST double space,
/// because a tag may hold single spaces and data may hold double ones.
export function eachBlock(file, onBlock) {
	let txt = '';
	try { txt = fs.readFileSync(file, 'utf8'); } catch (e) { return; }
	let cur = null;
	for (const line of txt.split('\n')) {
		const h = HEADER_RE.exec(line);
		if (h) {
			if (cur) onBlock(cur);
			cur = { bt: Number(h[1]), account: h[2], device: h[3], claimed: Number(h[4]), rows: [] };
			continue;
		}
		if (!cur || !line.startsWith('  ')) continue;
		const rest = line.slice(2);
		const m = /^(\d+)  /.exec(rest);
		if (!m) { cur.rows.push({ ts: cur.bt, tag: rest.trim(), data: '' }); continue; }
		const after = rest.slice(m[0].length);
		const cut = after.indexOf('  ');
		cur.rows.push({
			ts:   Number(m[1]),
			tag:  cut < 0 ? after : after.slice(0, cut),
			data: cut < 0 ? ''    : after.slice(cut + 2),
		});
	}
	if (cur) onBlock(cur);
}

const ROT_RE = /^(.+)\.ndjson\.(\d{8}T\d{6}Z)(?:-(\d+))?$/;

/// The lens files under a mirror directory, oldest content first: every rotated file before every
/// live one, so a stream's receive times only rise, and within a stream by stamp. Directories are
/// walked (a sink keeps `client/` and `peer/` apart). `fmt` is `lines` for `<key>.ndjson[.<stamp>]`
/// and `text` for Daimond's `<name>.log` and `<name>.log.1`; `sealed` marks a stamped file, which
/// is closed. A pre-stamp `<key>.ndjson.1` is kept and read first, as the oldest.
export function mirrorFiles(dir) {
	const out = [];
	const walk = (d, rel, depth) => {
		let ents = [];
		try { ents = fs.readdirSync(d, { withFileTypes: true }); } catch (e) { return; }
		for (const e of ents) {
			const p = path.join(d, e.name), r = rel ? rel + '/' + e.name : e.name;
			if (e.isDirectory()) { if (depth < 4) walk(p, r, depth + 1); continue; }
			let m, f = null;
			const base = (n) => (rel ? rel + '/' : '') + n;
			if ((m = ROT_RE.exec(e.name)))                  f = { fmt: 'lines', rank: 0, stem: base(m[1]), stamp: m[2], n: Number(m[3] || 0), sealed: true };
			else if ((m = /^(.+)\.ndjson\.1$/.exec(e.name))) f = { fmt: 'lines', rank: 0, stem: base(m[1]), stamp: '', n: 0, sealed: false };
			else if ((m = /^(.+)\.ndjson$/.exec(e.name)))    f = { fmt: 'lines', rank: 1, stem: base(m[1]), stamp: '', n: 0, sealed: false };
			else if ((m = /^(.+)\.log\.1$/.exec(e.name)))    f = { fmt: 'text', rank: 0, stem: base(m[1]), stamp: '', n: 0, sealed: false };
			else if ((m = /^(.+)\.log$/.exec(e.name)))       f = { fmt: 'text', rank: 1, stem: base(m[1]), stamp: '', n: 0, sealed: false };
			if (!f) continue;
			let size = 0;
			try { size = fs.statSync(p).size; } catch (er) { continue; }
			out.push(Object.assign(f, { file: p, rel: r, size }));
		}
	};
	walk(dir, '', 0);
	out.sort((a, b) => a.rank - b.rank || (a.stem < b.stem ? -1 : a.stem > b.stem ? 1 : 0)
		|| (a.stamp < b.stamp ? -1 : a.stamp > b.stamp ? 1 : 0) || a.n - b.n || (a.rel < b.rel ? -1 : 1));
	return out;
}

// ── Formatting ───────────────────────────────────────────────────────

export function num(n) {
	n = Number(n) || 0;
	if (n >= 1e9) return (n / 1e9).toFixed(2) + 'G';
	if (n >= 1e6) return (n / 1e6).toFixed(2) + 'M';
	if (n >= 1e3) return Math.round(n / 1e3) + 'k';
	return String(Math.round(n));
}

export function hm(ts) {
	if (!ts) return '--:--:--';
	const d = new Date(ts);
	const p = (n) => String(n).padStart(2, '0');
	return p(d.getHours()) + ':' + p(d.getMinutes()) + ':' + p(d.getSeconds());
}

/// A `--since` bound as a label. Time alone for a window inside the day, date
/// and time beyond it, so `--since 7d` does not report itself as "00:11:56".
export function sinceLabel(ts) {
	if (!ts) return 'the start of the archive';
	if (Date.now() - ts < 20 * 3600000) return hm(ts);
	return new Date(ts).toISOString().slice(0, 16).replace('T', ' ') + 'Z';
}

/// A wall-clock label in UTC, for anything a reader might compare against a
/// gateway log or another machine.
export function utcLabel(ts) {
	if (!ts) return '?';
	return new Date(ts).toISOString().slice(0, 16).replace('T', ' ') + 'Z';
}

/// A coverage stamp, in UTC: the clock alone inside the day, the date in front
/// of it beyond that, because "14:41Z" three days ago is a trap.
export function covLabel(ts) {
	if (!ts) return '?';
	const d = new Date(ts);
	const p = (n) => String(n).padStart(2, '0');
	const hhmm = p(d.getUTCHours()) + ':' + p(d.getUTCMinutes()) + 'Z';
	return (Date.now() - ts < 20 * 3600000) ? hhmm : d.toISOString().slice(0, 10) + ' ' + hhmm;
}

/// How a window should describe itself once coverage is taken into account.
export function windowLabel(since, cov) {
	if (cov && since < cov) return utcLabel(cov) + ' (archive start)';
	return sinceLabel(since);
}

/// How long before `at` (now, unless given) a time was.
export function ago(ts, at) {
	if (!ts) return 'never';
	const s = Math.max(0, Math.round(((at == null ? Date.now() : at) - ts) / 1000));
	if (s < 90)	return s + 's';
	if (s < 5400)	return Math.round(s / 60) + 'm';
	if (s < 172800)	return Math.round(s / 3600) + 'h';
	return Math.round(s / 86400) + 'd';
}

export function dev7(d) { return String(d || '?').slice(0, 7); }

/// Match two device identifiers loosely. A command-line `--device`, this
/// archive's directory name, and the full id a snapshot's roster or event
/// carries are three different lengths for the SAME device, so a plain
/// `startsWith` has to run both directions.
export function idsMatch(a, b) {
	a = String(a); b = String(b);
	return a.startsWith(b.slice(0, 16)) || b.startsWith(a.slice(0, 16));
}

// ── Output size is the budget ────────────────────────────────────────

/// Print at most `cap` lines, saying how many were withheld rather than going
/// quiet -- a truncated answer that does not admit it is a wrong answer.
export function emit(lines, cap) {
	if (!cap || lines.length <= cap) { console.log(lines.join('\n')); return; }
	const keep = lines.slice(0, cap - 1);
	keep.push(`… ${lines.length - cap + 1} more line(s); raise --limit or use --json`);
	console.log(keep.join('\n'));
}

// The longest prefix of `s` within `n` bytes of UTF-8, never splitting a character.
function clipBytes(s, n) {
	if (n <= 0) return '';
	if (Buffer.byteLength(s) <= n) return s;
	let end = Math.min(s.length, n);
	while (end > 0 && Buffer.byteLength(s.slice(0, end)) > n) end--;
	const c = s.charCodeAt(end - 1);
	if (end > 0 && c >= 0xD800 && c <= 0xDBFF) end--;
	return s.slice(0, end);
}

/// Join lines into text of at most `max` bytes. The first line stays, and so do the last two;
/// the lines between go from the back of the middle first, and a notice after the first line
/// says how many went. A single line too long for the cap is clipped on a character boundary,
/// so the result is never over `max`, whatever it is given.
export function capBytes(lines, max) {
	const cap = Math.max(0, Math.floor(Number(max) || 0));
	const keep = lines.map(String);
	const bytes = keep.map((l) => Buffer.byteLength(l));
	const total = () => bytes.reduce((a, b) => a + b, 0) + Math.max(0, keep.length - 1);
	if (total() <= cap) return keep.join('\n');
	let dropped = 0, size = total();
	const notice = () => '… ' + dropped + ' line(s) withheld by the ' + cap + '-byte cap';
	while (keep.length > 2 && size + (dropped ? Buffer.byteLength(notice()) + 1 : 0) > cap) {
		const i = Math.max(1, keep.length - 3);
		size -= bytes[i] + 1;
		keep.splice(i, 1); bytes.splice(i, 1);
		dropped++;
	}
	if (dropped) keep.splice(Math.min(1, keep.length), 0, notice());
	return clipBytes(keep.join('\n'), cap);
}

// ── Questions over archive records ───────────────────────────────────

const label = (r) => r.src === 'ev' ? r.kind : r.tag;

function bodyOf(r) {
	if (r.src === 'ev') return r.ev;
	const d = r.data;
	if (typeof d !== 'string' || d.charAt(0) !== '{') return null;
	try { return JSON.parse(d); } catch (e) { return null; }
}

/// Keep the records that fall in a window, name a device, carry a kind (an `ev` kind or a row's tag)
/// and hold a pattern (a string or a RegExp, tried over the label and the body). `wantDevice`, when
/// given, replaces the id-prefix test for the device.
export function filterRows(rows, o) {
	o = o || {};
	const re = o.grep == null ? null : (o.grep instanceof RegExp ? o.grep : new RegExp(String(o.grep), 'i'));
	const dev = o.device == null || o.device === '' ? null
		: (o.wantDevice ? (d) => o.wantDevice(d, o.device)
			: (d) => String(d).startsWith(String(o.device)) || String(o.device).startsWith(String(d)));
	return rows.filter((r) => {
		if (o.since && (r.ts || r.bt || 0) < o.since) return false;
		if (dev && !dev(r.device)) return false;
		if (o.kind && label(r) !== o.kind) return false;
		if (re) {
			const body = r.src === 'ev' ? JSON.stringify(r.ev) : (r.data || '');
			re.lastIndex = 0;
			if (!re.test(label(r) + ' ' + body)) return false;
		}
		return true;
	});
}

/// Archive records as timeline entries, `{bt, ts, device, src, label, run, conn, sid, rec}`, ordered
/// by the time the sink received them and then by the client's own clock. The ids come from the
/// event's payload, or from the row's data when that is a JSON object; `fields` renames them.
export function timeline(records, o) {
	const f = Object.assign({ run: 'run', conn: 'conn', sid: 'sid' }, o && o.fields);
	const out = records.map((r, i) => {
		const body = bodyOf(r);
		const id = (k) => (body && typeof body === 'object' && body[f[k]] != null) ? body[f[k]] : null;
		return { bt: r.bt || 0, ts: r.ts || 0, device: r.device, src: r.src, label: label(r),
			run: id('run'), conn: id('conn'), sid: id('sid'), rec: r, i };
	});
	out.sort((a, b) => a.bt - b.bt || a.ts - b.ts || a.i - b.i);
	return out.map((e) => { delete e.i; return e; });
}

/// The entries of one page load, one connection or one process, from `timeline`'s output. A page
/// load (`sid`) takes its own rows and every row of the connections it carried; a connection
/// (`conn`, with `run` to say which process, since a connection number restarts) takes its rows and
/// the page loads that rode it; a `run` alone takes that process's rows. A connection is told from
/// another of the same number by its run, so a restarted peer's conn 7 is not this one.
export function session(entries, key) {
	key = key || {};
	const same = (a, b) => a != null && String(a) === String(b);
	const cid = (e) => (e.run == null ? '' : String(e.run)) + '|' + e.conn;
	if (key.run != null && key.sid == null && key.conn == null) return entries.filter((e) => same(e.run, key.run));
	let seeds = [];
	if (key.sid != null) seeds = entries.filter((e) => same(e.sid, key.sid));
	else if (key.conn != null) seeds = entries.filter((e) => same(e.conn, key.conn) && (key.run == null || same(e.run, key.run)));
	if (!seeds.length) return [];
	const sids = new Set(seeds.filter((e) => e.sid != null).map((e) => String(e.sid)));
	const conns = new Set(seeds.filter((e) => e.conn != null).map(cid));
	return entries.filter((e) => (e.sid != null && sids.has(String(e.sid))) || (e.conn != null && conns.has(cid(e))));
}

export const DIGEST_MAX = 2048;
// A label that reads as something going wrong.
const BAD_RE = /(?:^|[._ -])(?:error|fail|fault|drop|refus|reject|exception|throttl)/i;

/// What a pickup needs in at most 2 KB: when, how many devices, how stale each is, what the window
/// held, the commonest labels, and the worst errors. `extra` (an array, or a function from the
/// windowed records to one) adds the app's own lines after the device block. The cap is enforced here
/// and a caller may lower it with `max` and never raise it.
export function digest(records, o) {
	o = o || {};
	const now   = o.now == null ? Date.now() : o.now;
	const since = o.since || 0;
	const max   = Math.min(DIGEST_MAX, Number.isFinite(o.max) && o.max > 0 ? Math.floor(o.max) : DIGEST_MAX);
	const all   = records || [];
	let first = 0;
	for (const r of all) { const t = r.ts || r.bt || 0; if (t && (!first || t < first)) first = t; }
	const recs  = all.filter((r) => (r.ts || r.bt || 0) >= since);
	const by = new Map(), tags = new Map(), errs = new Map();
	let bad = 0;
	for (const r of recs) {
		const t = r.ts || r.bt || 0, lab = label(r);
		const d = by.get(r.device) || { n: 0, last: 0, e: 0 };
		d.n++; d.last = Math.max(d.last, t);
		tags.set(lab, (tags.get(lab) || 0) + 1);
		const body = r.src === 'ev' ? r.ev : null;
		if (BAD_RE.test(lab) || (body && body.lvl === 'error')) {
			d.e++; bad++;
			let msg = body ? (body.msg ?? body.m ?? body.err ?? '') : (r.data || '');
			if (typeof msg !== 'string') msg = JSON.stringify(msg);
			const key = lab + '|' + msg.replace(/\d+/g, 'N');
			const e = errs.get(key) || { lab, msg, n: 0, last: 0 };
			e.n++; e.last = Math.max(e.last, t);
			errs.set(key, e);
		}
		by.set(r.device, d);
	}
	const lines = [(o.title || 'lens') + ' — ' + utcLabel(now) + '  ' + by.size + ' device(s)  archive since ' + utcLabel(first)];
	for (const [dv, d] of [...by].sort((a, b) => b[1].last - a[1].last).slice(0, 4)) {
		lines.push(dev7(dv) + ' ' + ago(d.last, now) + ' ago  ' + d.n + ' row(s)' + (d.e ? '  ' + d.e + ' err' : ''));
	}
	const extra = typeof o.extra === 'function' ? o.extra(recs) : (o.extra || []);
	for (const l of extra || []) lines.push(String(l));
	lines.push('window: ' + recs.length + ' row(s), ' + bad + ' error-like');
	const top = [...tags].sort((a, b) => b[1] - a[1]).slice(0, 5);
	if (top.length) lines.push('top: ' + top.map(([k, n]) => 'x' + n + ' ' + k).join('  '));
	for (const e of [...errs.values()].sort((a, b) => b.n - a.n || b.last - a.last).slice(0, 3)) {
		lines.push('  x' + e.n + ' ' + e.lab + ': ' + e.msg.slice(0, 88));
	}
	return capBytes(lines, max);
}

// ── The reader: a home directory, a mirror, an archive ───────────────

/// A reader over `cfg.home`. All of it optional but `home`:
///
///   mirror    the mirror's directory name under home (default `mirror`);
///   pull      how the mirror is fetched: an argv array, an array of them (run in order, no shell), or
///             a function; a failure is reported in the stat and never stops the ingest;
///   scrub     a scrubber `{text, deep}` (default `createScrubber()`), run on every record filed;
///   redact    a redactor from `createRedactor`, run first by name and by string test;
///   prepare   `bundle -> bundle`, a snapshot's second-line redaction before it is filed;
///   tick      `bundle -> fields`, what a telemetry tick keeps of its bundle (default all of it);
///   index     `bundle -> fields`, facts lifted into a snapshot's index record;
///   nameFor   `device -> name`, so `--device` can name a roster entry as well as an id prefix;
///   recentMs, pendingTtlMs, now   the dedupe window, how long an unfinished set waits, the clock.
export function createReader(cfg) {
	cfg = cfg || {};
	if (!cfg.home) throw new Error('createReader needs a home directory');
	const dirs = {
		home:    cfg.home,
		mirror:  path.join(cfg.home, cfg.mirror || 'mirror'),
		archive: path.join(cfg.home, 'archive'),
		snaps:   path.join(cfg.home, 'archive', 'snapshots'),
		pending: path.join(cfg.home, 'pending'),
		state:   path.join(cfg.home, 'state.json'),
	};
	const now      = cfg.now || (() => Date.now());
	const recentMs = cfg.recentMs || 10 * 60 * 1000;
	const ttlMs    = cfg.pendingTtlMs || 6 * 60 * 60 * 1000;
	const scrub    = cfg.scrub || createScrubber();
	const redact   = cfg.redact || null;
	const prepare  = cfg.prepare || ((b) => b);
	const tick     = cfg.tick || ((b) => b);
	const index    = cfg.index || (() => ({}));
	const nameFor  = cfg.nameFor || (() => '');

	const partsFile     = (device, kind, id) => path.join(dirs.pending, safeName(device), safeName(kind) + '_' + safeName(id) + '.parts');
	const eventsFile    = (device) => path.join(dirs.archive, safeName(device) + '.events.ndjson');
	const telFile       = (device) => path.join(dirs.archive, safeName(device) + '.telemetry.ndjson');
	const snapIndexFile = (device) => path.join(dirs.archive, safeName(device) + '.snapshots.ndjson');
	const snapBodyFile  = (device, id) => path.join(dirs.snaps, safeName(device) + '-' + safeName(id) + '.json');

	/// Append one archive record. THE INGEST SEAM: every event, telemetry tick and
	/// snapshot-index row the archive holds is written here and nowhere else, so the
	/// content scrubber runs here -- once, on the way to disk.
	function appendLine(f, obj) {
		ensureDir(path.dirname(f));
		fs.appendFileSync(f, JSON.stringify(scrub.deep(obj)) + '\n');
	}

	// ── State ────────────────────────────────────────────────────────

	function loadState() {
		const s = readJson(dirs.state, null);
		if (s && s.v === 1) return s;
		return { v: 1, devices: {} };
	}

	function devState(st, device) {
		if (!st.devices[device]) {
			st.devices[device] = {
				maxBt:   0,	// newest text-block receive time ingested for this device
				minBt:   0,	// earliest receive time of any kind, which is where coverage starts
				top:     0,	// newest receive time of any kind, which ages the closed sets
				recent:  {},	// text-block times within recentMs of maxBt, exactly
				ev:      {},	// envelope device id -> seen sequence ranges
				pending: {},	// "<kind>:<id>" -> chunk set being assembled
				done:    {},	// "<kind>:<id>" -> when the set was closed
			};
		}
		return st.devices[device];
	}

	// One stream is one key's files, live and rotated: the unit whose receive times only rise.
	function streamState(st, stem) {
		if (!st.streams) st.streams = {};
		if (!st.streams[stem]) st.streams[stem] = { maxBt: 0, lines: {} };
		return st.streams[stem];
	}

	/// Has this text block already been ingested? Blocks arrive in receive order within a
	/// stream, so anything well behind the watermark is certainly seen; near it, the exact
	/// key decides, which is what makes two posts in one millisecond safe.
	function blockSeen(ds, bt) {
		if (bt < ds.maxBt - recentMs) return true;
		return !!ds.recent[String(bt)];
	}

	function blockMark(ds, bt) {
		ds.recent[String(bt)] = 1;
		if (bt > ds.maxBt) ds.maxBt = bt;
		if (bt > ds.top) ds.top = bt;
		if (!ds.minBt || bt < ds.minBt) ds.minBt = bt;
		const floor = ds.maxBt - recentMs;
		for (const k of Object.keys(ds.recent)) if (Number(k) < floor) delete ds.recent[k];
	}

	// A line is told by its receive time and a hash of its text. `k` counts the times the same text
	// has come up in this file, so a row that was genuinely posted twice is kept twice and a file
	// read again adds nothing.
	const lineHash = (raw) => crypto.createHash('sha1').update(raw).digest('hex').slice(0, 16);

	function lineSeen(ss, bt, h, k) {
		if (bt < ss.maxBt - recentMs) return true;
		const m = ss.lines[String(bt)];
		return !!m && (m[h] || 0) >= k;
	}

	function lineMark(ss, bt, h, k) {
		const m = ss.lines[String(bt)] || (ss.lines[String(bt)] = {});
		m[h] = Math.max(m[h] || 0, k);
		if (bt > ss.maxBt) ss.maxBt = bt;
		const floor = ss.maxBt - recentMs;
		for (const t of Object.keys(ss.lines)) if (Number(t) < floor) delete ss.lines[t];
	}

	/// Mark a chunk set closed, and forget closures older than a day so the marker
	/// map cannot grow for the life of the archive.
	function closeSet(ds, key, bt) {
		if (!ds.done) ds.done = {};
		ds.done[key] = bt || now();
		const floor = (ds.top || ds.maxBt || now()) - 86400000;
		for (const k of Object.keys(ds.done)) if (ds.done[k] < floor) delete ds.done[k];
	}

	// ── Ingest ───────────────────────────────────────────────────────

	/// Concatenate a completed chunk set, decode it, and file it. A set that will not decode is
	/// recorded as a broken set rather than thrown: one corrupt bundle must not stop a pull.
	function materialise(device, p, stat) {
		const pf = partsFile(device, p.kind, p.id);
		const parts = new Map();
		let txt = '';
		try { txt = fs.readFileSync(pf, 'utf8'); } catch (e) { /* nothing on disk */ }
		for (const line of txt.split('\n')) {
			if (!line) continue;
			const tab = line.indexOf('\t');
			if (tab < 0) continue;
			const i = Number(line.slice(0, tab));
			// Last writer wins, which for a duplicated chunk is the same bytes.
			if (Number.isFinite(i)) parts.set(i, line.slice(tab + 1));
		}
		const ordered = [];
		for (let i = 1; i <= p.n; i++) ordered.push(parts.get(i) || '');
		let bundle = null, error = null;
		try { bundle = JSON.parse(Buffer.from(ordered.join(''), 'base64').toString('utf8')); }
		catch (e) { error = String(e && e.message || e).slice(0, 120); }
		if (bundle && redact) bundle = redact.value(bundle);

		if (p.kind === 'telemetry') {
			if (bundle) {
				appendLine(telFile(device), Object.assign({}, tick(bundle), {
					id: p.id, device, bt: p.lastBt, ts: bundle.ts || p.lastTs,
				}));
				stat.telemetry++;
			} else {
				appendLine(telFile(device), { id: p.id, device, bt: p.lastBt, ts: p.lastTs, broken: error });
				stat.broken++;
			}
		} else {
			const rec = {
				id: p.id, device, bt: p.lastBt, ts: bundle ? (bundle.ts || p.lastTs) : p.lastTs,
				n: p.n, have: rangeCount(p.have), complete: rangeCount(p.have) === p.n,
				missing: rangeCount(p.have) === p.n ? '' : rangeMissing(p.have, p.n),
			};
			if (bundle) {
				// The app's own second line first (a camelCase name the client missed), then the
				// content scrubber over the whole body, which is the only thing filed here that
				// does not pass through `appendLine`.
				bundle = scrub.deep(prepare(bundle));
				ensureDir(dirs.snaps);
				fs.writeFileSync(snapBodyFile(device, p.id), JSON.stringify(bundle));
				rec.body = path.basename(snapBodyFile(device, p.id));
				rec.iso  = bundle.iso || null;
				// A few facts are lifted into the index so a status never has to open a
				// megabyte of snapshot to answer a one-line question.
				Object.assign(rec, index(bundle));
				stat.snapshots++;
			} else {
				rec.broken = error;
				stat.broken++;
			}
			appendLine(snapIndexFile(device), rec);
		}
		try { fs.unlinkSync(pf); } catch (e) { /* already gone */ }
		return bundle ? 'ok' : 'broken';
	}

	/// Record a chunk set that can never complete, and stop holding its parts open.
	/// The index keeps the gap so a reader can say what was lost rather than going
	/// quiet about a bundle it never saw.
	function expire(device, p, stat) {
		if (p.kind === 'telemetry') {
			appendLine(telFile(device), {
				id: p.id, device, bt: p.lastBt, ts: p.lastTs,
				gap: true, n: p.n, have: rangeCount(p.have), missing: rangeMissing(p.have, p.n),
			});
		} else {
			appendLine(snapIndexFile(device), {
				id: p.id, device, bt: p.lastBt, ts: p.lastTs,
				n: p.n, have: rangeCount(p.have), complete: false,
				missing: rangeMissing(p.have, p.n), gap: true,
			});
		}
		try { fs.renameSync(partsFile(device, p.kind, p.id), partsFile(device, p.kind, p.id) + '.stale'); }
		catch (e) { /* nothing to keep */ }
		stat.gapped++;
	}

	function ingestRow(ds, device, bt, row, stat) {
		const t = tagFamily(row.tag);
		if (t.fam === 'ds') {
			const key = t.kind + ':' + t.id;
			// A chunk of a set that is already filed. The client retries a post whose
			// reply it lost, so this arrives after the set has completed -- and without
			// this line it would open the set again and leave a phantom half-assembled
			// bundle behind that nothing ever finishes.
			if (!ds.done) ds.done = {};
			if (ds.done[key]) { stat.dupChunks++; return; }
			let p = ds.pending[key];
			if (!p) p = ds.pending[key] = { kind: t.kind, id: t.id, n: t.n, have: [], firstTs: row.ts, lastTs: row.ts, lastBt: bt };
			p.n = Math.max(p.n, t.n);
			p.lastTs = row.ts;
			p.lastBt = bt;
			// A duplicated chunk index is written once. The client can resend a post
			// the sink accepted but whose reply was lost, so this is ordinary.
			if (rangeAdd(p.have, t.i)) {
				const pf = partsFile(device, t.kind, t.id);
				ensureDir(path.dirname(pf));
				fs.appendFileSync(pf, t.i + '\t' + row.data + '\n');
				stat.chunks++;
			} else {
				stat.dupChunks++;
			}
			if (rangeCount(p.have) >= p.n) {
				materialise(device, p, stat);
				delete ds.pending[key];
				closeSet(ds, key, bt);
			}
			return;
		}

		if (t.fam === 'ev') {
			let obj = null;
			try { obj = JSON.parse(row.data); } catch (e) { obj = null; }
			if (obj && typeof obj === 'object') {
				// The envelope's own device id is the dedupe axis, not the filename:
				// `(d,n)` is what the client makes unique, and a device that has been
				// re-keyed writes to the same file under a new `d`.
				const d = String(obj.d || device);
				const n = Number(obj.n);
				const rs = ds.ev[d] || (ds.ev[d] = []);
				if (Number.isFinite(n) && !rangeAdd(rs, n)) { stat.dupEvents++; return; }
				appendLine(eventsFile(device), {
					src: 'ev', device, bt, ts: obj.t || row.ts, kind: t.kind, ev: redact ? redact.value(obj) : obj,
				});
				stat.events++;
				return;
			}
			// An `ev` row whose data will not parse is still a fact about the client.
			appendLine(eventsFile(device), {
				src: 'row', device, bt, ts: row.ts, tag: row.tag, data: guardText(row.data), malformed: true,
			});
			stat.rows++;
			return;
		}

		// Everything else -- election traces, hand-off latency, diagnostics -- is kept verbatim as a
		// generic row, so a search can reach it without anyone having taught this file what the
		// tag means.
		appendLine(eventsFile(device), { src: 'row', device, bt, ts: row.ts, tag: row.tag, data: guardText(row.data) });
		stat.rows++;
	}

	// A row's text, replaced whole when the caller's string test calls it secret.
	function guardText(s) {
		if (!redact) return s;
		const r = redact.text(s);
		return r == null ? s : r;
	}

	function ingestLines(f, st, stat) {
		const { rows, bad } = readLines(f.file);
		stat.bad += bad;
		const ss = streamState(st, f.stem);
		const counts = new Map();
		for (const blk of lineBlocks(rows)) {
			const ds = devState(st, blk.device);
			const fresh = [];
			for (const r of blk.rows) {
				const h = lineHash(r.raw), ck = blk.bt + '|' + h;
				const k = (counts.get(ck) || 0) + 1;
				counts.set(ck, k);
				if (lineSeen(ss, blk.bt, h, k)) continue;
				lineMark(ss, blk.bt, h, k);
				fresh.push(r);
			}
			if (!fresh.length) { stat.skipped++; continue; }
			stat.blocks++;
			if (!ds.minBt || blk.bt < ds.minBt) ds.minBt = blk.bt;
			if (blk.bt > ds.top) ds.top = blk.bt;
			for (const r of fresh) ingestRow(ds, blk.device, blk.bt, r, stat);
		}
	}

	// The caller's command, or commands, as one word for the stat.
	function runPull() {
		const p = cfg.pull;
		if (!p || (Array.isArray(p) && !p.length)) return 'skipped';
		const first = (e) => String(e && e.message || e).split('\n')[0].slice(0, 80);
		if (typeof p === 'function') {
			try { p(dirs); return 'ok'; } catch (e) { return 'failed: ' + first(e); }
		}
		let failed = null;
		for (const argv of (Array.isArray(p[0]) ? p : [p])) {
			// An argv, never a shell string, so a host name or a path is never parsed.
			try { execFileSync(argv[0], argv.slice(1), { stdio: ['ignore', 'pipe', 'pipe'], timeout: cfg.pullTimeoutMs || 300000 }); }
			catch (e) { failed = failed || first(e); }
		}
		return failed ? 'failed: ' + failed : 'ok';
	}

	/// Fetch the mirror by the caller's command (unless `noPull`), then file what it holds that the
	/// archive has not seen. Idempotent. A failed fetch still ingests whatever the mirror has, which
	/// is the point of keeping a local archive at all.
	function pull(o) {
		o = o || {};
		ensureDir(dirs.mirror); ensureDir(dirs.archive);
		const stat = {
			blocks: 0, skipped: 0, rows: 0, events: 0, dupEvents: 0, chunks: 0, dupChunks: 0,
			telemetry: 0, snapshots: 0, gapped: 0, broken: 0, bad: 0, sealed: 0, pull: 'skipped',
		};
		if (!o.noPull) stat.pull = runPull();
		const st = loadState();
		if (!st.sealed) st.sealed = {};
		for (const f of mirrorFiles(dirs.mirror)) {
			if (f.sealed && st.sealed[f.rel] === f.size) { stat.sealed++; continue; }
			if (f.fmt === 'text') {
				eachBlock(f.file, (b) => {
					const ds = devState(st, b.device);
					if (blockSeen(ds, b.bt)) { stat.skipped++; return; }
					blockMark(ds, b.bt);
					stat.blocks++;
					for (const row of b.rows) ingestRow(ds, b.device, b.bt, row, stat);
				});
			} else {
				ingestLines(f, st, stat);
			}
			if (f.sealed) st.sealed[f.rel] = f.size;
		}
		// Chunk sets whose missing rows rotated away at the source are closed off, so the
		// pending area cannot grow without bound and the gap is on the record.
		const at = now();
		for (const device of Object.keys(st.devices)) {
			const ds = st.devices[device];
			for (const key of Object.keys(ds.pending)) {
				const p = ds.pending[key];
				if (at - (p.lastBt || p.lastTs || 0) > ttlMs) {
					expire(device, p, stat);
					delete ds.pending[key];
					closeSet(ds, key, p.lastBt || at);
				}
			}
		}
		writeJson(dirs.state, st);
		return stat;
	}

	// ── Reading the archive ──────────────────────────────────────────

	function archiveDevices() {
		let names = [];
		try { names = fs.readdirSync(dirs.archive); } catch (e) { return []; }
		const set = new Set();
		for (const n of names) {
			const m = /^(.+)\.(events|telemetry|snapshots)\.ndjson$/.exec(n);
			if (m) set.add(m[1]);
		}
		return [...set].sort();
	}

	function wantDevice(device, filter) {
		if (!filter) return true;
		const f = String(filter);
		if (String(device).startsWith(f) || f.startsWith(String(device))) return true;
		// `--device` naming the roster instead of the id, matched case-insensitively against
		// whatever this archive has ever called that device. The id-prefix check above runs
		// first and is unaffected: this only fires for a filter that was never a prefix.
		const nm = nameFor(device);
		return !!nm && nm.toLowerCase().includes(f.toLowerCase());
	}

	/// When this archive's record begins, PER DEVICE and PER SOURCE. The two are not the same number
	/// and conflating them lies: a device's oldest row may be an election trace from three days ago
	/// while its oldest telemetry tick, the only source that can see a turn or a cost, is from this
	/// afternoon.
	///
	///   `ticks`  earliest telemetry tick, and earliest `ev` event;
	///   `rows`   earliest row of any kind, which is what a search can reach and nothing more.
	function coverage(filter) {
		const st = loadState();
		const byDevice = {};
		let ticks = 0, rows = 0;
		const lower = (a, b) => (!a || (b && b < a)) ? b : a;
		for (const d of archiveDevices()) {
			if (!wantDevice(d, filter)) continue;
			const rec = { device: d, ticks: 0, rows: 0 };
			// Each archive file is append-ordered, so its first record is its earliest; only the
			// events file has to be walked, because it mixes `ev` events with generic rows.
			const t0 = readNdjson(telFile(d))[0];
			if (t0) rec.ticks = t0.ts || t0.bt || 0;
			for (const r of readNdjson(eventsFile(d))) {
				const ts = r.ts || r.bt || 0;
				if (!ts) continue;
				rec.rows = lower(rec.rows, ts);
				if (r.src === 'ev') rec.ticks = lower(rec.ticks, ts);
			}
			const s0 = readNdjson(snapIndexFile(d))[0];
			if (s0) rec.rows = lower(rec.rows, s0.ts || s0.bt || 0);
			if (st.devices && st.devices[d] && st.devices[d].minBt) rec.rows = lower(rec.rows, st.devices[d].minBt);
			rec.rows = lower(rec.rows, rec.ticks);
			byDevice[d] = rec;
			ticks = lower(ticks, rec.ticks);
			rows  = lower(rows, rec.rows);
		}
		return { ticks: ticks || 0, rows: rows || 0, byDevice };
	}

	function loadFrom(fileOf, filter, since) {
		const out = [];
		for (const d of archiveDevices()) {
			if (!wantDevice(d, filter)) continue;
			for (const r of readNdjson(fileOf(d))) {
				if (since && (r.ts || r.bt || 0) < since) continue;
				out.push(r);
			}
		}
		out.sort((a, b) => (a.ts || a.bt || 0) - (b.ts || b.bt || 0));
		return out;
	}
	const loadTelemetry = (filter, since) => loadFrom(telFile, filter, since);
	const loadEvents    = (filter, since) => loadFrom(eventsFile, filter, since);
	const loadSnapshotIndex = (filter) => loadFrom(snapIndexFile, filter, 0);

	/// Chunk sets part-assembled and still waiting for their remaining rows. They are a live fact,
	/// not a fault: a snapshot is thousands of rows and drains over minutes.
	function pendingSets(filter) {
		const st = loadState();
		const out = [];
		for (const device of Object.keys(st.devices || {})) {
			if (!wantDevice(device, filter)) continue;
			const pend = st.devices[device].pending || {};
			for (const key of Object.keys(pend)) {
				const p = pend[key];
				out.push({ device, kind: p.kind, id: p.id, n: p.n, have: rangeCount(p.have || []), lastBt: p.lastBt });
			}
		}
		out.sort((a, b) => (b.lastBt || 0) - (a.lastBt || 0));
		return out;
	}

	/// The digest of what the archive holds for a device and a window: `digest` over its events.
	function digestOf(o) {
		o = o || {};
		const recs = loadEvents(o.device, o.since || 0);
		return digest(recs, Object.assign({ now: now() }, o));
	}

	return {
		dirs, appendLine, loadState, pull, archiveDevices, wantDevice, coverage,
		loadTelemetry, loadEvents, loadSnapshotIndex, pendingSets, digest: digestOf,
		eventsFile, telFile, snapIndexFile, snapBodyFile, partsFile,
	};
}
