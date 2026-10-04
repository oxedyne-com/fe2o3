#!/usr/bin/env node
// Ranks a V8 `.cpuprofile` (node `--cpu-prof`, or the inspector's Profiler domain, which
// cpuprof_runs.mjs drives) taken on the profiling wasm package: the top functions by self time, and the
// totals by module and by crate.
//
// Rust symbols in the wasm name section are legacy-mangled (`_ZN...17h<hash>E`) and are demangled here, so
// the output reads `oxedyne_fe2o3_austenite::flow::block::layout`. A frame's *crate* is the first path
// segment of its name; its *module* is the first two (`flow::block` within the crate), and its *area* the
// first one. Three views carry the totals:
//
//   - self time by the function's own crate and module: where the instructions were;
//   - self time attributed to the nearest `oxedyne_fe2o3_austenite` frame on the stack: which of
//     Austenite's modules asked for the work, whoever executed it (alloc, hashbrown, memcpy);
//   - inclusive time of named functions (`--incl=a,b`, substrings of the demangled name): the share of the
//     samples that had one of them on the stack, counted once per sample whatever the recursion.
//
// A sample's duration is the delta to the next sample, as the DevTools profile view takes it.
//
// Usage: cpuprof_rank.mjs FILE.cpuprofile [--top=30] [--incl=name,name,...] [--json]
import fs from 'node:fs';

const AUSTENITE = 'oxedyne_fe2o3_austenite';

const ESCAPES = {
	SP: '@', BP: '*', RF: '&', LT: '<', GT: '>', LP: '(', RP: ')', C: ',',
};

// One identifier of a legacy-mangled path: `$LT$` and the rest, `$u20$` and the like, `..` for `::`.
function unescapeIdent(id) {
	let s = id.startsWith('_$') ? id.slice(1) : id;
	s = s.replace(/\$([A-Za-z]+)\$/g, (m, name) => (name in ESCAPES ? ESCAPES[name] : m));
	s = s.replace(/\$u([0-9a-f]+)\$/g, (_, hex) => String.fromCharCode(parseInt(hex, 16)));
	return s.replace(/\.\./g, '::');
}

// `_ZN<len><ident>...E` to a path; the trailing `h<16 hex>` hash segment is dropped. Anything that is not
// a legacy symbol comes back as it was.
export function demangle(name) {
	if (!name.startsWith('_ZN')) return name;
	let i = 3;
	const parts = [];
	while (i < name.length && name[i] !== 'E') {
		let j = i;
		while (j < name.length && name[j] >= '0' && name[j] <= '9') j++;
		if (j === i) return name;
		const len = Number(name.slice(i, j));
		parts.push(name.slice(j, j + len));
		i = j + len;
	}
	if (parts.length && /^h[0-9a-f]{16}$/.test(parts[parts.length - 1])) parts.pop();
	return parts.map(unescapeIdent).join('::');
}

const PATH = /^[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*/;

// The path a name's crate and module come from: the implementing type's for `<T as Trait>::f`, else the
// function's own.
function ownerPath(dem) {
	let s = dem;
	while (s.startsWith('<')) s = s.slice(1);
	const m = s.match(PATH);
	if (m && m[0].includes('::')) return m[0];
	const as = dem.indexOf(' as ');
	if (as >= 0) {
		const t = dem.slice(as + 4).match(PATH);
		if (t) return t[0];
	}
	return m ? m[0] : null;
}

// What a frame is: its demangled name and where it counts.
function classify(cf) {
	const raw = cf.functionName || '';
	const url = cf.url || '';
	if (raw.startsWith('(')) return { name: raw, crate: raw, module: raw, area: raw, kind: 'runtime' };
	const wasm = url.startsWith('wasm://') || /^wasm-function\[/.test(raw) || raw.startsWith('_ZN');
	if (!wasm) {
		const where = url ? url.split('/').pop() : '';
		const name = `${raw || '(anonymous)'}${where ? ` [${where}]` : ''}`;
		return { name, crate: '(js)', module: '(js)', area: '(js)', kind: 'js' };
	}
	const name = demangle(raw);
	const path = ownerPath(name);
	if (!path) return { name, crate: '(wasm)', module: '(wasm)', area: '(wasm)', kind: 'wasm' };
	const seg = path.split('::');
	return {
		name,
		crate:	seg[0],
		module:	seg.length > 1 ? `${seg[0]}::${seg[1]}` : seg[0],
		area:	seg[0] === AUSTENITE && seg.length > 1 ? `${seg[0]}::${seg[1]}` : seg[0],
		kind:	'wasm',
		path:	seg,
	};
}

// The austenite module a frame belongs to, two segments deep: `flow::block`.
function austeniteModule(info) {
	if (info.crate !== AUSTENITE || !info.path) return null;
	return info.path.slice(1, 3).join('::') || '(root)';
}

function add(map, key, ms) {
	map.set(key, (map.get(key) || 0) + ms);
}

function table(map, total, top) {
	return [...map.entries()].sort((a, b) => b[1] - a[1]).slice(0, top).map(([name, ms]) => ({
		name, ms: ms, pct: total > 0 ? (100 * ms) / total : 0,
	}));
}

export function rank(profile, opts = {}) {
	const top = opts.top ?? 30;
	const incl = opts.incl ?? [];
	const nodes = new Map();
	for (const n of profile.nodes) nodes.set(n.id, n);
	const parent = new Map();
	for (const n of profile.nodes) for (const c of n.children || []) parent.set(c, n.id);
	const info = new Map();
	for (const n of profile.nodes) info.set(n.id, classify(n.callFrame));

	const samples = profile.samples;
	const deltas = profile.timeDeltas;
	const durMs = (i) => (i + 1 < deltas.length ? deltas[i + 1] : 0) / 1000;

	const self = new Map();		// function name -> ms
	const byCrate = new Map();
	const byModule = new Map();
	const byArea = new Map();
	const byAskedBy = new Map();	// nearest austenite module on the stack
	const byKind = new Map();
	const inclMs = new Map(incl.map((s) => [s, 0]));
	let total = 0;
	let idle = 0;
	for (let i = 0; i < samples.length; i++) {
		const ms = durMs(i);
		const id = samples[i];
		const inf = info.get(id);
		if (!inf) continue;
		if (inf.name === '(idle)') {
			idle += ms;
			continue;
		}
		total += ms;
		add(self, inf.name, ms);
		add(byCrate, inf.crate, ms);
		add(byModule, inf.module, ms);
		add(byArea, inf.area, ms);
		add(byKind, inf.kind, ms);
		let asked = null;
		const seen = new Set();
		for (let at = id; at !== undefined; at = parent.get(at)) {
			const f = info.get(at);
			if (!f) continue;
			if (asked === null) asked = austeniteModule(f);
			for (const s of incl) {
				if (!seen.has(s) && f.name.includes(s)) {
					seen.add(s);
					inclMs.set(s, inclMs.get(s) + ms);
				}
			}
		}
		add(byAskedBy, asked === null ? '(no austenite frame)' : `austenite::${asked}`, ms);
	}
	return {
		total_ms:	total,
		idle_ms:	idle,
		samples:	samples.length,
		span_ms:	(profile.endTime - profile.startTime) / 1000,
		top_self:	table(self, total, top),
		by_crate:	table(byCrate, total, 40),
		by_module:	table(byModule, total, 40),
		by_area:	table(byArea, total, 40),
		asked_by:	table(byAskedBy, total, 40),
		by_kind:	table(byKind, total, 10),
		inclusive:	[...inclMs.entries()].map(([name, ms]) => ({ name, ms, pct: total > 0 ? (100 * ms) / total : 0 })),
	};
}

function fmtRows(rows, width = 9) {
	return rows.map((r) => `${r.ms.toFixed(1).padStart(width)} ms ${r.pct.toFixed(1).padStart(5)}%  ${r.name}`).join('\n');
}

export function render(r) {
	const out = [];
	out.push(`samples ${r.samples}, busy ${r.total_ms.toFixed(1)} ms (idle ${r.idle_ms.toFixed(1)} ms, span ${r.span_ms.toFixed(1)} ms)`);
	out.push('\nTop functions by self time');
	out.push(fmtRows(r.top_self));
	out.push('\nSelf time by crate');
	out.push(fmtRows(r.by_crate));
	out.push('\nSelf time by module (crate::module)');
	out.push(fmtRows(r.by_module));
	out.push('\nSelf time attributed to the nearest austenite frame on the stack (module, two deep)');
	out.push(fmtRows(r.asked_by));
	out.push('\nSelf time by area (austenite::area, other crates whole)');
	out.push(fmtRows(r.by_area));
	out.push('\nSelf time by kind of frame');
	out.push(fmtRows(r.by_kind));
	if (r.inclusive.length) {
		out.push('\nInclusive time of named functions');
		out.push(fmtRows(r.inclusive));
	}
	return out.join('\n') + '\n';
}

function main() {
	const args = { top: 30, incl: [], json: false };
	let file = null;
	for (const a of process.argv.slice(2)) {
		let m;
		if ((m = a.match(/^--top=(\d+)$/))) args.top = Number(m[1]);
		else if ((m = a.match(/^--incl=(.*)$/))) args.incl = m[1].split(',').filter(Boolean);
		else if (a === '--json') args.json = true;
		else if (!a.startsWith('--')) file = a;
	}
	if (!file) {
		process.stderr.write('usage: cpuprof_rank.mjs FILE.cpuprofile [--top=30] [--incl=name,name] [--json]\n');
		process.exit(2);
	}
	const r = rank(JSON.parse(fs.readFileSync(file, 'utf8')), args);
	process.stdout.write(args.json ? JSON.stringify(r) + '\n' : render(r));
}

import { fileURLToPath } from 'node:url';
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main();
