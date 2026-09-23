#!/usr/bin/env node
// heap_probe.mjs -- peak memory and wall time of a compile, Austenite against typst.ts, in node.
//
// Each (engine, page count) runs in a fresh child process, so one compile's heap cannot flatter or
// burden the next: a wasm32 linear memory only ever grows, which makes its `byteLength` after the
// compile the high-water mark of that compile, and the child's `VmHWM` is the kernel's own peak
// resident set for the whole process (node itself included; the baseline before the engine loads
// is reported beside it so the difference can be read).
//
// The corpus is synthetic and deterministic: headings, paragraphs, lists, a small table and an
// equation per section, sized by a words-per-page estimate. The page count each engine actually
// produced is read back from its PDF and printed, so a corpus that came out shorter or longer than
// asked is visible rather than assumed.
//
//   node tools/heap_probe.mjs                                   # 50, 300, 1000 pages, both engines
//   node tools/heap_probe.mjs --pages 50,300 --engines austenite --austenite-pkg pkg --eval
//   node tools/heap_probe.mjs --gate --json heap.json
//
// Run it under a memory cap, as every heavy job on this host is:
//   systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice node tools/heap_probe.mjs
//
// Options:
//   --pages a,b,c        page counts to generate (default 50,300,1000)
//   --engines x,y        `typst` (typst.ts web compiler) and/or `austenite` (default both)
//   --typst-vendor DIR   directory holding typst_ts_web_compiler{.mjs,_bg.wasm} and fonts/
//   --austenite-pkg DIR  a wasm-pack `--target web` output directory for fe2o3_austenite
//   --eval               ask Austenite for the evaluator path (`engine: "eval"` in the project)
//   --plain              prose and headings only, for an engine that refuses richer constructs
//   --timeout SEC        per child (default 600)
//   --json FILE          also write every measurement as JSON
//   --gate               exit non-zero unless the pass line holds (owner decision 1): Austenite's
//                        peak heap at 300 pages is at most half of typst.ts's
//   --dump PAGES         print the generated source for PAGES and exit

import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HOME = process.env.HOME || '';
const DEFAULT_TYPST = join(HOME, 'usr/code/web/apps/oxedyne/daimond/www/vendor/typst');
const DEFAULT_AUST = join(HOME, 'usr/code/web/apps/oxedyne/daimond/www/vendor/austenite');

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

// About 320 words to an A4 page of 11pt Libertinus with Typst's default margins, less the space the
// headings and blocks take; the read-back page count is what is reported, not this estimate.
const WORDS_PER_PAGE = 320;

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
	return {
		memory: exports.memory,
		compile(src) {
			compiler.reset_shadow();
			compiler.add_source('/main.typ', src);
			const ret = compiler.compile('/main.typ', undefined, 'pdf', 3);
			const pdf = ret instanceof Uint8Array ? ret : (ret && (ret.result || ret.artifact));
			if (!(pdf instanceof Uint8Array)) {
				return { error: JSON.stringify(ret && ret.diagnostics ? ret.diagnostics : ret).slice(0, 400) };
			}
			return { pdf };
		},
	};
}

async function loadAustenite(dir, useEval) {
	const js = readdirSync(dir).find((n) => /^oxedyne_fe2o3_austenite\.js$/.test(n));
	const wasm = readdirSync(dir).find((n) => /_bg\.wasm$/.test(n));
	if (!js || !wasm) throw new Error('no wasm-pack output in ' + dir);
	const mod = await import(pathToFileURL(join(dir, js)).href);
	const exports = await mod.default({ module_or_path: readFileSync(join(dir, wasm)) });
	const aus = new mod.DaimondTypst();
	return {
		memory: exports.memory,
		compile(src) {
			const project = { main: '/main.typ', sources: [['/main.typ', src]], assets: [], fonts: [] };
			if (useEval) project.engine = 'eval';
			const ret = aus.compileProject(project);
			if (ret && ret.pdf) return { pdf: ret.pdf };
			return { error: String(ret && ret.error ? ret.error : 'no pdf').slice(0, 400) };
		},
	};
}

async function child(engine, pages) {
	const base = status();
	const src = corpus(pages, !flag('--plain'));
	const t0 = performance.now();
	const eng = engine === 'typst'
		? await loadTypst(resolve(opt('--typst-vendor', DEFAULT_TYPST)))
		: await loadAustenite(resolve(opt('--austenite-pkg', DEFAULT_AUST)), flag('--eval'));
	const t1 = performance.now();
	const heapBefore = eng.memory.buffer.byteLength;
	const out = eng.compile(src);
	const t2 = performance.now();
	const st = status();
	const row = {
		engine, pages, sourceBytes: Buffer.byteLength(src),
		initMs: Math.round(t1 - t0), compileMs: Math.round(t2 - t1),
		heapBefore, heapPeak: eng.memory.buffer.byteLength,
		rssBaselineKb: base.rss, rssPeakKb: st.hwm,
		pagesOut: out.pdf ? pdfPages(out.pdf) : 0,
		error: out.error || null,
	};
	process.stdout.write('@@ROW ' + JSON.stringify(row) + '\n');
}

// Parent

const mb = (b) => (b / 1048576).toFixed(1);

function runChild(engine, pages) {
	const args = [fileURLToPath(import.meta.url), '--child', engine, String(pages)];
	for (const k of ['--typst-vendor', '--austenite-pkg']) {
		const v = opt(k, null);
		if (v) args.push(k, v);
	}
	for (const k of ['--eval', '--plain']) if (flag(k)) args.push(k);
	const res = spawnSync(process.execPath, args, {
		encoding: 'utf8', timeout: +opt('--timeout', '600') * 1000, maxBuffer: 1 << 26,
	});
	const line = (res.stdout || '').split('\n').find((l) => l.startsWith('@@ROW '));
	if (line) return JSON.parse(line.slice(6));
	const why = res.error ? String(res.error.message)
		: res.signal ? 'killed by ' + res.signal : 'exit ' + res.status;
	return { engine, pages, error: why + ': ' + (res.stderr || '').trim().split('\n').slice(-3).join(' | ') };
}

async function main() {
	if (flag('--child')) {
		const i = process.argv.indexOf('--child');
		return child(process.argv[i + 1], +process.argv[i + 2]);
	}
	if (opt('--dump', null)) {
		process.stdout.write(corpus(+opt('--dump', '1'), !flag('--plain')));
		return;
	}
	const pages = opt('--pages', '50,300,1000').split(',').map(Number);
	const engines = opt('--engines', 'typst,austenite').split(',');
	const rows = [];
	console.log('engine     pages  out  src KB  init ms  compile ms  heap MB  rss base MB  rss peak MB  note');
	for (const p of pages) {
		for (const e of engines) {
			const r = runChild(e, p);
			rows.push(r);
			console.log(
				e.padEnd(10) + String(p).padStart(6) + String(r.pagesOut ?? '-').padStart(5)
				+ String(r.sourceBytes ? Math.round(r.sourceBytes / 1024) : '-').padStart(8)
				+ String(r.initMs ?? '-').padStart(9) + String(r.compileMs ?? '-').padStart(12)
				+ (r.heapPeak ? mb(r.heapPeak) : '-').padStart(9)
				+ (r.rssBaselineKb ? mb(r.rssBaselineKb * 1024) : '-').padStart(13)
				+ (r.rssPeakKb ? mb(r.rssPeakKb * 1024) : '-').padStart(13)
				+ '  ' + (r.error ? 'ERROR ' + r.error : ''));
		}
	}
	const at = (e, p) => rows.find((r) => r.engine === e && r.pages === p && !r.error);
	const a = at('austenite', 300);
	const t = at('typst', 300);
	let pass = null;
	if (a && t) {
		pass = a.heapPeak <= t.heapPeak / 2;
		console.log('pass line (300 pages, Austenite heap <= half typst.ts): ' + mb(a.heapPeak) + ' MB vs '
			+ mb(t.heapPeak) + ' MB -> ' + (pass ? 'PASS' : 'FAIL'));
	} else {
		console.log('pass line not evaluated: both engines must compile the 300-page corpus');
	}
	const json = opt('--json', null);
	if (json) writeFileSync(json, JSON.stringify({ rows, pass }, null, 1));
	if (flag('--gate') && pass !== true) process.exit(1);
}

await main();
