#!/usr/bin/env node
// S0 incremental edit-latency bench: a scripted one-character edit to a
// paragraph, timed to the recompiled page -- Austenite's compileProjectDelta
// (changed-only) against typst.ts's real Daimond wiring, which is a full
// recompile to the `vector` format on every edit (see lib/wasm_common.mjs and
// tools/bench/README.md; Daimond does not call typst.ts's incr_compile/
// IncrServer API anywhere, so timing that API here would not be the number a
// Daimond user feels).
//
// The compiler instance and, for Austenite, the `known`-id cache are kept live
// across every edit in this one process, matching how the real watch loop
// holds them for the life of a view.
//
// Usage: edit_latency.mjs --engine=austenite|typstts --vendor=DIR --doc=FILE
//        [--edits=N] [--positions=N]
// Prints one JSON summary to stdout: { doc, engine, samples: [s...], p50_s, p95_s, ok }.
import fs from 'node:fs';
import { loadEngine, timeOnce } from './lib/wasm_common.mjs';

function parseArgs(argv) {
	const out = { edits: 10, positions: 5 };
	for (const a of argv.slice(2)) {
		const m = a.match(/^--([^=]+)=(.*)$/);
		if (!m) continue;
		const [, k, v] = m;
		out[k] = /^\d+$/.test(v) ? Number(v) : v;
	}
	return out;
}

function percentile(sorted, p) {
	if (!sorted.length) return null;
	const idx = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
	return sorted[idx];
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
	const tokens = [...text.matchAll(/EDITTOK(\d+)/g)].map((m) => m[0]);
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

	// One initial full compile, untimed (the doc's first load, not an edit).
	{
		const project = { main: '/main.typ', sources: [['/main.typ', text]], known: [] };
		const ret = engine.compileDelta(project);
		if (ret && Array.isArray(ret.order)) known = ret.order;
		if (ret && ret.error) ok = false;
	}

	for (let i = 0; i < args.edits; i++) {
		const tok = positions[i % positions.length];
		editCounter += 1;
		// A one-character edit: append a digit to the sentinel token, distinct
		// each time, so the paragraph's text genuinely differs run to run.
		const marker = `${tok}e${editCounter}`;
		text = text.split(tok).join(marker);
		// The token at this position now reads `marker`; later edits at the same
		// position must search for that, not the original `EDITTOKn`.
		positions[i % positions.length] = marker;

		const project = { main: '/main.typ', sources: [['/main.typ', text]], known };
		const { wall_s, ret } = timeOnce(() => engine.compileDelta(project));
		if (!ret || ret.error) {
			ok = false;
			process.stderr.write(`edit_latency: edit ${i} error: ${ret && ret.error}\n`);
		} else if (Array.isArray(ret.order)) {
			known = ret.order;
		}
		samples.push(wall_s);
	}

	const sorted = [...samples].sort((a, b) => a - b);
	process.stdout.write(
		JSON.stringify({
			doc: args.doc,
			engine: args.engine,
			edits: args.edits,
			positions: args.positions,
			samples,
			p50_s: percentile(sorted, 50),
			p95_s: percentile(sorted, 95),
			ok,
		}) + '\n'
	);
}

main().catch((e) => {
	process.stdout.write(JSON.stringify({ ok: false, error: String((e && e.stack) || e) }) + '\n');
	process.exit(1);
});
