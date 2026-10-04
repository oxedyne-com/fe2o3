#!/usr/bin/env node
// S0 incremental edit-latency bench: scripted typing, one character per edit
// into a word at each of several paragraphs in turn, each edit timed to the
// recompiled page -- Austenite's compileProjectDelta
// (changed-only) against typst.ts's real Daimond wiring, which is a full
// recompile to the `vector` format on every edit (see lib/wasm_common.mjs and
// tools/bench/README.md; Daimond does not call typst.ts's incr_compile/
// IncrServer API anywhere, so timing that API here would not be the number a
// Daimond user feels).
//
// One run of this script is one session, as D-20261001-08 defines it: one
// fresh node process holding one compiler instance, an untimed open, then
// `--edits` timed edits. The compiler instance and, for Austenite, the
// `known`-id cache are kept live across every edit in this one process,
// matching how the real watch loop holds them for the life of a view.
//
// The positions are the EDITTOK markers at 0, 1/N, 2/N ... of the marker list
// (N = --positions), visited round-robin, so 50 edits at 5 positions is 10 edits
// at each. A sample is the wall time from just before the compile call to its
// return, with the drawable result in hand (Austenite: the delta object with
// every changed page's SVG string walked; typst.ts: the vector bytes). Painting
// is excluded for both engines. A sample that returns `{error}`, or whose
// `pages` differs from the open's (Austenite reports it; typst.ts's vector bytes
// do not), ends the session as invalid.
//
// Austenite's projects carry `strict: true`, as Daimond sends it on every door;
// typst.ts ignores the field.
//
// Usage: edit_latency.mjs --engine=austenite|typstts --vendor=DIR --doc=FILE
//        [--edits=N] [--positions=N]
// Prints one JSON summary to stdout: { doc, engine, samples: [s...], p50_s, p95_s,
// ok, invalid, open_s, pages, ... }. Percentiles are nearest-rank (lib/stats.mjs).
import fs from 'node:fs';
import { loadEngine, timeOnce, loadavg1, psiAvg10 } from './lib/wasm_common.mjs';
import { nearestRank } from './lib/stats.mjs';

function parseArgs(argv) {
	const out = { edits: 50, positions: 5 };
	for (const a of argv.slice(2)) {
		const m = a.match(/^--([^=]+)=(.*)$/);
		if (!m) continue;
		const [, k, v] = m;
		out[k] = /^\d+$/.test(v) ? Number(v) : v;
	}
	return out;
}

// The drawable result of one compile, in hand: Austenite's changed pages with their SVG strings
// walked (the walk is inside the timed call, so a lazily built string would be paid for), or the
// vector bytes from typst.ts. Returns the size of what was walked, or -1 when there is none.
function drawable(ret) {
	if (!ret || ret.error) return -1;
	let n = 0;
	if (Array.isArray(ret.changed)) {
		for (const c of ret.changed) n += typeof c.svg === 'string' ? c.svg.length : 0;
		return n;
	}
	if (ret.vector && typeof ret.vector.length === 'number') return ret.vector.length;
	return -1;
}

async function main() {
	const args = parseArgs(process.argv);
	for (const req of ['engine', 'vendor', 'doc']) {
		if (!args[req]) {
			process.stderr.write(`edit_latency: --${req} is required\n`);
			process.exit(2);
		}
	}
	let text = fs.readFileSync(args.doc, 'utf8');
	// Each marker is followed by a space, so `EDITTOK1 ` names paragraph 1 alone and never the start of
	// `EDITTOK10`; matching the bare marker would edit every paragraph whose number it prefixes.
	const tokens = [...text.matchAll(/EDITTOK(\d+) /g)].map((m) => m[0].slice(0, -1));
	const uniqueTokens = [...new Set(tokens)];
	if (!uniqueTokens.length) {
		process.stdout.write(JSON.stringify({ ok: false, error: 'no EDITTOK markers found; doc not from gen_synthetic.py' }) + '\n');
		process.exit(1);
	}
	const positions = [];
	for (let i = 0; i < args.positions; i++) {
		const idx = Math.floor((i / args.positions) * uniqueTokens.length);
		positions.push(uniqueTokens[Math.min(idx, uniqueTokens.length - 1)]);
	}

	const engine = await loadEngine(args.engine, args.vendor);
	let known = [];
	let editCounter = 0;
	const samples = [];
	let ok = true;
	let invalid = null;
	let openPages = null;
	let open_s = null;

	// One initial full compile, untimed (the doc's first load, not an edit), but its duration is kept.
	{
		const project = { main: '/main.typ', sources: [['/main.typ', text]], known: [], strict: true };
		const { wall_s, ret } = timeOnce(() => engine.compileDelta(project));
		open_s = wall_s;
		if (ret && Array.isArray(ret.order)) known = ret.order;
		if (!ret || ret.error) {
			ok = false;
			invalid = `open returned an error: ${ret && ret.error}`;
		} else if (typeof ret.pages === 'number') {
			openPages = ret.pages;
		}
	}

	const start = { load1: loadavg1(), psi_cpu: psiAvg10('cpu') };
	for (let i = 0; i < args.edits && ok; i++) {
		const tok = positions[i % positions.length];
		editCounter += 1;
		// One keystroke: a letter typed onto the end of the word at this position, so each edit adds
		// exactly one character and every source differs from every one before it.
		const letter = String.fromCharCode(97 + (editCounter % 26));
		const marker = `${tok}${letter}`;
		const at = text.indexOf(`${tok} `);
		if (at < 0) {
			ok = false;
			invalid = `marker ${tok} is gone from the source`;
			process.stderr.write(`edit_latency: ${invalid}\n`);
			break;
		}
		text = text.slice(0, at) + marker + text.slice(at + tok.length);
		// The word at this position now reads `marker`; the next edit here types onto that.
		positions[i % positions.length] = marker;

		const project = { main: '/main.typ', sources: [['/main.typ', text]], known, strict: true };
		let size = -1;
		const { wall_s, ret } = timeOnce(() => {
			const r = engine.compileDelta(project);
			size = drawable(r);
			return r;
		});
		if (!ret || ret.error) {
			ok = false;
			invalid = `edit ${i} returned an error: ${ret && ret.error}`;
			process.stderr.write(`edit_latency: ${invalid}\n`);
			break;
		}
		if (size < 0) {
			ok = false;
			invalid = `edit ${i} returned no drawable result`;
			break;
		}
		if (openPages !== null && ret.pages !== openPages) {
			ok = false;
			invalid = `edit ${i} has ${ret.pages} pages, the open had ${openPages}`;
			break;
		}
		if (Array.isArray(ret.order)) known = ret.order;
		samples.push(wall_s);
	}
	const end = { load1: loadavg1(), psi_cpu: psiAvg10('cpu') };

	const sorted = [...samples].sort((a, b) => a - b);
	process.stdout.write(
		JSON.stringify({
			doc: args.doc,
			engine: args.engine,
			edits: args.edits,
			positions: args.positions,
			samples,
			p50_s: nearestRank(sorted, 50),
			p95_s: nearestRank(sorted, 95),
			ok,
			invalid,
			open_s,
			pages: openPages,
			timed_start: start,
			timed_end: end,
		}) + '\n'
	);
}

main().catch((e) => {
	process.stdout.write(JSON.stringify({ ok: false, error: String((e && e.stack) || e) }) + '\n');
	process.exit(1);
});
