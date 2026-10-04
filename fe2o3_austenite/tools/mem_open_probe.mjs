#!/usr/bin/env node
// mem_open_probe.mjs -- where the wasm heap goes on the live view's open, one experiment a process.
//
// The wasm linear memory only grows, so `heapMB()` after a call is that process's high-water mark so
// far. Each experiment runs in a fresh node process and prints one JSON line with the heap after each
// of its calls, the bytes of SVG the delta carried, and the page count.
//
//   mem_open_probe.mjs --pkg DIR --doc FILE.typ --exp NAME [--order FILE] [--out FILE]
//
// Experiments:
//   pdf            one `compileProject` (PDF): the engine's own working set, the sink streaming;
//   open           `compileProjectDelta` with `known: []`, three times in one instance: does the second
//                  and third open reuse the heap the first grew, or grow it again;
//   vector         one `compileProjectVector` (every page's SVG in one array), the other live-view door;
//   pdf_open       a PDF, then an open: is the open's heap the engine's, or the open's own;
//   open_pdf       an open, then a PDF in the same instance: does the heap come back down (it cannot);
//   open1          one open, the result held as a consumer holds it, with node's own figures beside the wasm's
//                  (RSS, V8 heap, external and array buffers) before, after and, run with --expose-gc,
//                  after the result is dropped (then --edits N edits, if asked), and the process RSS sampled every 250 ms from a worker thread
//                  while the open runs (the call is synchronous, so the wasm size cannot be read inside it;
//                  the partial experiments give it at 1, 10, 30 and 100 pages held instead);
//   edits          an open, then --edits one-letter edits carrying `known`: what an edit adds;
//   ids            an open, writing the page ids in order to --out for `partial`;
//   partial:K      an open whose `known` holds every id but the last K distinct ones (from --order), so
//                  K pages are rendered to SVG and held, the rest only laid out, hashed and dropped.
//
// The edits are heap_probe.mjs's: a letter typed onto a plain word of a body paragraph, the paragraphs
// spread evenly through the document.
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { Worker } from 'node:worker_threads';

function arg(name, dflt) {
	const a = process.argv.find((x) => x.startsWith(`--${name}=`));
	if (a) return a.slice(name.length + 3);
	const i = process.argv.indexOf(`--${name}`);
	return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : dflt;
}

const pkg = arg('pkg');
const doc = arg('doc');
const exp = arg('exp');
if (!pkg || !doc || !exp) {
	process.stderr.write('mem_open_probe: --pkg, --doc and --exp are required\n');
	process.exit(2);
}

const mod = await import(pathToFileURL(path.join(pkg, 'oxedyne_fe2o3_austenite.js')).href);
const wasm = fs.readFileSync(path.join(pkg, 'oxedyne_fe2o3_austenite_bg.wasm'));
const exports = await mod.default({ module_or_path: wasm });
const aus = new mod.DaimondTypst();
const src = fs.readFileSync(doc, 'utf8');
const mib = () => exports.memory.buffer.byteLength / 1048576;
const hwm = () => {
	const m = /VmHWM:\s+(\d+) kB/.exec(fs.readFileSync('/proc/self/status', 'utf8'));
	return m ? +m[1] / 1024 : 0;
};
const project = (text, extra) => ({ main: '/main.typ', sources: [['/main.typ', text]], assets: [], fonts: [], strict: true, ...extra });

const rec = { exp, doc: path.basename(doc), heap_start_mb: mib(), calls: [] };
const note = (what, ret, extra) => {
	const e = { what, heap_mb: mib(), ...extra };
	if (ret && ret.error) e.error = String(ret.error).slice(0, 200);
	if (ret && ret.pages !== undefined) e.pages = ret.pages;
	rec.calls.push(e);
	return e;
};
const svgBytes = (ret) => {
	let n = 0;
	for (const c of ret.changed || []) n += c.svg.length;
	return n;
};
const open = (known) => {
	const t = performance.now();
	const ret = aus.compileProjectDelta(project(src, { known }));
	const ms = Math.round(performance.now() - t);
	if (!ret || ret.error) return { ret, e: note('open', ret, { ms }) };
	const e = note('open', ret, { ms, changed: ret.changed.length, svg_bytes: svgBytes(ret), order: ret.order.length, distinct: new Set(ret.order).size });
	return { ret, e };
};
const pdf = () => {
	const t = performance.now();
	const ret = aus.compileProject(project(src));
	const e = note('pdf', ret, { ms: Math.round(performance.now() - t), bytes: ret.pdf ? ret.pdf.length : 0 });
	return { ret, e };
};

// `n` one-letter edits after an open, each carrying the previous `order` as `known`.
const runEdits = (known, n) => {
	let text = src;
	const lines = text.split('\n');
	const body = [];
	for (let i = 0; i < lines.length; i++) {
		if (lines[i].length > 80 && !/^[=\-#$]/.test(lines[i])) body.push(i);
	}
	for (let k = 0; k < n; k++) {
		const at = body[Math.min(Math.floor(((k + 0.5) / n) * body.length), body.length - 1)];
		const words = lines[at].split(' ');
		const w = words.findIndex((x) => /^[a-z]{4,}$/.test(x));
		words[w] += 'x';
		lines[at] = words.join(' ');
		text = lines.join('\n');
		const t = performance.now();
		const r = aus.compileProjectDelta(project(text, { known }));
		const e = note('edit', r, { ms: Math.round(performance.now() - t) });
		if (r && !r.error) {
			e.changed = r.changed.length;
			e.svg_bytes = svgBytes(r);
			known = r.order;
		}
	}
};

if (exp === 'pdf') {
	pdf();
} else if (exp === 'open') {
	for (let i = 0; i < 3; i++) open([]);
} else if (exp === 'vector') {
	const t = performance.now();
	const ret = aus.compileProjectVector(project(src));
	let n = 0;
	for (const s of ret.svg || []) n += s.length;
	note('vector', ret, { ms: Math.round(performance.now() - t), svg_bytes: n });
} else if (exp === 'pdf_open') {
	pdf();
	open([]);
} else if (exp === 'open_pdf') {
	open([]);
	pdf();
} else if (exp === 'open1') {
	const use = () => {
		const u = process.memoryUsage();
		const m = (b) => Math.round((b / 1048576) * 10) / 10;
		return { wasm_mb: mib(), rss_mb: m(u.rss), v8_heap_used_mb: m(u.heapUsed), external_mb: m(u.external), array_buffers_mb: m(u.arrayBuffers), vmhwm_mb: Math.round(hwm()) };
	};
	// A worker thread keeps sampling the process RSS while the main thread is inside the wasm call.
	const sab = new SharedArrayBuffer(4 * (1 + 2 * 8192));
	const cells = new Int32Array(sab);
	const worker = new Worker(`
		const { workerData } = require('node:worker_threads');
		const fs = require('node:fs');
		const a = new Int32Array(workerData);
		const t0 = Date.now();
		let n = 0;
		setInterval(() => {
			if (n >= 8190) return;
			const m = /VmRSS:\\s+(\\d+) kB/.exec(fs.readFileSync('/proc/self/status', 'utf8'));
			a[1 + 2 * n] = Date.now() - t0;
			a[2 + 2 * n] = m ? +m[1] : 0;
			n++;
			Atomics.store(a, 0, n);
		}, 250);
	`, { eval: true, workerData: sab });
	await new Promise((r) => setTimeout(r, 600));
	rec.before = use();
	let held = open([]).ret;
	rec.after = use();
	const n = Atomics.load(cells, 0);
	const samples = [];
	for (let i = 0; i < n; i++) samples.push([cells[1 + 2 * i] / 1000, Math.round(cells[2 + 2 * i] / 1024)]);
	rec.rss_samples_s_mb = samples.filter((_, i) => i % 8 === 0 || i === n - 1);
	rec.rss_sample_max_mb = Math.max(...samples.map((x) => x[1]));
	await worker.terminate();
	const order = held.order;
	held = null;
	if (global.gc) {
		global.gc();
		rec.after_gc = use();
	}
	if (+arg('edits', '0') > 0) {
		runEdits(order, +arg('edits', '0'));
		rec.after_edits = use();
	}
} else if (exp === 'edits') {
	const { ret } = open([]);
	runEdits(ret.order, +arg('edits', '5'));
} else if (exp === 'ids') {
	const { ret } = open([]);
	fs.writeFileSync(arg('out'), JSON.stringify(ret.order));
} else if (exp.startsWith('partial:')) {
	const k = +exp.slice(8);
	const order = JSON.parse(fs.readFileSync(arg('order'), 'utf8'));
	const distinct = [...new Set(order)];
	const known = distinct.slice(0, Math.max(distinct.length - k, 0));
	open(known);
} else {
	process.stderr.write(`mem_open_probe: unknown experiment ${exp}\n`);
	process.exit(2);
}
rec.heap_end_mb = mib();
rec.vmhwm_mb = hwm();
process.stdout.write(JSON.stringify(rec) + '\n');
