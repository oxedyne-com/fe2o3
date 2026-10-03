// Walks a project root into the shape Daimond's gatherer (`src/wasm/typst.rs`, `gather`) hands the
// compiler: `{ root, main, sources, assets, fonts }`, with every path as the compiler's shadow
// filesystem names it, `/` and then the path under the root. It takes paths as arguments and knows no
// project: the same code gathers a three-file synthetic and a book.
//
// How a file is classified, by extension, after the directory it is in:
//   - under a font directory: a font when it is .ttf, .otf, .ttc or .otc, and not gathered otherwise;
//   - a `.typ` file, or text data (.bib .csv .json .yaml .yml .toml .xml .txt) that is valid UTF-8: a source;
//   - anything else: an asset, as bytes.
// Left out, as Daimond leaves them out: a `.pdf` (typst.ts cannot place one, and a project directory
// usually holds the last build), and any name that begins with a dot (`.git`, `.stversions`, ...).
//
// Run directly, it prints counts and nothing else, so it is safe to point at a project whose text
// must not appear in a log:
//
//     node gather.mjs <root> <main> [--font-dir <dir>]... [--exclude <dir>]...

import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export const TEXT_EXTS = ['typ', 'bib', 'csv', 'json', 'yaml', 'yml', 'toml', 'xml', 'txt'];
export const FONT_EXTS = ['ttf', 'otf', 'ttc', 'otc'];
export const SKIP_EXTS = ['pdf'];
export const FONT_DIRS = ['assets/fonts', 'fonts']; // Daimond's, relative to the root
export const MAX_BYTES = 512 * 1024 * 1024; // refused above this, never cut short
const PACKAGE_IMPORT = /import\s+"@[a-z0-9_-]+\//;

// A refusal carries a closed `code`, so a caller that must not print the message can print the code.
function refuse(code, message) {
	const e = new Error(message);
	e.code = code;
	return e;
}

// The stat of a path, or null for a link that points nowhere.
const statOf = (p) => {
	try {
		return fs.statSync(p);
	} catch {
		return null;
	}
};

const ext = (name) => {
	const i = name.lastIndexOf('.');
	return i < 0 ? '' : name.slice(i + 1).toLowerCase();
};

// The path under `root` as a shadow path, or null when `p` is not under it.
function shadow(root, p) {
	const rel = path.relative(root, p);
	if (rel === '' || rel.startsWith('..') || path.isAbsolute(rel)) return null;
	return '/' + rel.split(path.sep).join('/');
}

/// Gathers the project under `root` whose entry file is `main`.
///
/// # Arguments
/// * `main`    - The entry `.typ` file, under `root`, relative to it or absolute.
/// * `fontDirs` - Directories searched for fonts, relative to `root` or absolute; those that do not
///   exist are skipped. Defaults to Daimond's `assets/fonts` and `fonts`.
/// * `exclude` - Directories under `root` left out of the walk, relative to `root` or absolute.
export function gather(root, main, { fontDirs = FONT_DIRS, exclude = [], maxBytes = MAX_BYTES } = {}) {
	let rootAbs;
	try {
		rootAbs = fs.realpathSync(path.resolve(root));
	} catch {
		throw refuse('no-root', 'the root does not exist');
	}
	if (!fs.statSync(rootAbs).isDirectory()) throw refuse('no-root', 'the root is not a directory');
	const mainAbs = path.resolve(rootAbs, main);
	const mainShadow = shadow(rootAbs, mainAbs);
	if (!mainShadow || ext(mainAbs) !== 'typ' || !fs.existsSync(mainAbs)) {
		throw refuse('no-main', 'the main file is not a .typ file under the root');
	}
	const abs = (d) => path.resolve(rootAbs, d);
	const fontRoots = fontDirs.map(abs).filter((d) => fs.existsSync(d) && fs.statSync(d).isDirectory());
	const skip = exclude.map(abs);
	const under = (p, dir) => p === dir || p.startsWith(dir + path.sep);

	const sources = [];
	const assets = [];
	const fonts = [];
	const seenFont = new Set(); // by leaf name, as Daimond does: one face reached twice is one font
	const ancestry = new Set(); // real paths of the directories being walked, so a link back up is not followed
	let bytes = 0;

	const take = (n) => {
		bytes += n;
		if (bytes > maxBytes) throw refuse('too-large', 'the project is past the gather limit');
	};

	// Font directories are walked first and in full, wherever they are; the main walk then steps over them.
	const walkFonts = (dir) => {
		for (const e of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
			if (e.name.startsWith('.')) continue;
			const p = path.join(dir, e.name);
			const st = statOf(p);
			if (!st) continue;
			if (st.isDirectory()) {
				walkFonts(p);
			} else if (FONT_EXTS.includes(ext(e.name))) {
				const leaf = e.name.toLowerCase();
				if (seenFont.has(leaf)) continue;
				seenFont.add(leaf);
				const buf = fs.readFileSync(p);
				take(buf.length);
				fonts.push([e.name, buf]);
			}
		}
	};
	for (const d of fontRoots) walkFonts(d);

	const decoder = new TextDecoder('utf-8', { fatal: true });
	const walk = (dir) => {
		const real = fs.realpathSync(dir);
		if (ancestry.has(real)) return; // a link back into the walk
		ancestry.add(real);
		for (const e of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
			if (e.name.startsWith('.')) continue;
			const p = path.join(dir, e.name);
			if (skip.some((s) => under(p, s)) || fontRoots.some((f) => under(p, f))) continue;
			const st = statOf(p);
			if (!st) continue;
			if (st.isDirectory()) {
				walk(p);
				continue;
			}
			const x = ext(e.name);
			if (SKIP_EXTS.includes(x)) continue;
			const sp = shadow(rootAbs, p);
			if (!sp) continue;
			const buf = fs.readFileSync(p);
			take(buf.length);
			if (TEXT_EXTS.includes(x)) {
				try {
					sources.push([sp, decoder.decode(buf)]);
					continue;
				} catch {
					// Not UTF-8, so not text to the compiler either: it goes in as bytes.
				}
			}
			assets.push([sp, buf]);
		}
		ancestry.delete(real);
	};
	walk(rootAbs);

	// Sources that import a package. Daimond vendors a few and re-points the imports at them; this gather
	// does not, so a project that names one will not compile in typst.ts, and this count says why.
	const packages = sources.filter(([, t]) => PACKAGE_IMPORT.test(t)).length;

	return {
		root: rootAbs,
		main: mainShadow,
		sources,
		assets,
		fonts,
		stats: { sources: sources.length, assets: assets.length, fonts: fonts.length, packages, bytes },
	};
}

// ── Command line ─────────────────────────────────────────────────────────────

// Parses `<root> <main> [--font-dir d]... [--exclude d]...`; null on any other shape.
export function parseArgs(argv) {
	const pos = [];
	const fontDirs = [];
	const exclude = [];
	for (let i = 0; i < argv.length; i++) {
		const a = argv[i];
		if (a === '--font-dir' || a === '--exclude') {
			if (i + 1 >= argv.length) return null;
			(a === '--font-dir' ? fontDirs : exclude).push(argv[++i]);
		} else if (a.startsWith('--')) {
			return null;
		} else {
			pos.push(a);
		}
	}
	if (pos.length !== 2) return null;
	return { root: pos[0], main: pos[1], fontDirs: fontDirs.length ? fontDirs : FONT_DIRS, exclude };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
	const a = parseArgs(process.argv.slice(2));
	if (!a) {
		console.log('gather: error bad-args');
		process.exit(2);
	}
	try {
		const g = gather(a.root, a.main, { fontDirs: a.fontDirs, exclude: a.exclude });
		const s = g.stats;
		console.log(`gather: sources ${s.sources} assets ${s.assets} fonts ${s.fonts} packages ${s.packages} bytes ${s.bytes}`);
	} catch (e) {
		// Only the closed code: a message could name a file of the project.
		console.log(`gather: error ${e && e.code ? e.code : 'failed'}`);
		process.exit(1);
	}
}
