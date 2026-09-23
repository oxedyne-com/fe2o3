// Shared node-side helpers for the S0 wasm bench: loading the two vendored wasm
// compilers exactly as Daimond's `www/js/typst.js` sequences them (dummy access
// model, the same bundled font set, shadow sources), but reading everything with
// `fs` instead of `fetch`, because Node's `fetch` does not resolve `file://` --
// confirmed on this host, Node v20.20.2. This is the one deliberate deviation
// from the browser driving code; see tools/bench/README.md.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const DIAG_FULL = 3; // typst.ts diagnostics_format: full (matches www/js/typst.js)

const FONTS = [
	'LibertinusSerif-Regular.otf',
	'LibertinusSerif-Bold.otf',
	'LibertinusSerif-Italic.otf',
	'LibertinusSerif-BoldItalic.otf',
	'NewCMMath-Regular.otf',
];

/// Reads a `.typ` file as one project source at `/main.typ`, the shape both
/// wasm compile entry points want (`{ main, sources: [[path, text], ...] }`).
export function projectFromFile(typPath) {
	const text = fs.readFileSync(typPath, 'utf8');
	return { main: '/main.typ', sources: [['/main.typ', text]] };
}

/// Builds the Austenite wasm instance from a vendor directory containing
/// `oxedyne_fe2o3_austenite.js` and `..._bg.wasm`, exactly as `getAustenite()`
/// does in `www/js/typst.js`.
export async function loadAustenite(vendorDir) {
	const glue = pathToFileURL(path.join(vendorDir, 'oxedyne_fe2o3_austenite.js')).href;
	const wasmPath = path.join(vendorDir, 'oxedyne_fe2o3_austenite_bg.wasm');
	const mod = await import(glue);
	await mod.default(fs.readFileSync(wasmPath));
	const instance = new mod.DaimondTypst();
	return {
		engine: 'austenite',
		compilePdf(project) {
			return instance.compileProject(project);
		},
		compileVector(project) {
			return instance.compileProjectVector(project);
		},
		compileDelta(project) {
			return instance.compileProjectDelta(project);
		},
		heapMB() {
			return instance.heapMB();
		},
	};
}

/// Builds the typst.ts wasm compiler from a vendor directory containing
/// `typst_ts_web_compiler.mjs`/`..._bg.wasm` and a `fonts/` subdirectory with
/// the bundled font set, exactly as `getCompiler()` does in `www/js/typst.js`:
/// a dummy access model (sources are injected, nothing is read from disk) and
/// the same five bundled fonts.
export async function loadTypstTs(vendorDir) {
	const glue = pathToFileURL(path.join(vendorDir, 'typst_ts_web_compiler.mjs')).href;
	const wasmPath = path.join(vendorDir, 'typst_ts_web_compiler_bg.wasm');
	const mod = await import(glue);
	await mod.default(fs.readFileSync(wasmPath));
	const builder = new mod.TypstCompilerBuilder();
	builder.set_dummy_access_model();
	for (const name of FONTS) {
		const buf = fs.readFileSync(path.join(vendorDir, 'fonts', name));
		await builder.add_raw_font(buf);
	}
	const compiler = await builder.build();
	let lastMain = null;
	function ensureSources(project) {
		compiler.reset_shadow();
		for (const [p, text] of project.sources) {
			if (compiler.add_source(p, text) === false) {
				throw new Error(`typst.ts refused source ${p}`);
			}
		}
		lastMain = project.main;
	}
	function extract(ret, key) {
		if (ret instanceof Uint8Array) return ret;
		if (ret && typeof ret === 'object') {
			const cand = ret.result ?? ret.artifact ?? ret[key];
			if (cand instanceof Uint8Array) return cand;
			if (cand && cand.buffer) return new Uint8Array(cand.buffer);
		}
		return null;
	}
	return {
		engine: 'typstts',
		compilePdf(project) {
			ensureSources(project);
			const ret = compiler.compile(project.main, undefined, 'pdf', DIAG_FULL);
			const bytes = extract(ret, 'pdf');
			return bytes ? { pdf: bytes } : { error: JSON.stringify(ret).slice(0, 500) };
		},
		compileVector(project) {
			ensureSources(project);
			const ret = compiler.compile(project.main, undefined, 'vector', DIAG_FULL);
			const bytes = extract(ret, 'vector');
			return bytes ? { vector: bytes } : { error: JSON.stringify(ret).slice(0, 500) };
		},
		// typst.ts has no changed-only delta door in Daimond's wiring (see
		// tools/bench/README.md): its live-view path is a full re-compile to
		// `vector` on every edit, which is what edit_latency.mjs times.
		compileDelta(project) {
			return this.compileVector(project);
		},
	};
}

export async function loadEngine(kind, vendorDir) {
	if (kind === 'austenite') return loadAustenite(vendorDir);
	if (kind === 'typstts') return loadTypstTs(vendorDir);
	throw new Error(`unknown engine ${kind}`);
}

/// One wall-clock timing of a synchronous compile call, in seconds.
export function timeOnce(fn) {
	const t0 = process.hrtime.bigint();
	const ret = fn();
	const t1 = process.hrtime.bigint();
	return { wall_s: Number(t1 - t0) / 1e9, ret };
}

export function loadavg1() {
	// os.loadavg()[0] mirrors /proc/loadavg field 1.
	return os.loadavg()[0];
}

export function psiAvg10(resource) {
	try {
		const raw = fs.readFileSync(`/proc/pressure/${resource}`, 'utf8');
		const line = raw.split('\n').find((l) => l.startsWith('some'));
		const m = line && line.match(/avg10=([0-9.]+)/);
		return m ? Number(m[1]) : 0;
	} catch {
		return 0;
	}
}

export const __dirnameOf = (importMetaUrl) => path.dirname(fileURLToPath(importMetaUrl));
