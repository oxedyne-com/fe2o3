#!/usr/bin/env node
// U1 RATIO: how much of a document's page-by-page carry state a small edit leaves unchanged.
//
// The native binary compiles the document with `--eval --timings`, whose `pages` array holds, for each page
// of each fixpoint pass, a fingerprint of what the page was made from: the pairs it took from the flow and the
// flow's carry state as it entered the page (src/timings.rs `PageRec`, src/flow/carry.rs). This driver compiles
// a document, edits it, compiles it again and compares the last pass's records page by page. A page whose
// record is equal before and after is one a page ledger could replay rather than lay out again.
//
// Edit sites are the EDITTOK markers of gen_synthetic.py, or, for a document without them, the first word of
// each plain body paragraph. The sites used are those at 0, 1/N, 2/N ... of the list (N = --positions), and
// each gets two one-letter edits (a letter typed onto the end of the site's word, from the unedited text).
// Two further edits, at the third site, insert a clause of about two lines (`clause`) and one of about sixteen
// (`clause+`), so the paragraph gains lines and the second moves every later page break.
//
// Two equalities are reported for every edit. The raw one compares the pairs, the input fingerprint and every
// entry field. The masked one leaves out the absolute counters (`count`, `pulled`, `base`, `work_idx`, `abs`),
// which shift for every later page when an edit adds a line. A page is "edited" when its raw record is the first
// to differ. The resync distance is the number of pages from the edited page to the start of the run of equal
// pages that lasts to the end of the document; `null` when the last page differs. The keyed figure is the share of
// the edited run's pages whose record, counters left out, occurs on any page of the unedited run: what a ledger
// keyed by content, not by page number, could reuse.
//
// Usage: replay_ratio.mjs <austenite-binary> <doc.typ> [--work=DIR] [--json=FILE] [--positions=N]
// Prints the per-edit table and a summary to stdout; --json writes everything. Run it under
// `systemd-run --user --scope -p MemoryMax=3G`; each compile is a child of this process.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { median } from './lib/stats.mjs';

const ORDER = ['count', 'paginator', 'level', 'pulled', 'base', 'pull_rest', 'children', 'config', 'work_idx',
	'spill', 'floats', 'footnotes', 'foot_spill', 'tags', 'skips', 'abs', 'locator'];
const COUNTERS = new Set(['count', 'pulled', 'base', 'work_idx', 'abs']);
const CLAUSE = ' and, to take the matter a little further than a first reading allows, the same may be said of '
	+ 'the other things that were set beside it, which is why the whole of it was kept in view as long as it was';
const CLAUSE_LONG = CLAUSE.repeat(8);	// about sixteen lines, enough to move every later page break

function parseArgs(argv) {
	const out = { positions: 5, work: null, json: null, rest: [] };
	for (const a of argv.slice(2)) {
		const m = a.match(/^--([^=]+)=(.*)$/);
		if (!m) { out.rest.push(a); continue; }
		out[m[1]] = /^\d+$/.test(m[2]) ? Number(m[2]) : m[2];
	}
	return out;
}

// Where a letter may be typed: the end of an EDITTOK marker (a space follows each, so EDITTOK1 never
// matches the start of EDITTOK10), else the end of the first word of a plain body paragraph.
function sitesOf(text) {
	const marks = [...text.matchAll(/EDITTOK(\d+) /g)];
	if (marks.length) return marks.map((m) => ({ at: m.index + m[0].length - 1, name: m[0].slice(0, -1) }));
	return [...text.matchAll(/(?:^|\n\n)([A-Za-z]{4,})(?= )/g)].map((m) => ({ at: m.index + m[0].length, name: m[1] }));
}

function compile(bin, text, work, tag) {
	const src = path.join(work, `${tag}.typ`);
	const json = path.join(work, `${tag}.json`);
	const out = path.join(work, `out_${tag}`);
	fs.writeFileSync(src, text);
	const t0 = process.hrtime.bigint();
	const r = spawnSync(bin, ['--eval', '--timings', json, src, out], { encoding: 'utf8', maxBuffer: 1 << 26, timeout: 900000 });
	const wall_s = Number(process.hrtime.bigint() - t0) / 1e9;
	fs.rmSync(out, { recursive: true, force: true });
	if (r.status !== 0 || !fs.existsSync(json)) {
		throw new Error(`compile ${tag} failed: status ${r.status} ${r.signal || ''} ${(r.stderr || '').slice(0, 400)}`);
	}
	const d = JSON.parse(fs.readFileSync(json, 'utf8'));
	fs.rmSync(json, { force: true });
	fs.rmSync(src, { force: true });
	const pages = d.pages[d.pages.length - 1];
	const probe = d.pass[d.pass.length - 1].probe;
	return { pages, probe_ns: probe.ns, probe_n: probe.n, probe_ns_page: probe.ns / pages.length, wall_s, total_ns: d.total };
}

// Are two page records equal, and if not, in which field first? `input` covers the pairs and their count.
function same(a, b, masked) {
	if (!a || !b) return { eq: false, field: 'missing' };
	if (a.pairs !== b.pairs || a.input !== b.input) return { eq: false, field: 'input' };
	for (const f of ORDER) {
		if (masked && COUNTERS.has(f)) continue;
		if (a.entry[f] !== b.entry[f]) return { eq: false, field: f };
	}
	return { eq: true, field: null };
}

// A record without its absolute counters, as a key: the same content at another page number has the same key.
function key(r) {
	return `${r.pairs}:${r.input}:${ORDER.filter((f) => !COUNTERS.has(f)).map((f) => r.entry[f]).join(':')}`;
}

// The start of the run of equal pages that lasts to the end, minus the edited page; null if the last differs.
function resync(eq, e) {
	const n = eq.length;
	if (!n || !eq[n - 1]) return null;
	let r = n - 1;
	while (r > 0 && eq[r - 1]) r--;
	return Math.max(r, e + 1) - e;
}

function analyse(base, ed) {
	const n = Math.max(base.length, ed.length);
	const raw = [], msk = [];
	for (let i = 0; i < n; i++) {
		raw.push(same(base[i], ed[i], false));
		msk.push(same(base[i], ed[i], true));
	}
	const reqs = raw.map((x) => x.eq), meqs = msk.map((x) => x.eq);
	let e = reqs.indexOf(false);
	const count = (a, from = 0) => a.slice(from).filter(Boolean).length;
	const res = { pages: n, pages_base: base.length, pages_edit: ed.length, edited: e < 0 ? null : e };
	res.raw_equal = count(reqs) / n;
	res.masked_equal = count(meqs) / n;
	if (e < 0) e = n; // nothing differs
	// Content-keyed reuse: edited-run pages whose record (counters left out) occurs anywhere in the base run.
	const keys = new Set(base.map(key));
	res.keyed_equal = ed.filter((r) => keys.has(key(r))).length / n;
	res.raw_after = n > e + 1 ? count(reqs, e + 1) / (n - e - 1) : null;
	res.masked_after = n > e + 1 ? count(meqs, e + 1) / (n - e - 1) : null;
	res.resync_raw = resync(reqs, e);
	res.resync_masked = resync(meqs, e);
	// Pages after the edited one that differ under the masked equality: those whose pairs changed, and
	// those whose pairs are equal while the carry state is not (the failures a ledger would care about).
	const input = {}, entry = {};
	let first_entry = null, first_input = null;
	for (let i = e + 1; i < n; i++) {
		if (msk[i].eq) continue;
		const f = msk[i].field;
		if (f === 'input' || f === 'missing') {
			input[f] = (input[f] || 0) + 1;
			if (first_input === null) first_input = i;
		} else {
			entry[f] = (entry[f] || 0) + 1;
			if (first_entry === null) first_entry = { page: i, field: f };
		}
	}
	res.diff_input = input;
	res.diff_entry = entry;
	res.first_entry_diff = first_entry;
	// The line shift: how far the last page's absolute child counter moved.
	const k = Math.min(base.length, ed.length) - 1;
	res.shift = k >= 0 ? parseInt(ed[k].entry.work_idx, 16) - parseInt(base[k].entry.work_idx, 16) : null;
	return res;
}

const pct = (x) => (x === null ? '  -  ' : (100 * x).toFixed(1).padStart(5));

function main() {
	const args = parseArgs(process.argv);
	const [bin, doc] = args.rest;
	if (!bin || !doc) {
		process.stderr.write('usage: replay_ratio.mjs <austenite-binary> <doc.typ> [--work=DIR] [--json=FILE] [--positions=N]\n');
		process.exit(2);
	}
	const work = args.work || fs.mkdtempSync(path.join(process.env.HOME, '.cache', 'replay_ratio_'));
	fs.mkdirSync(work, { recursive: true });
	const text = fs.readFileSync(doc, 'utf8');
	const sites = sitesOf(text);
	if (!sites.length) {
		process.stderr.write('replay_ratio: no edit sites found\n');
		process.exit(1);
	}
	const picks = [];
	for (let i = 0; i < args.positions; i++) picks.push(sites[Math.min(Math.floor((i / args.positions) * sites.length), sites.length - 1)]);

	const base = compile(bin, text, work, 'base');
	// The control: the unedited text again must give an identical record for every page.
	const ctl = analyse(base.pages, compile(bin, text, work, 'ctl').pages);
	const probes = [base.probe_ns_page];
	const edits = [];
	const run = (kind, pos, site, insert, letter) => {
		const ed = compile(bin, text.slice(0, site.at) + insert + text.slice(site.at), work, 'ed');
		probes.push(ed.probe_ns_page);
		const a = analyse(base.pages, ed.pages);
		edits.push({ kind, pos, site: site.name, letter, ...a, probe_ns_page: ed.probe_ns_page, wall_s: ed.wall_s });
		process.stderr.write(`${kind} pos ${pos} ${site.name} ${letter || ''}: raw ${pct(a.raw_equal)} masked ${pct(a.masked_equal)} resync ${a.resync_raw}/${a.resync_masked}\n`);
	};
	for (let p = 0; p < picks.length; p++) {
		for (const letter of ['q', 'z']) run('letter', p, picks[p], letter, letter);
	}
	run('clause', 2, picks[Math.min(2, picks.length - 1)], CLAUSE, null);
	run('clause+', 2, picks[Math.min(2, picks.length - 1)], CLAUSE_LONG, null);

	const letters = edits.filter((x) => x.kind === 'letter');
	const mean = (xs) => (xs.length ? xs.reduce((s, x) => s + x, 0) / xs.length : null);
	const dist = (xs) => {
		const ok = xs.filter((x) => x !== null);
		return { median: median(ok), worst: ok.length ? Math.max(...ok) : null, none: xs.length - ok.length, n: xs.length };
	};
	const hist = (key) => {
		const h = {};
		for (const x of letters) for (const [k, v] of Object.entries(x[key])) h[k] = (h[k] || 0) + v;
		return h;
	};
	const summary = {
		doc: path.basename(doc), pages: base.pages.length, sites: sites.length,
		control_raw_equal: ctl.raw_equal,
		letter_raw_equal: mean(letters.map((x) => x.raw_equal)),
		letter_masked_equal: mean(letters.map((x) => x.masked_equal)),
		letter_raw_after: mean(letters.map((x) => x.raw_after).filter((x) => x !== null)),
		letter_masked_after: mean(letters.map((x) => x.masked_after).filter((x) => x !== null)),
		letter_resync_raw: dist(letters.map((x) => x.resync_raw)),
		letter_resync_masked: dist(letters.map((x) => x.resync_masked)),
		letter_diff_input: hist('diff_input'),
		letter_diff_entry: hist('diff_entry'),
		clause: edits.filter((x) => x.kind.startsWith('clause')),
		letter_keyed_equal: mean(letters.map((x) => x.keyed_equal)),
		probe_ns_page_median: median(probes),
		probe_ns_page_max: Math.max(...probes),
		compile_wall_s: base.wall_s,
	};

	const lines = [`${summary.doc}: ${summary.pages} pages, ${summary.sites} sites; control raw ${pct(ctl.raw_equal)}%`];
	lines.push('kind    pos site          raw%  msk%  key%  rawA% mskA%  edp  rsyR rsyM shift pgs  entry-diff fields (masked, after edited)');
	for (const x of edits) {
		lines.push(`${x.kind.padEnd(7)} ${String(x.pos).padStart(3)} ${x.site.padEnd(12)} ${pct(x.raw_equal)} ${pct(x.masked_equal)} ${pct(x.keyed_equal)} ${pct(x.raw_after)} ${pct(x.masked_after)} `
			+ `${String(x.edited).padStart(4)} ${String(x.resync_raw).padStart(5)} ${String(x.resync_masked).padStart(4)} ${String(x.shift).padStart(5)} ${String(x.pages_edit - x.pages_base).padStart(3)}  `
			+ `${JSON.stringify(x.diff_entry)} in:${JSON.stringify(x.diff_input)}`);
	}
	lines.push(`one-letter mean: raw ${pct(summary.letter_raw_equal)}%  masked ${pct(summary.letter_masked_equal)}%  (after edited page: raw ${pct(summary.letter_raw_after)}%  masked ${pct(summary.letter_masked_after)}%)`);
	lines.push(`content-keyed reuse (one-letter mean): ${pct(summary.letter_keyed_equal)}%`);
	lines.push(`resync raw ${JSON.stringify(summary.letter_resync_raw)}  masked ${JSON.stringify(summary.letter_resync_masked)}`);
	lines.push(`probe ns/page median ${Math.round(summary.probe_ns_page_median)} max ${Math.round(summary.probe_ns_page_max)} (dev build); compile ${summary.compile_wall_s.toFixed(1)} s`);
	process.stdout.write(lines.join('\n') + '\n');
	if (args.json) fs.writeFileSync(args.json, JSON.stringify({ summary, edits, control: ctl }, null, 1) + '\n');
}

main();
