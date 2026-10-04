// The typst.ts half of `skew.sh`: gathers a project the way Daimond does, compiles it to a PDF with the
// vendored typst.ts in node, and writes the PDF where `--out` says. It prints closed-vocabulary lines and
// nothing else: typst.ts's diagnostics quote the source, so they are never printed.
//
//     node skew_ts.mjs --root <dir> --main <file> --vendor <dir> --out <pdf> [--font-dir <dir>]... [--exclude <dir>]...
//                      [--packages <dir>]
//
// Prints: `typstts: gathered sources <n> assets <n> fonts <n> packages <n> missing <n>`; one
// `typstts: package preview <name> <version> <supplied|missing>` for each spec in the registry namespace
// `preview` (a public name; specs of any other namespace are only counted); then `typstts: compile ok` or
// `typstts: compile failed`; or `typstts: error <code>` with a code from {bad-args, no-root, no-main,
// no-packages, bad-package, too-large, failed}.

import fs from 'node:fs';
import { gather, FONT_DIRS } from '../bench/lib/gather.mjs';
import { loadTypstTs } from '../bench/lib/wasm_common.mjs';

function args(argv) {
	const a = { fontDirs: [], exclude: [] };
	for (let i = 0; i < argv.length; i += 2) {
		const k = argv[i];
		const v = argv[i + 1];
		if (v === undefined) return null;
		if (k === '--font-dir') a.fontDirs.push(v);
		else if (k === '--exclude') a.exclude.push(v);
		else if (k === '--packages') a.packages = v;
		else if (k === '--root') a.root = v;
		else if (k === '--main') a.main = v;
		else if (k === '--vendor') a.vendor = v;
		else if (k === '--out') a.out = v;
		else return null;
	}
	if (!a.root || !a.main || !a.vendor || !a.out) return null;
	if (!a.fontDirs.length) a.fontDirs = FONT_DIRS;
	return a;
}

const a = args(process.argv.slice(2));
if (!a) {
	console.log('typstts: error bad-args');
	process.exit(2);
}

let code = 'failed';
try {
	const g = gather(a.root, a.main, { fontDirs: a.fontDirs, exclude: a.exclude, packages: a.packages ?? null });
	console.log(`typstts: gathered sources ${g.stats.sources} assets ${g.stats.assets} fonts ${g.stats.fonts} packages ${g.stats.packages} missing ${g.stats.missing}`);
	for (const { spec, found } of g.specs) {
		if (spec.ns === 'preview') console.log(`typstts: package preview ${spec.name} ${spec.version} ${found ? 'supplied' : 'missing'}`);
	}
	code = 'compile';
	const ts = await loadTypstTs(a.vendor, g);
	const r = ts.compilePdf({ main: g.main, sources: g.sources, assets: g.assets });
	if (r.pdf && r.pdf.length > 4) {
		fs.writeFileSync(a.out, r.pdf);
		console.log('typstts: compile ok');
	} else {
		console.log('typstts: compile failed');
		process.exit(1);
	}
} catch (e) {
	if (code === 'compile') console.log('typstts: compile failed');
	else console.log(`typstts: error ${e && e.code ? e.code : 'failed'}`);
	process.exit(1);
}
