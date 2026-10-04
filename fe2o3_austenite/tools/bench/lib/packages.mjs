// Typst packages for the typst.ts leg, supplied as Daimond supplies them (`src/wasm/typst.rs`: `Spec`,
// `imports`, `repoint`, `prepare`, `packages`, `parse_pack`). The scanners are ports of its own, so what
// is gathered here, what is followed into a package's dependencies and what is re-pointed cannot disagree.
//
// typst.ts has no network and no package registry, so Daimond carries packages beside the page and
// re-points every `#import "@preview/name:1.2.3"` at a one-line module it writes at
// `/_pkg/<ns>/<name>/<version>/<name>.typ`, which star-imports the package's entrypoint. A package is
// looked for in a directory laid out in either of two ways:
//
//   <dir>/<ns>/<name>/<version>/...        Typst's own package cache; the .typ files, `typst.toml` and the
//                                          licence are taken, as Daimond's refresh.sh takes them
//   <dir>/<ns>/<name>/<version>.pack       Daimond's vendored packs: an ASCII header, one blank line, the
//                                          file bodies end to end (`www/assets/typst/packs`)
//
// A package is `{ spec, pack }`: `spec` is `{ ns, name, version }` and `pack` is `{ entry, files }`, with
// `files` as `[[package-relative path, bytes]]`. That is the shape a host hands a compiler that takes
// packages from outside.

import fs from 'node:fs';
import path from 'node:path';

export const PKG_ROOT = '/_pkg';

// A refusal carries a closed `code`, so a caller that must not print the message can print the code.
export function refuse(code, message) {
	const e = new Error(message);
	e.code = code;
	return e;
}

const ext = (name) => {
	const i = name.lastIndexOf('.');
	return i < 0 ? '' : name.slice(i + 1).toLowerCase();
};

// ── A spec ──────────────────────────────────────────────────────────────────

const isWord = (s) => /^[A-Za-z0-9_-]+$/.test(s);
const isSemver = (s) => /^[0-9]+\.[0-9]+\.[0-9]+$/.test(s);

/// A spec from the text of a quoted literal, `@preview/cetz:0.3.4`, or null when it is not one.
export function parseSpec(lit) {
	if (!lit.startsWith('@')) return null;
	const rest = lit.slice(1);
	const slash = rest.indexOf('/');
	if (slash < 0) return null;
	const ns = rest.slice(0, slash);
	const tail = rest.slice(slash + 1);
	const colon = tail.indexOf(':');
	if (colon < 0) return null;
	const name = tail.slice(0, colon);
	const version = tail.slice(colon + 1);
	if (!isWord(ns) || !isWord(name) || !isSemver(version)) return null;
	return { ns, name, version };
}

export const specText = (s) => `@${s.ns}/${s.name}:${s.version}`;
export const specDir = (s) => `${PKG_ROOT}/${s.ns}/${s.name}/${s.version}`;
// Named for the package, so that `#import "@preview/cetz:0.3.4"` goes on binding `cetz`.
export const specShim = (s) => `${specDir(s)}/${s.name}.typ`;

// ── Reading a source for what it names ──────────────────────────────────────

/// Every double-quoted literal in `src` as `[start, end, raw text]`; `start` is the opening quote and `end`
/// one past the closing one.
export function quoted(src) {
	const out = [];
	const n = src.length;
	let i = 0;
	while (i < n) {
		if (src[i] !== '"') {
			i++;
			continue;
		}
		const start = i;
		i++;
		const from = i;
		let closed = false;
		while (i < n) {
			if (src[i] === '"') {
				closed = true;
				break;
			}
			if (src[i] === '\\' && i + 1 < n) {
				i += 2;
				continue;
			}
			if (src[i] === '\n') break;
			i++;
		}
		if (closed) {
			out.push([start, i + 1, src.slice(from, i)]);
			i++;
		} else {
			i = start + 1;
		}
	}
	return out;
}

/// Every raw block and raw span in `src`, as `[start, end]`: a run of N backticks opens a region that the
/// next run of exactly N closes.
export function rawSpans(src) {
	const out = [];
	const n = src.length;
	let i = 0;
	while (i < n) {
		if (src[i] !== '`') {
			i++;
			continue;
		}
		const open = i;
		let run = 0;
		while (i < n && src[i] === '`') {
			run++;
			i++;
		}
		let j = i;
		let end = null;
		while (j < n) {
			if (src[j] !== '`') {
				j++;
				continue;
			}
			let m = 0;
			while (j < n && src[j] === '`') {
				m++;
				j++;
			}
			if (m === run) {
				end = j;
				break;
			}
		}
		if (end === null) break;
		out.push([open, end]);
		i = end;
	}
	return out;
}

const within = (spans, at) => spans.some(([a, b]) => at >= a && at < b);

/// Is the literal beginning at `at` what an `#import` or `#include` is being given? A quotation of an import
/// in prose is not one.
function isImportOperand(src, at) {
	let i = at;
	while (i > 0 && /\s/.test(src[i - 1])) i--;
	for (const kw of ['import', 'include']) {
		const n = kw.length;
		if (i < n || src.slice(i - n, i) !== kw) continue;
		if (i === n) return true;
		// A longer identifier that merely ends in "import" is not the keyword.
		if (!/[A-Za-z0-9_-]/.test(src[i - n - 1])) return true;
	}
	return false;
}

/// Every package `src` imports, as `{ start, spec }`; a literal inside a raw block or span is not an import.
export function imports(src) {
	if (!src.includes('@')) return [];
	const raws = rawSpans(src);
	const out = [];
	for (const [start, , raw] of quoted(src)) {
		const spec = parseSpec(raw);
		if (!spec) continue;
		if (!isImportOperand(src, start)) continue;
		if (within(raws, start)) continue;
		out.push({ start, spec });
	}
	return out;
}

/// `.` and `..` collapsed in a slash path; null when it climbs above the top.
export function norm(p) {
	const out = [];
	for (const seg of p.split('/')) {
		if (seg === '' || seg === '.') continue;
		if (seg === '..') {
			if (out.pop() === undefined) return null;
			continue;
		}
		out.push(seg);
	}
	return out.join('/');
}

/// Re-points every package and package-root reference in `text`.
///
/// # Arguments
/// * `inside` - When `text` is a package's own file: `{ dir, names }`, the shadow directory the package sits
///   at and every path it holds. Null for a project's own source.
export function repoint(text, inside = null) {
	const imps = imports(text);
	const raws = rawSpans(text);
	let out = '';
	let last = 0;
	for (const [start, end, raw] of quoted(text)) {
		let rep = null;
		const imp = imps.find((i) => i.start === start);
		if (imp) {
			rep = specShim(imp.spec);
		} else if (inside && raw.startsWith('/') && !within(raws, start)) {
			// Inside a package a leading "/" means the package root, not the project's. Re-pointed only when it
			// names a file the pack holds, so a string of prose that begins with a slash is left alone.
			const p = norm(raw);
			if (p !== null && inside.names.includes(p)) rep = `${inside.dir}/pkg/${p}`;
		}
		if (rep !== null) {
			out += text.slice(last, start) + '"' + rep + '"';
			last = end;
		}
	}
	return out + text.slice(last);
}

const lossy = new TextDecoder('utf-8');

/// A package laid out for the compiler, read for what it imports in turn.
///
/// Returns `{ sources, assets, deps }`: shadow `.typ` files, re-pointed, with the one-line module that
/// carries the binding; shadow files that are not sources (the manifest, the licence); and the specs the
/// package imports, each with the file of its own that names it.
export function prepare(spec, pack) {
	const dir = specDir(spec);
	const names = pack.files.map(([p]) => p);
	const sources = [];
	const assets = [];
	const deps = [];
	for (const [rel, data] of pack.files) {
		const shadow = `${dir}/pkg/${rel}`;
		if (ext(rel) !== 'typ') {
			assets.push([shadow, data]);
			continue;
		}
		const text = lossy.decode(data);
		for (const { spec: dep } of imports(text)) {
			if (specText(dep) === specText(spec) || deps.some(([d]) => specText(d) === specText(dep))) continue;
			deps.push([dep, rel]);
		}
		sources.push([shadow, repoint(text, { dir, names })]);
	}
	// The binding: a star import re-exports whatever the entrypoint exports.
	sources.push([specShim(spec), `#import "pkg/${pack.entry}": *\n`]);
	return { sources, assets, deps };
}

// ── Finding a package ───────────────────────────────────────────────────────

const isDir = (p) => {
	try {
		return fs.statSync(p).isDirectory();
	} catch {
		return false;
	}
};
const isFile = (p) => {
	try {
		return fs.statSync(p).isFile();
	} catch {
		return false;
	}
};

// What of a package directory is taken, as Daimond's refresh.sh takes it.
const keep = (rel) => {
	const b = rel.slice(rel.lastIndexOf('/') + 1);
	return ext(rel) === 'typ' || b === 'typst.toml' || b === 'LICENSE' || b.startsWith('LICENSE.');
};

function readDirPackage(d) {
	const files = [];
	const walk = (dir, prefix) => {
		for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
			const rel = prefix ? `${prefix}/${e.name}` : e.name;
			const p = path.join(dir, e.name);
			if (e.isDirectory() || (e.isSymbolicLink() && isDir(p))) walk(p, rel);
			else if (keep(rel) && isFile(p)) files.push([rel, fs.readFileSync(p)]);
		}
	};
	walk(d, '');
	files.sort((a, b) => (Buffer.from(a[0]).compare(Buffer.from(b[0]))));
	const toml = files.find(([rel]) => rel === 'typst.toml');
	const m = toml && /^[ \t]*entrypoint[ \t]*=[ \t]*"(.*)".*$/m.exec(Buffer.from(toml[1]).toString('utf8'));
	if (!m) throw refuse('bad-package', 'a package names no entrypoint');
	const entry = m[1].replace(/^\/+/, '');
	if (!files.some(([rel]) => rel === entry)) throw refuse('bad-package', 'the entrypoint is not among the files');
	return { entry, files };
}

function readPack(bytes, spec) {
	let split = -1;
	for (let i = 0; i + 1 < bytes.length; i++) {
		if (bytes[i] === 0x0a && bytes[i + 1] === 0x0a) {
			split = i;
			break;
		}
	}
	if (split < 0) throw refuse('bad-package', 'a pack has no header');
	const lines = Buffer.from(bytes.subarray(0, split)).toString('utf8').split('\n');
	if (lines[0] !== 'DAIMOND TYPST PACK 1') throw refuse('bad-package', 'not a pack this reader knows');
	let entry = '';
	const got = { ns: '', name: '', version: '' };
	const names = [];
	for (const line of lines.slice(1)) {
		const sp = line.indexOf(' ');
		if (sp < 0) continue;
		const key = line.slice(0, sp);
		const rest = line.slice(sp + 1).trim();
		if (key === 'namespace') got.ns = rest;
		else if (key === 'name') got.name = rest;
		else if (key === 'version') got.version = rest;
		else if (key === 'entrypoint') entry = rest.replace(/^\/+/, '');
		else if (key === 'file') {
			const s2 = rest.indexOf(' ');
			if (s2 < 0) continue;
			const len = Number(rest.slice(0, s2));
			if (!Number.isInteger(len) || len < 0) continue;
			names.push([rest.slice(s2 + 1), len]);
		}
	}
	if (specText(got) !== specText(spec)) throw refuse('bad-package', 'a pack holds another package');
	if (!entry) throw refuse('bad-package', 'a pack names no entrypoint');
	const body = bytes.subarray(split + 2);
	const total = names.reduce((n, [, len]) => n + len, 0);
	if (total !== body.length) throw refuse('bad-package', 'a pack header does not account for its bytes');
	const files = [];
	let at = 0;
	for (const [p, len] of names) {
		files.push([p, body.subarray(at, at + len)]);
		at += len;
	}
	if (!files.some(([p]) => p === entry)) throw refuse('bad-package', 'the entrypoint is not among the files');
	return { entry, files };
}

/// The package `spec` names under `dir`, or null when `dir` holds none.
export function readPackage(dir, spec) {
	const d = path.join(dir, spec.ns, spec.name, spec.version);
	if (isDir(d)) return readDirPackage(d);
	const f = `${d}.pack`;
	if (isFile(f)) return readPack(fs.readFileSync(f), spec);
	return null;
}

/// The transitive closure of the packages the project's `.typ` sources import, looked for under `dir`
/// (null looks nowhere, so every spec is missing).
///
/// The project's own imports are taken in the order it makes them, and each package found is read for the
/// imports it makes, which go on the end of the same queue; a dependency is never assumed from a list.
/// Returns `{ found: [{ spec, pack }], missing: [spec], specs: [{ spec, found }] }`, `specs` in queue order.
///
/// # Arguments
/// * `sources` - `[[shadow path, text]]`; only those ending `.typ` are read.
export function closure(sources, dir) {
	const want = [];
	const has = (s) => want.some((w) => specText(w) === specText(s));
	for (const [p, text] of sources) {
		if (ext(p) !== 'typ') continue;
		for (const { spec } of imports(text)) if (!has(spec)) want.push(spec);
	}
	const found = [];
	const missing = [];
	const specs = [];
	for (let at = 0; at < want.length; at++) {
		const spec = want[at];
		const pack = dir ? readPackage(dir, spec) : null;
		if (!pack) {
			missing.push(spec);
			specs.push({ spec, found: false });
			continue;
		}
		specs.push({ spec, found: true });
		found.push({ spec, pack });
		for (const [dep] of prepare(spec, pack).deps) if (!has(dep)) want.push(dep);
	}
	return { found, missing, specs };
}
