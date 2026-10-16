#!/usr/bin/env node
// live_heap_check.mjs -- does the live view's open stay inside the PDF door's heap, as a bounded multiple.
//
// `compileProjectDelta` opened with `known: []` is the live view's first build, and wasm linear memory only
// grows, so `heapMB()` after it is the high-water mark Daimond's `holdCheck` compares for the life of the
// page. The open draws every page, yet the engine's own working set is the PDF door's, so the open must not
// cost much more than a PDF of the same document. Before the delta drew each page as it was taken and held
// every SVG in wasm, the open cost 9.4 times a PDF at 50 pages and 17 times at 300; the check holds it to
// RATIO, and the heap after the edits to the same line. The open now holds the frame of each page it will
// draw (about 0.1 MiB a page on the rich corpus) beyond what a PDF holds, and measures 1.3 to 1.5 times the
// PDF door on the rich and synthetic corpora and 1.8 times on the plain-text one at 797 pages; the line of
// 2.5 sits 1.4 times above the worst of those and 3.8 times below the fault it guards against.
//
// One fresh node process per measurement (`mem_open_probe.mjs`: `pdf`, then `edits` with the open and N
// one-letter edits), on the rich synthetic corpus `heap_probe.mjs --dump` writes. Exit 0 when every line
// holds, 1 when one fails, 2 when a measurement could not be made.
//
//   node tools/live_heap_check.mjs --pkg PKG [--pages 50,300] [--ratio 2.5] [--edits 5] [--phone 768]
//
// Run it under a memory cap, as every heavy job on this host is, and a 300-page cell through the slot helper:
//   systemd-run --user --scope --quiet -p MemoryMax=3G --slice=claude-rc.slice node tools/live_heap_check.mjs
//
// Options:
//   --pkg DIR        a wasm-pack `--target web` output directory for fe2o3_austenite (required)
//   --pages a,b      page counts to generate (default 50)
//   --ratio R        the most the open's heap may be, as a multiple of the PDF door's heap (default 2.5)
//   --phone MIB      also hold the open to this absolute heap, Daimond's phone budget (default 768; 0 skips)
//   --edits N        one-letter edits after the open (default 5)
//   --work DIR       where the generated sources go (default ~/.cache/austenite/live_heap_check)
//   --json FILE      write every measurement as JSON

import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const HOME = process.env.HOME || '';

function opt(name, dflt) {
	const i = process.argv.indexOf(name);
	return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : dflt;
}

const pkg = opt('--pkg', null);
if (!pkg) {
	process.stderr.write('live_heap_check: --pkg is required\n');
	process.exit(2);
}
const pages = opt('--pages', '50').split(',').map(Number);
const ratio = +opt('--ratio', '2.5');
const phone = +opt('--phone', '768');
const edits = +opt('--edits', '5');
const work = resolve(opt('--work', join(HOME, '.cache/austenite/live_heap_check')));
const jsonOut = opt('--json', null);
mkdirSync(work, { recursive: true });

const node = (args, what) => {
	const r = spawnSync(process.execPath, ['--expose-gc', ...args], { encoding: 'utf8', maxBuffer: 1 << 28 });
	if (r.status !== 0) {
		process.stderr.write(`live_heap_check: ${what} failed (exit ${r.status}): ${(r.stderr || '').slice(-400)}\n`);
		process.exit(2);
	}
	return r;
};

const rows = [];
let failed = false;
for (const n of pages) {
	const doc = join(work, `rich${n}.typ`);
	writeFileSync(doc, node([join(HERE, 'heap_probe.mjs'), '--dump', String(n)], `dump ${n}`).stdout);
	const probe = (exp, extra = []) => JSON.parse(node(
		[join(HERE, 'mem_open_probe.mjs'), '--pkg', pkg, '--doc', doc, '--exp', exp, ...extra], `${exp} ${n}`,
	).stdout.trim().split('\n').pop());
	const p = probe('pdf');
	const o = probe('edits', ['--edits', String(edits)]);
	const open = o.calls.find((c) => c.what === 'open');
	const last = o.calls[o.calls.length - 1];
	if (!open || open.error || !p.calls[0] || p.calls[0].error) {
		process.stderr.write(`live_heap_check: ${n} pages: a compile failed: ${JSON.stringify([open, p.calls[0]])}\n`);
		process.exit(2);
	}
	const pdf = p.heap_end_mb;
	const row = {
		pages: n, pagesOut: open.pages, pdf_mib: pdf, open_mib: open.heap_mb, after_edits_mib: o.heap_end_mb,
		open_ms: open.ms, edit_ms: o.calls.filter((c) => c.what === 'edit').map((c) => c.ms),
		open_over_pdf: open.heap_mb / pdf, edits_over_pdf: o.heap_end_mb / pdf, last: last.what,
	};
	const bad = [];
	if (row.open_over_pdf > ratio) bad.push(`the open is ${row.open_over_pdf.toFixed(2)}x the PDF door's heap, over ${ratio}`);
	if (row.edits_over_pdf > ratio) bad.push(`after the edits ${row.edits_over_pdf.toFixed(2)}x, over ${ratio}`);
	if (phone > 0 && o.heap_end_mb > phone) bad.push(`${o.heap_end_mb.toFixed(1)} MiB over the phone budget ${phone}`);
	row.verdict = bad.length ? 'FAIL' : 'ok';
	row.bad = bad;
	failed = failed || bad.length > 0;
	rows.push(row);
	process.stdout.write(
		`${row.verdict} ${n} pages (${open.pages} out): PDF ${pdf.toFixed(1)} MiB, open ${open.heap_mb.toFixed(1)} ` +
		`(${row.open_over_pdf.toFixed(2)}x, ${open.ms} ms), after ${edits} edits ${o.heap_end_mb.toFixed(1)} ` +
		`(${row.edits_over_pdf.toFixed(2)}x)${bad.length ? ': ' + bad.join('; ') : ''}\n`,
	);
}
if (jsonOut) writeFileSync(jsonOut, JSON.stringify({ ratio, phone, edits, rows }, null, 1));
process.exit(failed ? 1 : 0);
