#!/usr/bin/env node
// Reads phase_runs.sh's JSONL and prints, for each document, the median over its runs (each run's last
// attempt) of every phase: whole-run times, then the passes' phases summed over the passes, then each
// pass's phases. A median is taken for each cell on its own, so a column need not add up to its total;
// the `sum` line adds the medians of the run's own cells to show how close they come.
//
// Usage: phase_table.mjs FILE.jsonl
import fs from 'node:fs';

const RUN = ['load', 'eval', 'finish', 'write'];
const PASS = ['realise', 'flow', 'place', 'decorate', 'sink', 'settle'];

function median(xs) {
	const s = [...xs].sort((a, b) => a - b);
	const n = s.length;
	return n ? (n % 2 ? s[(n - 1) / 2] : (s[n / 2 - 1] + s[n / 2]) / 2) : NaN;
}

const file = process.argv[2];
const lines = fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
// Each (doc, run)'s last attempt.
const last = new Map();
for (const r of lines) last.set(`${r.doc}#${r.run}`, r);
const byDoc = new Map();
for (const r of last.values()) {
	if (!byDoc.has(r.doc)) byDoc.set(r.doc, []);
	byDoc.get(r.doc).push(r);
}
const ms = (ns) => (ns / 1e6).toFixed(1);
const out = [];
for (const [doc, runs] of byDoc) {
	const ok = runs.filter((r) => r.timings && r.exit_code === 0);
	const passes = ok.map((r) => r.timings.passes);
	out.push(`## ${doc}: ${runs.length} runs, ${ok.length} usable, fixpoint passes ${[...new Set(passes)].join('/')}, flagged ${runs.filter((r) => r.flagged).length}, PSI avg10 ${Math.min(...runs.map((r) => Math.min(r.psi_before, r.psi_after)))} to ${Math.max(...runs.map((r) => Math.max(r.psi_before, r.psi_after)))}`);
	const total = median(ok.map((r) => r.timings.total));
	const rows = [];
	const cell = (name, f, n) => {
		const v = median(ok.map(f));
		rows.push({ name, v, n: n ? median(ok.map(n)) : null });
		return v;
	};
	let sum = 0;
	for (const p of RUN) sum += cell(p, (r) => r.timings.run[p].ns, (r) => r.timings.run[p].n);
	for (const p of PASS) {
		sum += cell(p, (r) => r.timings.pass.reduce((a, b) => a + b[p].ns, 0), (r) => r.timings.pass.reduce((a, b) => a + b[p].n, 0));
	}
	out.push(['phase', 'ms', '% of total', 'entries'].join(' | '));
	for (const r of rows) out.push([r.name, ms(r.v), ((100 * r.v) / total).toFixed(1), r.n].join(' | '));
	out.push(['sum of medians', ms(sum), ((100 * sum) / total).toFixed(1), ''].join(' | '));
	out.push(['total (wall, in process)', ms(total), '100', ''].join(' | '));
	const eval_ = median(ok.map((r) => r.timings.run.eval.ns + r.timings.run.load.ns));
	const layout = median(ok.map((r) => r.timings.pass.reduce((a, b) => a + b.realise.ns + b.flow.ns + b.place.ns + b.decorate.ns, 0)));
	out.push(`load+eval ${ms(eval_)} ms (${((100 * eval_) / total).toFixed(1)}%); realise+flow+place+decorate ${ms(layout)} ms (${((100 * layout) / total).toFixed(1)}%)`);
	out.push(`cpu user ${median(ok.map((r) => r.user_s)).toFixed(2)} s, sys ${median(ok.map((r) => r.sys_s)).toFixed(2)} s, wall ${median(ok.map((r) => r.wall_s)).toFixed(2)} s, peak rss ${(median(ok.map((r) => r.rss_kb)) / 1024).toFixed(0)} MB, total spread ${ms(Math.min(...ok.map((r) => r.timings.total)))} to ${ms(Math.max(...ok.map((r) => r.timings.total)))} ms`);
	const maxPasses = Math.max(...passes);
	if (maxPasses > 1) {
		out.push('\nper pass (ms):');
		out.push(['pass', ...PASS].join(' | '));
		for (let i = 0; i < maxPasses; i++) {
			const row = PASS.map((p) => ms(median(ok.filter((r) => r.timings.pass[i]).map((r) => r.timings.pass[i][p].ns))));
			out.push([i + 1, ...row].join(' | '));
		}
	}
	out.push('');
}
process.stdout.write(out.join('\n') + '\n');
