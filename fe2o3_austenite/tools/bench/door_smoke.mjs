#!/usr/bin/env node
// door_smoke.mjs -- the wasm package's doors, driven with Daimond's project shape in node.
//
// The package is a wasm-pack `--target web` output. Each case is a strict project, `{ main, sources, assets,
// fonts, strict: true }`, compiled through `compileProject`, and printed as one line:
//
//   <case> ok|error <kind> <file>:<line>:<col>
//
// `ok` carries `-` and `-:0:0`; `error` carries the first diagnostic, which is the error's own site, its kind
// and its 1-based line and UTF-16 column. The native test `tests/eval_door.rs` prints the same lines through
// the same door, and both are held to `tests/fixtures/door_smoke_lines.txt`, so the browser's results are the
// native ones. After the case lines come `interop <name> ok` lines for the surface the cases do not reach: the
// PDF copy-out, the other doors, the package calls, `needs`, `queryProject` and `engineInfo`.
//
//   node tools/bench/door_smoke.mjs --pkg DIR [--expect FILE] [--git SHA] [--delta]
//
// Run it under a memory cap, as every heavy job on this host is:
//   systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice node tools/bench/door_smoke.mjs --pkg DIR
//
// Options:
//   --pkg DIR      the wasm-pack output directory (required)
//   --expect FILE  fail unless the case lines equal the lines in FILE
//   --git SHA      fail unless `engineInfo().git` is exactly SHA (a build from a committed tree)
//   --delta        also drive the live view's changed-only delta, `compileProjectDelta`, over six pages: the
//                  `order` is as long as `pages`, every changed SVG holds the `.tsel` text layer, an unchanged
//                  second call sends no SVG, a one-letter edit sends one page, a refusal leaves `version`
//                  where it was, and a page that differs only in its face has another id

import { readFileSync, readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

function opt(name, dflt) {
	const i = process.argv.indexOf(name);
	return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : dflt;
}

const dir = opt('--pkg', null);
if (!dir) {
	console.error('usage: door_smoke.mjs --pkg DIR [--expect FILE] [--git SHA]');
	process.exit(2);
}

// The cases, identical to the native test's.
const SKELETON = '#set page(width: 200pt, height: 100pt)\n= Skeleton\nA short paragraph.\n';
const CASES = [
	['skeleton',		SKELETON],
	['missing_font',	'#set text(font: "Nope Sans")\nHello.\n'],
	['missing_import',	'Intro.\n#import "gone.typ": thing\n'],
	['missing_image',	'Text.\n\n#image("gone.png")\n'],
	['empty_main',		''],
];

const project = (text, extra = {}) => ({
	main: '/main.typ', sources: [['/main.typ', text]], assets: [], fonts: [], strict: true, ...extra,
});

function line(name, r) {
	if (r && r.pdf) return `${name} ok - -:0:0`;
	const d = r && r.diagnostics && r.diagnostics[0];
	if (!d) return `${name} error - ${String(r && r.error)}`;
	return `${name} error ${d.kind} ${d.file}:${d.line}:${d.col}`;
}

const failures = [];
function check(name, ok, why) {
	if (ok) {
		console.log(`interop ${name} ok`);
	} else {
		console.log(`interop ${name} FAILED ${why || ''}`);
		failures.push(name);
	}
}

const js = readdirSync(dir).find((n) => /^oxedyne_fe2o3_austenite\.js$/.test(n));
const wasm = readdirSync(dir).find((n) => /_bg\.wasm$/.test(n));
if (!js || !wasm) {
	console.error('no wasm-pack output in ' + dir);
	process.exit(2);
}
const mod = await import(pathToFileURL(join(resolve(dir), js)).href);
await mod.default({ module_or_path: readFileSync(join(resolve(dir), wasm)) });
const aus = new mod.DaimondTypst();

const lines = [];
for (const [name, text] of CASES) {
	const l = line(name, aus.compileProject(project(text)));
	lines.push(l);
	console.log(l);
}

// The PDF is copied out whole, at its own length.
const made = aus.compileProject(project(SKELETON));
check('pdf copy-out',
	made.pdf instanceof Uint8Array && made.pdf.length > 500
		&& Buffer.from(made.pdf.subarray(0, 5)).toString('latin1') === '%PDF-'
		&& Buffer.from(made.pdf.subarray(made.pdf.length - 6)).toString('latin1').includes('%%EOF'),
	`length ${made.pdf && made.pdf.length}`);
check('result shape', made.pages === 1 && Array.isArray(made.diagnostics) && made.diagnostics.length === 0
	&& made.skipped === null && Array.isArray(made.needs) && made.needs.length === 0,
	JSON.stringify({ pages: made.pages, skipped: made.skipped, needs: made.needs }));

// The error shape: the head, then the sites; the head's own line is `error`.
const bad = aus.compileProject(project('#set text(font: "Nope Sans")\nx\n'));
check('error shape', typeof bad.error === 'string' && bad.error.startsWith('/main.typ:1:17: ')
	&& bad.diagnostics[0].severity === 'error' && bad.diagnostics[0].hint === 'missing_font ×1'
	&& Array.isArray(bad.needs), String(bad.error));

// queryProject after a good compile: the rail's rows.
aus.compileProject(project('#set page(height: 60pt, margin: 5pt)\n= One\nx\n#pagebreak()\n== Two\ny\n'));
const rows = aus.queryProject('heading', '');
check('queryProject heading rows', Array.isArray(rows) && rows.length === 2
	&& rows[0].title === 'One' && rows[0].page === 1 && rows[0].level === 1 && rows[0].kind === 'heading'
	&& rows[1].title === 'Two' && rows[1].page === 2 && rows[1].level === 2,
	JSON.stringify(rows));
check('queryProject null after a failed compile', (aus.compileProject(project('#import "gone.typ"\n')), aus.queryProject('heading', '') === null));
check('queryProject null for a selector that does not evaluate', (aus.compileProject(project('= A\n')), aus.queryProject('nonesuch(', '') === null));

// The other doors.
const svg = aus.compileProjectVector(project(SKELETON));
check('vector', Array.isArray(svg.svg) && svg.svg.length === 1 && svg.svg[0].includes('<svg')
	&& svg.svg[0].includes('class="tsel"') && svg.pages === 1);
const delta = aus.compileProjectDelta(project('= Delta\nbody\n', { known: [] }));
check('delta', Array.isArray(delta.order) && delta.order.length === 1 && delta.reset === true
	&& delta.changed.length === 1 && delta.version >= 1 && Array.isArray(delta.needs), JSON.stringify(Object.keys(delta)));
const single = aus.compile('Hello, single source.\n');
check('compile(source)', single.pdf instanceof Uint8Array && single.pages === 1);

// Fonts.
const fams = aus.fontFamilies();
check('fontFamilies', Array.isArray(fams) && fams.includes('Libertinus Serif') && fams.includes('New Computer Modern Math')
	&& fams.join('|') === [...fams].sort((a, b) => a.toLowerCase() < b.toLowerCase() ? -1 : 1).join('|'), fams.join(','));

// Packages: supplied, imported, withdrawn, and `needs` names what an import still wants.
const spec = '@local/smoke:0.1.0';
const files = [
	['typst.toml', '[package]\nname = "smoke"\nversion = "0.1.0"\nentrypoint = "lib.typ"\n'],
	['lib.typ', new TextEncoder().encode('#let hi(n) = [Hi, #n.]\n')],
];
const sup = aus.supplyPackage(spec, files);
check('supplyPackage', sup.files === 2 && sup.spec === spec && aus.packages().includes(spec), JSON.stringify(sup));
const used = aus.compileProject(project('#import "@local/smoke:0.1.0": hi\n#hi("node")\n'));
check('package import', !!used.pdf && used.needs.length === 0, String(used.error));
check('withdrawPackage', aus.withdrawPackage(spec).withdrawn === true && !aus.packages().includes(spec));
const gone = aus.compileProject(project('#import "@local/smoke:0.1.0": hi\n#hi("node")\n'));
check('needs', !!gone.error && gone.needs.length === 1 && gone.needs[0] === spec
	&& gone.diagnostics[0].kind === 'package', JSON.stringify(gone.needs));

// Identity.
const info = aus.engineInfo();
check('engineInfo', info.engine === 'austenite' && info.typst === '0.15.1' && typeof info.version === 'string',
	JSON.stringify(info));
const want = opt('--git', null);
if (want !== null) {
	check('engineInfo git', info.git === want && !info.git.endsWith('-dirty'), `${info.git} is not ${want}`);
}

// The case lines against the native ones.
const file = opt('--expect', null);
if (file !== null) {
	const expected = readFileSync(file, 'utf8').split('\n').filter((l) => l.length > 0);
	check('case lines equal the native lines', JSON.stringify(expected) === JSON.stringify(lines),
		`expected ${JSON.stringify(expected)}`);
}

// The live view's delta (G10, the engine's side).
if (process.argv.includes('--delta')) {
	const NAMES = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot'];
	const six = (edit) => '#set page(width: 220pt, height: 100pt, margin: 12pt)\n' + NAMES.map((n, k) =>
		(k > 0 ? '#pagebreak()\n' : '') + `Page ${n} holds the ${edit === k ? 'quarts' : 'quartz'} of paragraph ${k} alone.\n`).join('');
	const hasTsel = (svg) => svg.includes('.tsel { fill: transparent; }') && svg.includes('<text class="tsel">');
	const key = (r) => JSON.stringify(r.order);
	// The characters of the selectable layer, spaces dropped.
	const layerText = (svg) => [...svg.matchAll(/<tspan[^>]*>([^<]*)<\/tspan>/g)].map((m) => m[1]).join('').replace(/\s+/g, '');

	const first = aus.compileProjectDelta(project(six(null), { known: [] }));
	first.order = first.order || [];
	first.changed = first.changed || [];
	check('delta order length equals pages', Array.isArray(first.order) && first.pages === 6
		&& first.order.length === first.pages && first.reset === true && first.version >= 1,
		JSON.stringify({ pages: first.pages, order: first.order && first.order.length, error: first.error }));
	check('delta ids are opaque decimal strings', first.order.every((id) => typeof id === 'string' && /^[0-9]+$/.test(id)));
	check('delta every changed svg holds the .tsel layer', first.changed.length === new Set(first.order).size
		&& first.changed.every((c) => first.order.includes(c.id) && hasTsel(c.svg)),
		`${first.changed.length} changed`);

	const second = aus.compileProjectDelta(project(six(null), { known: first.order }));
	second.order = second.order || [];
	second.changed = second.changed || [];
	check('delta an unchanged second call sends no svg', second.changed.length === 0 && second.reset === false
		&& key(second) === key(first) && second.version === first.version + 1,
		JSON.stringify({ changed: second.changed.length, reset: second.reset, version: second.version }));

	const edited = aus.compileProjectDelta(project(six(2), { known: first.order }));
	edited.order = edited.order || [];
	edited.changed = edited.changed || [];
	check('delta a one-letter edit sends the one page', edited.order.length === 6 && edited.changed.length === 1
		&& edited.changed[0].id === edited.order[2] && edited.order[2] !== first.order[2]
		&& edited.order.every((id, i) => i === 2 || id === first.order[i])
		&& hasTsel(edited.changed[0].svg) && layerText(edited.changed[0].svg).includes('quarts'),
		JSON.stringify({ changed: edited.changed.length }));

	const refusals = [
		['a missing font', '#set text(font: "Nope Sans")\nx\n', 'missing_font'],
		['a body that sets nothing', '#context none\n', 'internal'],
		['an unknown name', '#nonesuch()\n', 'unknown_variable'],
	];
	for (const [what, text, kind] of refusals) {
		const r = aus.compileProjectDelta(project(text, { known: first.order }));
		check(`delta ${what} is refused`, typeof r.error === 'string' && ((r.diagnostics || [])[0] || {}).kind === kind
			&& r.version === undefined && Array.isArray(r.needs), String(r.error));
	}
	const blank = aus.compileProjectDelta(project('#context []\n', { known: [] }));
	const at = (blank.diagnostics || [])[0] || {};
	check('delta a context that gives nothing is refused at 1:1', blank.error !== undefined && at.file === '/main.typ'
		&& at.line === 1 && at.col === 1, JSON.stringify(blank.diagnostics && blank.diagnostics[0]));
	const after = aus.compileProjectDelta(project(six(null), { known: first.order }));
	check('delta the refusals left the version where it was', after.version === edited.version + 1,
		`${after.version} after ${edited.version}`);

	const raw = (wrap) => `#set page(width: 220pt, height: 60pt, margin: 8pt)\n${wrap('raw("hello world")')}\n`;
	const regular = aus.compileProjectDelta(project(raw((r) => '#' + r), { known: [] }));
	regular.order = regular.order || [];
	regular.changed = regular.changed || [];
	const bold = aus.compileProjectDelta(project(raw((r) => `#strong(${r})`), { known: regular.order }));
	bold.order = bold.order || [];
	bold.changed = bold.changed || [];
	check('delta a page that differs only in its face has another id', bold.order[0] !== regular.order[0]
		&& bold.changed.length === 1 && bold.changed[0].svg !== regular.changed[0].svg);
}

if (failures.length > 0) {
	console.error(`door_smoke: ${failures.length} failed: ${failures.join(', ')}`);
	process.exit(1);
}
