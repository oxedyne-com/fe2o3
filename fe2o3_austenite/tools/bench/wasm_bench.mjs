#!/usr/bin/env node
// S0 wasm full-compile bench: Austenite wasm vs typst.ts wasm, in node, driven
// as Daimond drives them (see lib/wasm_common.mjs for the one deviation --
// fs reads in place of the browser's fetch). One process per (doc, engine)
// invocation, so the memory cap and peak-RSS reading in wasm_bench_runner.sh
// isolate one engine at a time; warm-ups and measured runs share the one long-
// lived compiler instance, matching how Daimond keeps it (see getCompiler()).
//
// Usage: wasm_bench.mjs --engine=austenite|typstts --vendor=DIR --doc=FILE
//        [--warmups=N] [--runs=N] [--fmt=pdf|vector]
// Prints one JSON object to stdout: { doc, engine, fmt, runs: [wall_s...], ok }.
import { loadEngine, projectFromFile, timeOnce } from './lib/wasm_common.mjs';

function parseArgs(argv) {
	const out = { warmups: 1, runs: 3, fmt: 'pdf' };
	for (const a of argv.slice(2)) {
		const m = a.match(/^--([^=]+)=(.*)$/);
		if (!m) continue;
		const [, k, v] = m;
		out[k] = /^\d+$/.test(v) ? Number(v) : v;
	}
	return out;
}

async function main() {
	const args = parseArgs(process.argv);
	for (const req of ['engine', 'vendor', 'doc']) {
		if (!args[req]) {
			process.stderr.write(`wasm_bench: --${req} is required\n`);
			process.exit(2);
		}
	}
	const engine = await loadEngine(args.engine, args.vendor);
	const project = projectFromFile(args.doc);
	const compileFn = args.fmt === 'vector' ? engine.compileVector : engine.compilePdf;

	for (let i = 0; i < args.warmups; i++) {
		const { ret } = timeOnce(() => compileFn.call(engine, project));
		if (ret && ret.error) {
			process.stderr.write(`wasm_bench: warm-up compile error: ${ret.error}\n`);
		}
	}

	const runs = [];
	let ok = true;
	for (let i = 0; i < args.runs; i++) {
		const { wall_s, ret } = timeOnce(() => compileFn.call(engine, project));
		if (!ret || ret.error) {
			ok = false;
			process.stderr.write(`wasm_bench: run ${i} error: ${ret && ret.error}\n`);
		}
		runs.push(wall_s);
	}

	process.stdout.write(
		JSON.stringify({ doc: args.doc, engine: args.engine, fmt: args.fmt, runs, ok }) + '\n'
	);
}

main().catch((e) => {
	process.stdout.write(JSON.stringify({ ok: false, error: String((e && e.stack) || e) }) + '\n');
	process.exit(1);
});
