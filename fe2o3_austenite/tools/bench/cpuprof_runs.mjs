#!/usr/bin/env node
// One CPU-profiled run of the Austenite wasm, for cpuprof_rank.mjs. The profiler is node's own (the
// inspector's Profiler domain, the engine behind `--cpu-prof`) switched on in process around only the
// calls being measured, so loading the wasm and the untimed open are not in the profile.
//
//   cold       a fresh instance; one `compileProject` (PDF), the profile covering that call;
//   warm       one untimed `compileProject`, then the profile covers a second of the same project;
//   edits      an untimed `compileProjectDelta` open with `known: []`, then --edits one-letter edits, each
//              a `compileProjectDelta` carrying the previous result's `order` as `known` (the live view's
//              own path) with every changed page's SVG string walked, the profile covering all of them;
//   unchanged  the same open, then --reps `compileProjectDelta` calls of the unchanged source carrying
//              `known`, the profile covering them.
//
// The edits are those of edit_latency.mjs: a letter typed onto the word at an EDITTOK marker, the markers
// at 0, 1/N, 2/N ... of the list (N = --positions) visited round-robin.
//
// Usage: cpuprof_runs.mjs --vendor=DIR --doc=FILE --mode=cold|warm|edits|unchanged --out=FILE.cpuprofile
//        [--edits=10] [--positions=5] [--reps=1] [--interval=100]
// Prints one JSON line: the wall time of each profiled call (inflated a little by the sampling), the page
// count and the sizes.
import fs from 'node:fs';
import inspector from 'node:inspector';
import { loadAustenite, projectFromFile, timeOnce } from './lib/wasm_common.mjs';

function parseArgs(argv) {
	const out = { edits: 10, positions: 5, reps: 1, interval: 100 };
	for (const a of argv.slice(2)) {
		const m = a.match(/^--([^=]+)=(.*)$/);
		if (!m) continue;
		out[m[1]] = /^\d+$/.test(m[2]) ? Number(m[2]) : m[2];
	}
	return out;
}

function drawable(ret) {
	if (!ret || ret.error) return -1;
	let n = 0;
	for (const c of ret.changed || []) n += typeof c.svg === 'string' ? c.svg.length : 0;
	return n;
}

function post(session, method, params) {
	return new Promise((resolve, reject) => {
		session.post(method, params, (e, r) => (e ? reject(e) : resolve(r)));
	});
}

async function main() {
	const args = parseArgs(process.argv);
	for (const req of ['vendor', 'doc', 'mode', 'out']) {
		if (!args[req]) {
			process.stderr.write(`cpuprof_runs: --${req} is required\n`);
			process.exit(2);
		}
	}
	const engine = await loadAustenite(args.vendor);
	const session = new inspector.Session();
	session.connect();
	await post(session, 'Profiler.enable');
	await post(session, 'Profiler.setSamplingInterval', { interval: args.interval });
	const result = { mode: args.mode, doc: args.doc, calls_s: [], ok: true };

	const profiled = async (fn) => {
		await post(session, 'Profiler.start');
		try {
			fn();
		} finally {
			const { profile } = await post(session, 'Profiler.stop');
			fs.writeFileSync(args.out, JSON.stringify(profile));
		}
	};

	const base = projectFromFile(args.doc);
	if (args.mode === 'cold' || args.mode === 'warm') {
		if (args.mode === 'warm') {
			const { wall_s, ret } = timeOnce(() => engine.compilePdf(base));
			result.open_s = wall_s;
			if (!ret || ret.error) throw new Error(`open failed: ${ret && ret.error}`);
		}
		await profiled(() => {
			const { wall_s, ret } = timeOnce(() => engine.compilePdf(base));
			if (!ret || ret.error) {
				result.ok = false;
				result.error = String(ret && ret.error).slice(0, 300);
				return;
			}
			result.calls_s.push(wall_s);
			result.pages = ret.pages;
			result.bytes = ret.pdf ? ret.pdf.length : 0;
		});
	} else if (args.mode === 'edits' || args.mode === 'unchanged') {
		let text = base.sources[0][1];
		const open = { ...base, known: [] };
		const { wall_s, ret } = timeOnce(() => engine.compileDelta(open));
		result.open_s = wall_s;
		if (!ret || ret.error || !Array.isArray(ret.order)) throw new Error(`open failed: ${ret && ret.error}`);
		let known = ret.order;
		result.pages = ret.pages;
		if (args.mode === 'unchanged') {
			await profiled(() => {
				for (let i = 0; i < args.reps; i++) {
					const { wall_s: w, ret: r } = timeOnce(() => {
						const x = engine.compileDelta({ ...base, known });
						drawable(x);
						return x;
					});
					if (!r || r.error || r.changed.length !== 0) {
						result.ok = false;
						result.error = `an unchanged recompile returned ${r && r.error ? r.error : r.changed.length + ' changed pages'}`;
						return;
					}
					known = r.order;
					result.calls_s.push(w);
				}
			});
		} else {
			const tokens = [...new Set([...text.matchAll(/EDITTOK(\d+) /g)].map((m) => m[0].slice(0, -1)))];
			if (!tokens.length) throw new Error('no EDITTOK markers; doc not from gen_synthetic.py');
			const positions = [];
			for (let i = 0; i < args.positions; i++) {
				positions.push(tokens[Math.min(Math.floor((i / args.positions) * tokens.length), tokens.length - 1)]);
			}
			result.changed = [];
			await profiled(() => {
				for (let i = 0; i < args.edits; i++) {
					const tok = positions[i % positions.length];
					const marker = `${tok}${String.fromCharCode(97 + ((i + 1) % 26))}`;
					const at = text.indexOf(`${tok} `);
					if (at < 0) throw new Error(`marker ${tok} is gone`);
					text = text.slice(0, at) + marker + text.slice(at + tok.length);
					positions[i % positions.length] = marker;
					const project = { main: '/main.typ', sources: [['/main.typ', text]], known, strict: true };
					const { wall_s: w, ret: r } = timeOnce(() => {
						const x = engine.compileDelta(project);
						drawable(x);
						return x;
					});
					if (!r || r.error) {
						result.ok = false;
						result.error = `edit ${i}: ${r && r.error}`;
						return;
					}
					known = r.order;
					result.calls_s.push(w);
					result.changed.push(r.changed.length);
				}
			});
		}
	} else {
		throw new Error(`unknown mode ${args.mode}`);
	}
	result.heap_mb = engine.heapMB ? engine.heapMB() : null;
	process.stdout.write(JSON.stringify(result) + '\n');
}

main().catch((e) => {
	process.stdout.write(JSON.stringify({ ok: false, error: String((e && e.stack) || e) }) + '\n');
	process.exit(1);
});
