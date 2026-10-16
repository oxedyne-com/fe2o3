#!/usr/bin/env node
// heap_probe.mjs -- peak memory of a compile, Austenite against typst.ts, in node.
//
// Each (engine, page count, run) is a fresh child process, so one compile's heap cannot flatter or
// burden the next: a wasm32 linear memory only ever grows, which makes its `byteLength` after the
// compile the high-water mark of that compile, and the child's `VmHWM` is the kernel's own peak
// resident set for the whole process (node itself included; the baseline before the engine loads
// is reported beside it so the difference can be read).
//
// The corpus is synthetic and deterministic: headings, paragraphs, lists, a small table and an
// equation per section, sized by a words-per-page estimate. The page count each engine actually
// produced is read back from its own PDF (or, on the live view's path, from the delta) and
// printed, so a corpus that came out shorter or longer than asked is visible rather than assumed.
//
// Two modes, the two ways Daimond compiles:
//   pdf    one compile to PDF, the door behind the PDF button and the model's tool;
//   delta  the live view. Austenite: `compileProjectDelta` opened with `known: []`, then --edits
//          one-letter edits each carrying the previous `order` as `known`, the consumer's own
//          cache (id -> SVG) kept as Daimond keeps it. typst.ts has no delta door: its live view is
//          a full `vector` compile on every edit, so the same edits go through that. The heap is
//          read after the open and after each edit.
//
//   node tools/heap_probe.mjs --pages 50,300,1000 --runs 3 --austenite-pkg PKG --json heap.json --gate
//   node tools/heap_probe.mjs --mode delta --pages 50,300,1000 --engines austenite,typst --runs 3
//   node tools/heap_probe.mjs --gate-file heap.json         # judge a saved run; runs nothing
//   tools/heap_gate/check.sh                                # the gate's own check, on synthetic files
//
// Run it under a memory cap, as every heavy job on this host is:
//   systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice node tools/heap_probe.mjs
//
// Options:
//   --pages a,b,c        page counts to generate (default 50,300,1000)
//   --engines x,y,z      `typst` (typst.ts web compiler), `austenite` (--austenite-pkg) and `ref`
//                        (--ref-pkg, Daimond's vendored curated Austenite), default typst,austenite
//   --mode pdf|delta     see above (default pdf)
//   --runs N             fresh processes per cell, the engine order alternating from one run to the
//                        next so neither always runs first on a warm host (default 1)
//   --edits N            one-letter edits after the open, in delta mode (default 5)
//   --typst-vendor DIR   directory holding typst_ts_web_compiler{.mjs,_bg.wasm} and fonts/
//   --austenite-pkg DIR  a wasm-pack `--target web` output directory for fe2o3_austenite
//   --ref-pkg DIR        the same, for the reference row
//   --plain              prose and headings only, for an engine that refuses richer constructs
//   --timeout SEC        per child (default 600)
//   --json FILE          also write every measurement, the per-cell medians and the gate as JSON
//   --gate               exit non-zero unless the memory gate holds (D-20260923-34), see `judge`
//   --gate-file A[,B..]  judge the rows of saved --json files (one cell's runs may be split across
//                        them) and exit; nothing is run
//   --dump PAGES         print the generated source for PAGES and exit

import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HOME = process.env.HOME || '';
const DAIMOND = join(HOME, 'usr/code/web/apps/oxedyne/daimond/www/vendor');
const DEFAULT_TYPST = join(DAIMOND, 'typst');
const DEFAULT_AUST = join(DAIMOND, 'austenite');

// The memory gate: Austenite's peak heap at 300 pages over typst.ts's must not exceed R_GATE (the aim
// is R_AIM), and its growth from 300 to 1,000 pages must not exceed G_GATE MiB for each page it produced.
export const R_GATE = 0.62;
export const R_AIM = 0.5;
export const G_GATE = 0.46;

function opt(name, dflt) {
	const i = process.argv.indexOf(name);
	return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : dflt;
}
const flag = (name) => process.argv.includes(name);

// Corpus

const WORDS = ('the of a and to in is that for it as was with be by on not this are or his from at ' +
	'which but have an they one you were her all she there would their we him been has when who will ' +
	'more no if out so said what up its about into than them can only other new some could time these ' +
	'two may then do first any my now such like our over man me even most made after also did many ' +
	'before must through back years where much your way well down should because each just those people ' +
	'how too little state good very make world still own see men work long get here between both life ' +
	'being under never day same another know while last might us great old year off come since against ' +
	'go came right used take three structure measure element paragraph evaluation realisation margin').split(' ');

// A small deterministic generator (xorshift32), so every run and every engine sees the same text.
function rng(seed) {
	let s = seed >>> 0 || 1;
	return () => {
		s ^= s << 13; s >>>= 0;
		s ^= s >> 17;
		s ^= s << 5; s >>>= 0;
		return s / 4294967296;
	};
}

function sentence(r, n) {
	const w = [];
	for (let i = 0; i < n; i++) w.push(WORDS[Math.floor(r() * WORDS.length)]);
	w[0] = w[0][0].toUpperCase() + w[0].slice(1);
	return w.join(' ') + '.';
}

function paragraph(r, words, rich) {
	const out = [];
	let left = words;
	while (left > 0) {
		const n = Math.min(left, 8 + Math.floor(r() * 12));
		let s = sentence(r, n);
		if (rich && r() < 0.15) s = s.replace(/^(\S+) (\S+)/, '$1 _$2_');
		if (rich && r() < 0.10) s = s.replace(/ (\S+)\.$/, ' *$1*.');
		out.push(s);
		left -= n;
	}
	return out.join(' ');
}

// Calibrated against typst.ts: about 590 of these words to an A4 page of 11pt Libertinus with
// Typst's default margins, headings and blocks included; the read-back page count is what is
// reported, not this estimate.
const WORDS_PER_PAGE = 590;

export function corpus(pages, rich) {
	const r = rng(0x5eed + pages);
	const parts = ['#set heading(numbering: "1.1")', ''];
	const target = pages * WORDS_PER_PAGE;
	let words = 0;
	let chapter = 0;
	let section = 0;
	while (words < target) {
		if (section % 6 === 0) {
			chapter++;
			parts.push('= Chapter ' + chapter + ': ' + sentence(r, 3).slice(0, -1), '');
		}
		section++;
		parts.push('== ' + sentence(r, 4).slice(0, -1), '');
		for (let p = 0; p < 4; p++) {
			const n = 70 + Math.floor(r() * 60);
			parts.push(paragraph(r, n, rich), '');
			words += n;
		}
		if (rich) {
			parts.push('- ' + sentence(r, 6), '- ' + sentence(r, 7), '- ' + sentence(r, 5), '');
			parts.push('#table(columns: 3, [' + WORDS[section % 50] + '], [' + section + '], [' +
				(section * 7) + '], [a], [b], [c])', '');
			parts.push('$ x_' + section + ' = sum_(i=1)^n a_i b_i $', '');
			words += 40;
		}
	}
	return parts.join('\n') + '\n';
}

// `n` successive sources, each the last with one letter typed onto a plain word of a body paragraph,
// the paragraphs spread evenly through the document and visited in turn, as a person typing about
// would. Only a word that is bare lowercase letters is touched, so no edit can break the markup.
export function edited(src, n) {
	const lines = src.split('\n');
	const body = [];
	for (let i = 0; i < lines.length; i++) {
		if (lines[i].length > 80 && !/^[=\-#$]/.test(lines[i])) body.push(i);
	}
	if (!body.length) throw new Error('the corpus has no body paragraph to edit');
	const out = [];
	for (let k = 0; k < n; k++) {
		const at = body[Math.min(Math.floor(((k + 0.5) / n) * body.length), body.length - 1)];
		const words = lines[at].split(' ');
		const w = words.findIndex((x) => /^[a-z]{4,}$/.test(x));
		if (w < 0) throw new Error('paragraph ' + at + ' has no plain word to edit');
		words[w] += 'x';
		lines[at] = words.join(' ');
		out.push(lines.join('\n'));
	}
	return out;
}

function pdfPages(bytes) {
	const s = Buffer.from(bytes).toString('latin1');
	const m = s.match(/\/Type\s*\/Page(?![s\w])/g);
	return m ? m.length : 0;
}

function status() {
	const s = readFileSync('/proc/self/status', 'utf8');
	const kb = (k) => {
		const m = new RegExp(k + ':\\s+(\\d+) kB').exec(s);
		return m ? +m[1] : 0;
	};
	return { rss: kb('VmRSS'), hwm: kb('VmHWM') };
}

// Engines, each run inside a child

async function loadTypst(dir) {
	const mod = await import(pathToFileURL(join(dir, 'typst_ts_web_compiler.mjs')).href);
	const exports = await mod.default({ module_or_path: readFileSync(join(dir, 'typst_ts_web_compiler_bg.wasm')) });
	const builder = new mod.TypstCompilerBuilder();
	builder.set_dummy_access_model();
	const fontDir = join(dir, 'fonts');
	for (const f of readdirSync(fontDir).filter((n) => /\.(otf|ttf)$/i.test(n)).sort()) {
		await builder.add_raw_font(new Uint8Array(readFileSync(join(fontDir, f))));
	}
	const compiler = await builder.build();
	const run = (src, kind) => {
		compiler.reset_shadow();
		compiler.add_source('/main.typ', src);
		const ret = compiler.compile('/main.typ', undefined, kind, 3);
		const out = ret instanceof Uint8Array ? ret : (ret && (ret.result || ret.artifact));
		if (!(out instanceof Uint8Array)) {
			return { error: JSON.stringify(ret && ret.diagnostics ? ret.diagnostics : ret).slice(0, 400) };
		}
		return { bytes: out };
	};
	return {
		memory: exports.memory,
		compile(src) {
			const o = run(src, 'pdf');
			return o.error ? o : { pdf: o.bytes };
		},
		// No delta door: the live view is a full `vector` compile on each edit.
		open(src) {
			const o = run(src, 'vector');
			return o.error ? o : { pages: null, sent: o.bytes.length };
		},
		edit(src) {
			return this.open(src);
		},
	};
}

async function loadAustenite(dir) {
	const js = readdirSync(dir).find((n) => /^oxedyne_fe2o3_austenite\.js$/.test(n));
	const wasm = readdirSync(dir).find((n) => /_bg\.wasm$/.test(n));
	if (!js || !wasm) throw new Error('no wasm-pack output in ' + dir);
	const mod = await import(pathToFileURL(join(dir, js)).href);
	const exports = await mod.default({ module_or_path: readFileSync(join(dir, wasm)) });
	const aus = new mod.DaimondTypst();
	const project = (src, extra) => ({ main: '/main.typ', sources: [['/main.typ', src]], assets: [], fonts: [], ...extra });
	// The consumer's page cache, id -> SVG, as Daimond keeps it: new pages in, absent ids out.
	const cache = new Map();
	let known = [];
	const delta = (src) => {
		const ret = aus.compileProjectDelta(project(src, { known }));
		if (!ret || ret.error || !Array.isArray(ret.order)) {
			return { error: String(ret && ret.error ? ret.error : 'no delta').slice(0, 400) };
		}
		let sent = 0;
		for (const c of ret.changed) {
			cache.set(c.id, c.svg);
			sent += c.svg.length;
		}
		const live = new Set(ret.order);
		for (const id of [...cache.keys()]) if (!live.has(id)) cache.delete(id);
		known = ret.order;
		return { pages: ret.order.length, sent };
	};
	return {
		memory: exports.memory,
		compile(src) {
			const ret = aus.compileProject(project(src));
			if (ret && ret.pdf) return { pdf: ret.pdf };
			return { error: String(ret && ret.error ? ret.error : 'no pdf').slice(0, 400) };
		},
		open: delta,
		edit: delta,
	};
}

function pkgOf(engine) {
	if (engine === 'ref') return opt('--ref-pkg', DEFAULT_AUST);
	const dir = opt('--austenite-pkg', null);
	if (!dir) throw new Error('the austenite engine needs --austenite-pkg DIR, the wasm-pack output to measure');
	return dir;
}

async function child(engine, pages) {
	const base = status();
	const src = corpus(pages, !flag('--plain'));
	const mode = opt('--mode', 'pdf');
	const t0 = performance.now();
	const eng = engine === 'typst'
		? await loadTypst(resolve(opt('--typst-vendor', DEFAULT_TYPST)))
		: await loadAustenite(resolve(pkgOf(engine)));
	const t1 = performance.now();
	const heapBefore = eng.memory.buffer.byteLength;
	const row = {
		engine, pages, mode, sourceBytes: Buffer.byteLength(src),
		initMs: Math.round(t1 - t0), heapBefore, rssBaselineKb: base.rss, error: null,
	};
	// A throw out of the wasm (an allocation past the 4 GiB a wasm32 memory can hold, a panic) is a result
	// of the cell, not of the probe: the row records it with the heap the instance had reached.
	if (mode === 'pdf') {
		try {
			const out = eng.compile(src);
			row.compileMs = Math.round(performance.now() - t1);
			row.heapPeak = eng.memory.buffer.byteLength;
			row.pagesOut = out.pdf ? pdfPages(out.pdf) : 0;
			row.error = out.error || null;
		} catch (e) {
			row.error = 'threw ' + String(e).slice(0, 300);
			row.heapPeak = eng.memory.buffer.byteLength;
			row.pagesOut = 0;
		}
	} else {
		const n = +opt('--edits', '5');
		const steps = [src, ...edited(src, n)];
		row.heaps = [];
		row.sent = [];
		row.ms = [];
		for (const s of steps) {
			const t = performance.now();
			let out;
			try {
				out = row.heaps.length === 0 ? eng.open(s) : eng.edit(s);
			} catch (e) {
				out = { error: 'threw ' + String(e).slice(0, 300) };
			}
			row.ms.push(Math.round(performance.now() - t));
			if (out.error) {
				row.error = (row.heaps.length === 0 ? 'open: ' : 'edit ' + row.heaps.length + ': ') + out.error;
				row.heapAtError = eng.memory.buffer.byteLength;
				break;
			}
			if (row.pagesOut === undefined && out.pages) row.pagesOut = out.pages;
			row.heaps.push(eng.memory.buffer.byteLength);
			row.sent.push(out.sent);
			// What the parent keeps if this process is killed before it finishes.
			process.stdout.write('@@STEP ' + JSON.stringify({ heaps: row.heaps, sent: row.sent, ms: row.ms, pagesOut: row.pagesOut ?? null, rssPeakKb: status().hwm }) + '\n');
		}
		row.heapPeak = eng.memory.buffer.byteLength;
		row.heapOpen = row.heaps[0] ?? null;
		row.pagesOut ??= null;
	}
	row.rssPeakKb = status().hwm;
	process.stdout.write('@@ROW ' + JSON.stringify(row) + '\n');
}

// Parent

const mb = (b) => (b / 1048576).toFixed(1);

function runChild(engine, pages) {
	const args = [fileURLToPath(import.meta.url), '--child', engine, String(pages)];
	for (const k of ['--typst-vendor', '--austenite-pkg', '--ref-pkg', '--mode', '--edits']) {
		const v = opt(k, null);
		if (v) args.push(k, v);
	}
	if (flag('--plain')) args.push('--plain');
	const res = spawnSync(process.execPath, args, {
		encoding: 'utf8', timeout: +opt('--timeout', '600') * 1000, maxBuffer: 1 << 26,
	});
	const lines = (res.stdout || '').split('\n');
	const line = lines.find((l) => l.startsWith('@@ROW '));
	if (line) return JSON.parse(line.slice(6));
	const why = res.error ? String(res.error.message)
		: res.signal ? 'killed by ' + res.signal : 'exit ' + res.status;
	const row = { engine, pages, mode: opt('--mode', 'pdf'), error: why + ': ' + (res.stderr || '').trim().split('\n').slice(-3).join(' | ') };
	// A killed delta run still says how far it got: the heap after each step it finished.
	const step = lines.filter((l) => l.startsWith('@@STEP ')).pop();
	if (step) {
		const st = JSON.parse(step.slice(7));
		Object.assign(row, st, { heapPeak: st.heaps[st.heaps.length - 1], heapOpen: st.heaps[0], finished: st.heaps.length });
	}
	return row;
}

function median(xs) {
	const v = xs.filter((x) => typeof x === 'number').sort((a, b) => a - b);
	if (!v.length) return null;
	const m = v.length >> 1;
	return v.length % 2 ? v[m] : (v[m - 1] + v[m]) / 2;
}

// The median of each measure over the runs of one (engine, pages) cell. A cell with an erroring run is
// left out: a median over the survivors would flatter the engine that fell over.
export function cells(rows) {
	const by = new Map();
	for (const r of rows) {
		const k = r.engine + '/' + r.pages;
		if (!by.has(k)) by.set(k, []);
		by.get(k).push(r);
	}
	const out = [];
	for (const [, rs] of by) {
		const bad = rs.filter((r) => r.error);
		const c = {
			engine: rs[0].engine, pages: rs[0].pages, runs: rs.length, errors: bad.length,
			heapPeak: null, rssPeakKb: null, pagesOut: null,
		};
		if (!bad.length) {
			c.heapPeak = median(rs.map((r) => r.heapPeak));
			c.rssPeakKb = median(rs.map((r) => r.rssPeakKb));
			c.pagesOut = median(rs.map((r) => r.pagesOut));
			c.heapOpen = median(rs.map((r) => r.heapOpen));
		} else {
			c.error = bad[0].error;
		}
		out.push(c);
	}
	return out;
}

// The memory gate (D-20260923-34), from the per-cell medians, in MiB:
//   R = H_A(300) / H_T(300), at most R_GATE, with R_AIM the aim;
//   G = (H_A(1000) - H_A(300)) / (P_A(1000) - P_A(300)), at most G_GATE MiB for each page Austenite
//       produced between the two sizes (P is the page count read back from its own PDF).
// typst.ts's own G is computed the same way and printed beside it. A missing or failed cell is a
// verdict of `incomplete`, which is not a pass.
export function judge(cs, a = 'austenite', t = 'typst') {
	const at = (e, p) => cs.find((c) => c.engine === e && c.pages === p && c.heapPeak !== null);
	const a300 = at(a, 300);
	const a1000 = at(a, 1000);
	const t300 = at(t, 300);
	const t1000 = at(t, 1000);
	const need = [[a, 300, a300], [a, 1000, a1000], [t, 300, t300]].filter((x) => !x[2]).map((x) => x[0] + '/' + x[1]);
	if (need.length) return { verdict: 'incomplete', missing: need };
	const R = a300.heapPeak / t300.heapPeak;
	const dp = a1000.pagesOut - a300.pagesOut;
	const G = dp > 0 ? (a1000.heapPeak - a300.heapPeak) / 1048576 / dp : Infinity;
	const dpt = t1000 && t300 ? t1000.pagesOut - t300.pagesOut : 0;
	const Gt = t1000 && dpt > 0 ? (t1000.heapPeak - t300.heapPeak) / 1048576 / dpt : null;
	const rOk = R <= R_GATE;
	const gOk = G <= G_GATE;
	return {
		verdict: rOk && gOk ? 'pass' : 'fail',
		R, R_gate: R_GATE, R_aim: R_AIM, R_ok: rOk, R_aim_met: R <= R_AIM,
		G, G_gate: G_GATE, G_ok: gOk, G_typst: Gt,
		pages_a300: a300.pagesOut, pages_a1000: a1000.pagesOut,
	};
}

function printJudgement(j) {
	if (j.verdict === 'incomplete') {
		console.log('gate INCOMPLETE: no measurement for ' + j.missing.join(', '));
		return;
	}
	const f = (x, d) => (x === null || x === undefined ? '-' : Number.isFinite(x) ? x.toFixed(d) : String(x));
	console.log('R = H_A(300)/H_T(300) = ' + f(j.R, 3) + '  gate <= ' + j.R_gate + ' -> ' + (j.R_ok ? 'PASS' : 'FAIL')
		+ ' (margin ' + f(j.R_gate - j.R, 3) + ')');
	console.log('aim R <= ' + j.R_aim + ' -> ' + (j.R_aim_met ? 'met' : 'not met (' + f(j.R - j.R_aim, 3) + ' over)'));
	console.log('G = growth 300 -> 1000 = ' + f(j.G, 3) + ' MiB a page over ' + (j.pages_a1000 - j.pages_a300)
		+ ' pages  gate <= ' + j.G_gate + ' -> ' + (j.G_ok ? 'PASS' : 'FAIL') + ' (margin ' + f(j.G_gate - j.G, 3)
		+ ')  typst.ts G = ' + f(j.G_typst, 3));
	console.log('memory gate: ' + j.verdict.toUpperCase());
}

function table(rows) {
	console.log('engine     mode   pages  out  src KB  init ms    work ms (open or compile, then edits)  heap MB  open MB  rss base MB  rss peak MB  note');
	for (const r of rows) {
		console.log(
			r.engine.padEnd(10) + String(r.mode ?? 'pdf').padEnd(6) + String(r.pages).padStart(6) + String(r.pagesOut ?? '-').padStart(5)
			+ String(r.sourceBytes ? Math.round(r.sourceBytes / 1024) : '-').padStart(8)
			+ String(r.initMs ?? '-').padStart(9)
			+ '  ' + String(r.compileMs ?? (r.ms ? r.ms.join('/') : '-')).padEnd(34)
			+ (r.heapPeak ? mb(r.heapPeak) : '-').padStart(9)
			+ (r.heapOpen ? mb(r.heapOpen) : '-').padStart(9)
			+ (r.rssBaselineKb ? mb(r.rssBaselineKb * 1024) : '-').padStart(13)
			+ (r.rssPeakKb ? mb(r.rssPeakKb * 1024) : '-').padStart(13)
			+ '  ' + (r.error ? 'ERROR ' + r.error : ''));
	}
}

async function main() {
	if (flag('--child')) {
		const i = process.argv.indexOf('--child');
		return child(process.argv[i + 1], +process.argv[i + 2]);
	}
	if (opt('--gate-file', null)) {
		const rows = opt('--gate-file', '').split(',').flatMap((f) => JSON.parse(readFileSync(f, 'utf8')).rows);
		const j = judge(cells(rows));
		printJudgement(j);
		process.exit(j.verdict === 'pass' ? 0 : j.verdict === 'fail' ? 1 : 2);
	}
	if (opt('--dump', null)) {
		process.stdout.write(corpus(+opt('--dump', '1'), !flag('--plain')));
		return;
	}
	const pages = opt('--pages', '50,300,1000').split(',').map(Number);
	const engines = opt('--engines', 'typst,austenite').split(',');
	const runs = +opt('--runs', '1');
	const rows = [];
	for (let run = 0; run < runs; run++) {
		const order = run % 2 ? [...engines].reverse() : engines;
		for (const p of pages) {
			for (const e of order) {
				const r = runChild(e, p);
				r.run = run;
				rows.push(r);
				process.stderr.write(`run ${run} ${e} ${p}: heap ${r.heapPeak ? mb(r.heapPeak) : '-'} MB${r.error ? ' ERROR' : ''}\n`);
			}
		}
	}
	table(rows);
	const cs = cells(rows);
	console.log('\nmedians over ' + runs + ' run(s), MiB:');
	for (const c of cs) {
		console.log(c.engine.padEnd(10) + String(c.pages).padStart(6) + ' pages out ' + String(c.pagesOut ?? '-').padStart(5)
			+ '  heap ' + (c.heapPeak !== null ? mb(c.heapPeak) : '-').padStart(8)
			+ '  VmHWM ' + (c.rssPeakKb !== null ? mb(c.rssPeakKb * 1024) : '-').padStart(8)
			+ (c.errors ? '  ' + c.errors + ' erroring run(s): ' + c.error : ''));
	}
	const j = judge(cs);
	printJudgement(j);
	if (engines.includes('ref')) {
		const k = judge(cs, 'ref');
		if (k.verdict !== 'incomplete') console.log('reference row (Daimond\'s vendored Austenite): R = ' + k.R.toFixed(3) + ', G = ' + k.G.toFixed(3) + ' MiB a page');
	}
	const json = opt('--json', null);
	if (json) writeFileSync(json, JSON.stringify({ rows, cells: cs, gate: j }, null, 1));
	if (flag('--gate') && j.verdict !== 'pass') process.exit(j.verdict === 'fail' ? 1 : 2);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) await main();
