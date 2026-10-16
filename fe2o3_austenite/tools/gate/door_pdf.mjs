#!/usr/bin/env node
// The door half of the gate: gathers a project as Daimond does, supplies its packages, and compiles it to a PDF
// through the wasm package's `compileProject` with `strict: true`, as the browser does. It prints a closed
// vocabulary and nothing else: a diagnostic's message can quote the source, so none is ever printed.
//
//     node door_pdf.mjs <pkg> <root> <main> <fontdir> <out> [--packages <dir>] [--exclude <dir>]... [--again <out2>]
//
//   <pkg>        the wasm-pack output directory (`oxedyne_fe2o3_austenite.js` and its `_bg.wasm`)
//   <root>       the project root
//   <main>       the entry file, relative to <root>
//   <fontdir>    the project's font directory, relative to <root> or absolute
//   <out>        where the PDF is written, when the compile succeeds
//   --packages   a directory of packages, as `tools/bench/lib/packages.mjs` describes (Typst's own layout or
//                Daimond's `.pack` files); each package the project's imports reach is handed over with
//                `supplyPackage`, with its dependencies
//   --exclude    a directory under <root> the gather leaves out; repeatable
//   --again      compile a second time on the same instance and write that PDF to <out2>, for G12
//
// Prints one of
//
//   door <ok|error> pages <n|none> kinds <none|kind:n,kind:n,...> needs <n>
//   door again <ok|error>                                          (only with --again)
//   door fail <bad-args|bad-pkg|no-root|no-main|no-packages|bad-package|too-large|supply|failed>
//
// `kinds` counts the diagnostics of the first compile by their `kind`, from the engine's closed list (a word
// outside it prints as `other`); `needs` is how many packages an import asked for that were not supplied.

import fs from 'node:fs';
import path from 'node:path';

import { gather } from '../bench/lib/gather.mjs';
import { specText } from '../bench/lib/packages.mjs';
import { loadAustenite } from '../bench/lib/wasm_common.mjs';

const KINDS = ['missing_file', 'encoding', 'missing_font', 'syntax', 'type', 'unknown_variable', 'package', 'limit', 'unsupported', 'lint', 'internal'];

function args(argv) {
	const pos = [];
	const a = { exclude: [], packages: null, again: null };
	for (let i = 0; i < argv.length; i++) {
		const k = argv[i];
		if (k === '--packages' || k === '--exclude' || k === '--again') {
			if (i + 1 >= argv.length) return null;
			const v = argv[++i];
			if (k === '--exclude') a.exclude.push(v);
			else if (k === '--packages') a.packages = v;
			else a.again = v;
		} else if (k.startsWith('--')) {
			return null;
		} else {
			pos.push(k);
		}
	}
	if (pos.length !== 5) return null;
	[a.pkg, a.root, a.main, a.fontdir, a.out] = pos;
	return a;
}

function kinds(r) {
	const n = {};
	for (const d of (r && r.diagnostics) || []) {
		const k = KINDS.includes(d && d.kind) ? d.kind : 'other';
		n[k] = (n[k] || 0) + 1;
	}
	const keys = Object.keys(n).sort();
	return keys.length ? keys.map((k) => `${k}:${n[k]}`).join(',') : 'none';
}

const a = args(process.argv.slice(2));
if (!a) {
	console.log('door fail bad-args');
	process.exit(2);
}

let stage = 'failed';
try {
	if (!fs.existsSync(path.join(a.pkg, 'oxedyne_fe2o3_austenite.js'))) {
		stage = 'bad-pkg';
		throw new Error('no package');
	}
	const g = gather(a.root, a.main, { fontDirs: [a.fontdir], exclude: a.exclude, packages: a.packages });
	const aus = await loadAustenite(a.pkg);
	const door = aus.instance;
	stage = 'supply';
	for (const { spec, pack } of g.packages) {
		const r = door.supplyPackage(specText(spec), pack.files);
		if (!r || r.error) throw new Error('supply');
	}
	stage = 'failed';
	const project = { main: g.main, sources: g.sources, assets: g.assets, fonts: g.fonts, strict: true };
	const once = (to) => {
		const r = door.compileProject(project);
		const ok = !!(r && r.pdf && r.pdf.length > 4);
		if (ok) fs.writeFileSync(to, r.pdf);
		return { r, ok };
	};
	const first = once(a.out);
	const pages = first.r && Number.isInteger(first.r.pages) ? first.r.pages : 'none';
	const needs = first.r && Array.isArray(first.r.needs) ? first.r.needs.length : 0;
	console.log(`door ${first.ok ? 'ok' : 'error'} pages ${pages} kinds ${kinds(first.r)} needs ${needs}`);
	if (a.again) {
		console.log(`door again ${once(a.again).ok ? 'ok' : 'error'}`);
	}
	process.exit(first.ok ? 0 : 1);
} catch (e) {
	// Only a closed code: a message could name a file of the project.
	const code = e && e.code ? e.code : stage;
	console.log(`door fail ${['bad-args', 'bad-pkg', 'no-root', 'no-main', 'no-packages', 'bad-package', 'too-large', 'supply', 'failed'].includes(code) ? code : 'failed'}`);
	process.exit(2);
}
