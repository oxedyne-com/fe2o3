#!/usr/bin/env node
// S0 wasm full-compile bench: Austenite wasm vs typst.ts wasm, in node, driven
// as Daimond drives them (see lib/wasm_common.mjs for the one deviation --
// fs reads in place of the browser's fetch). One process per (doc, engine,
// mode) invocation, so the memory cap and peak-RSS reading in
// wasm_bench_runner.sh isolate one leg at a time.
//
// Three modes, chosen with --mode, named as check G11 names them (review
// item 18: a "70x" figure was withdrawn because it compared a cold compile
// against typst.ts's comemo cache hit; every row now carries its mode):
//
//   cold       A fresh wasm instance -- new linear memory, so no static cache
//              survives, typst.ts's comemo included -- is loaded for every
//              measured run; the compile is timed, the load reported beside
//              it (`load_s`). A new compiler object on the old instance would
//              not do: the instance's statics outlive it. The runner starts
//              one process per cold run, so the engine's code is cold too.
//   unchanged  One long-lived instance; after the warm-ups, every measured
//              run recompiles the same unchanged project. For an engine that
//              memoises unchanged input this times its cache hit, which is
//              what an idle live view costs.
//   edit       One long-lived instance, warmed on the unchanged project; each
//              measured run first inserts one more letter into a word near
//              the middle of the document, so every source is new and a
//              memoising engine recompiles only what the edit reaches. This
//              is one keystroke's recompile, full format (pdf or vector).
//
// Usage: wasm_bench.mjs --engine=austenite|typstts --vendor=DIR --doc=FILE
//        [--mode=cold|unchanged|edit] [--warmups=N] [--runs=N]
//        [--fmt=pdf|vector]
// Prints one JSON object to stdout:
//   { doc, engine, mode, fmt, runs: [wall_s...], load_s: [s...], ok }.
import { loadEngine, projectFromFile, timeOnce } from './lib/wasm_common.mjs';

const MODES = ['cold', 'unchanged', 'edit'];

function parseArgs(argv) {
	const out = { warmups: 1, runs: 3, fmt: 'pdf', mode: 'unchanged' };
	for (const a of argv.slice(2)) {
		const m = a.match(/^--([^=]+)=(.*)$/);
		if (!m) continue;
		const [, k, v] = m;
		out[k] = /^\d+$/.test(v) ? Number(v) : v;
	}
	return out;
}

// The offset just inside a word nearest the middle of the source, where the
// edit mode types: the first lower-case letter from the midpoint on.
function editPoint(text) {
	let i = Math.floor(text.length / 2);
	while (i < text.length && !/[a-z]/.test(text[i])) i++;
	return i < text.length ? i + 1 : text.length;
}

// The source after `n` keystrokes at `at`: n letters typed into the word.
function typed(text, at, n) {
	let ins = '';
	for (let k = 0; k < n; k++) ins += String.fromCharCode(97 + (k % 26));
	return text.slice(0, at) + ins + text.slice(at);
}

async function main() {
	const args = parseArgs(process.argv);
	for (const req of ['engine', 'vendor', 'doc']) {
		if (!args[req]) {
			process.stderr.write(`wasm_bench: --${req} is required\n`);
			process.exit(2);
		}
	}
	if (!MODES.includes(args.mode)) {
		process.stderr.write(`wasm_bench: --mode must be one of ${MODES.join(', ')}\n`);
		process.exit(2);
	}

	const project = projectFromFile(args.doc);
	const [mainPath, baseText] = project.sources[0];
	const withText = (text) => ({ main: project.main, sources: [[mainPath, text]] });
	const runs = [];
	const loads = [];
	let ok = true;
	const check = (ret, what) => {
		if (!ret || ret.error) {
			ok = false;
			process.stderr.write(`wasm_bench: ${what} error: ${ret && ret.error}\n`);
		}
	};

	if (args.mode === 'cold') {
		// Every run, warm-up or measured, loads and discards its own instance --
		// there is nothing for a warm-up to warm, so it is skipped and noted.
		if (args.warmups > 0) {
			process.stderr.write('wasm_bench: cold mode skips warm-ups (each run is already cold)\n');
		}
		for (let i = 0; i < args.runs; i++) {
			const t0 = process.hrtime.bigint();
			const engine = await loadEngine(args.engine, args.vendor, true);
			loads.push(Number(process.hrtime.bigint() - t0) / 1e9);
			const compileFn = args.fmt === 'vector' ? engine.compileVector : engine.compilePdf;
			const { wall_s, ret } = timeOnce(() => compileFn.call(engine, project));
			check(ret, `run ${i}`);
			runs.push(wall_s);
		}
	} else {
		// unchanged and edit share one long-lived instance, warmed on the
		// unchanged project; they differ only in the source each run compiles.
		const t0 = process.hrtime.bigint();
		const engine = await loadEngine(args.engine, args.vendor);
		loads.push(Number(process.hrtime.bigint() - t0) / 1e9);
		const compileFn = args.fmt === 'vector' ? engine.compileVector : engine.compilePdf;
		for (let i = 0; i < args.warmups; i++) {
			const { ret } = timeOnce(() => compileFn.call(engine, project));
			check(ret, `warm-up ${i}`);
		}
		const at = editPoint(baseText);
		for (let i = 0; i < args.runs; i++) {
			const runProject = args.mode === 'edit' ? withText(typed(baseText, at, i + 1)) : project;
			const { wall_s, ret } = timeOnce(() => compileFn.call(engine, runProject));
			check(ret, `run ${i}`);
			runs.push(wall_s);
		}
	}

	process.stdout.write(
		JSON.stringify({ doc: args.doc, engine: args.engine, mode: args.mode, fmt: args.fmt, runs, load_s: loads, ok }) + '\n'
	);
}

main().catch((e) => {
	process.stdout.write(JSON.stringify({ ok: false, error: String((e && e.stack) || e) }) + '\n');
	process.exit(1);
});
