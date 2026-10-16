#!/usr/bin/env node
// Reads the JSONL a bench driver wrote and prints the verdict tables, in markdown.
//
//   bench_verdict.mjs typing --jsonl=FILE [--doc=NAME]
//       typing_sessions.sh's sessions (D-20261001-08): each session's nearest-rank p50 and p95, the
//       median of each across the valid sessions of an engine, T = max(1.25 x typst.ts, typst.ts +
//       16 ms) per percentile, and a pass when Austenite is at or under T for both. A round's last
//       attempt is the one that counts; an invalid session is listed and left out of the medians.
//   bench_verdict.mjs cost --jsonl=FILE [--doc=NAME] [--limit=1.5]
//       cost_runs.sh's cold and unchanged runs: the median of each (doc, format, mode, engine) cell
//       and its ratio to typst.ts, held against the limit (1.5).
import fs from 'node:fs';
import { median } from './lib/stats.mjs';
import { typingThreshold, floorBinds } from './lib/stats.mjs';

function parseArgs(argv) {
	const out = { _: [] };
	for (const a of argv.slice(2)) {
		const m = a.match(/^--([^=]+)=(.*)$/);
		if (m) out[m[1]] = m[2];
		else out._.push(a);
	}
	return out;
}

function readJsonl(file) {
	return fs
		.readFileSync(file, 'utf8')
		.split('\n')
		.filter((l) => l.trim())
		.map((l) => JSON.parse(l));
}

const ms = (s) => (s === null || s === undefined ? '-' : (s * 1000).toFixed(1));
const f2 = (x) => (x === null || x === undefined ? '-' : x.toFixed(2));

function typing(rows, docFilter) {
	const docs = [...new Set(rows.map((r) => r.doc))].filter((d) => !docFilter || d === docFilter);
	const out = [];
	for (const doc of docs) {
		const mine = rows.filter((r) => r.doc === doc);
		const sha = mine[0].doc_sha256;
		const rounds = [...new Set(mine.map((r) => r.round))].sort((a, b) => a - b);
		// The last attempt of each (engine, round).
		const last = {};
		for (const r of mine) {
			const k = `${r.engine}/${r.round}`;
			if (!last[k] || r.attempt > last[k].attempt) last[k] = r;
		}
		const edits = mine[0].edits;
		const valid = (r) => r && r.ok === true && Array.isArray(r.samples) && r.samples.length === edits;
		out.push(`### ${doc} (sha256 \`${sha}\`)\n`);
		out.push('| Session | Starts | Engine | p50 ms | p95 ms | open s | pages | PSI start / end | load1 start / end | attempt | note |');
		out.push('|---|---|---|---|---|---|---|---|---|---|---|');
		for (const rd of rounds) {
			const first = rd % 2 === 0 ? 'austenite' : 'typstts';
			for (const eng of ['austenite', 'typstts']) {
				const r = last[`${eng}/${rd}`];
				if (!r) continue;
				const notes = [];
				if (!valid(r)) notes.push(`INVALID: ${r.invalid || r.error || 'incomplete'}`);
				if (r.flagged) notes.push('FLAGGED (PSI avg10 > 20)');
				out.push(
					`| ${rd + 1} | ${first === eng ? 'first' : 'second'} | ${eng === 'austenite' ? 'Austenite' : 'typst.ts'} | ` +
						`${valid(r) ? ms(r.p50_s) : '-'} | ${valid(r) ? ms(r.p95_s) : '-'} | ${f2(r.open_s)} | ${r.pages ?? '-'} | ` +
						`${r.psi_before} / ${r.psi_after} | ${r.load_before} / ${r.load_after} | ${r.attempt} | ${notes.join('; ')} |`
				);
			}
		}
		const cell = (eng, key) => {
			const v = rounds.map((rd) => last[`${eng}/${rd}`]).filter(valid).map((r) => r[key]);
			return { n: v.length, med: median(v) };
		};
		const a50 = cell('austenite', 'p50_s'), a95 = cell('austenite', 'p95_s');
		const t50 = cell('typstts', 'p50_s'), t95 = cell('typstts', 'p95_s');
		out.push('');
		out.push(`Valid sessions: Austenite ${a50.n}, typst.ts ${t50.n}. Median across sessions:\n`);
		out.push('| Percentile | typst.ts ms | T ms | Austenite ms | A / typst.ts | A / T | 16 ms floor binds | Verdict |');
		out.push('|---|---|---|---|---|---|---|---|');
		const verdicts = [];
		for (const [name, a, t] of [['p50', a50, t50], ['p95', a95, t95]]) {
			if (a.med === null || t.med === null) {
				out.push(`| ${name} | - | - | - | - | - | - | NO DATA |`);
				verdicts.push(false);
				continue;
			}
			const T = typingThreshold(t.med);
			const pass = a.med <= T;
			verdicts.push(pass);
			out.push(
				`| ${name} | ${ms(t.med)} | ${ms(T)} | ${ms(a.med)} | ${f2(a.med / t.med)} | ${f2(a.med / T)} | ${floorBinds(t.med) ? 'yes' : 'no'} | ${pass ? 'pass' : 'miss'} |`
			);
		}
		out.push('');
		out.push(`**${doc}: ${verdicts.length === 2 && verdicts.every(Boolean) ? 'PASS' : 'MISS'}** (pass needs both percentiles at or under T)\n`);
	}
	return out.join('\n');
}

function cost(rows, docFilter, limit) {
	const docs = [...new Set(rows.map((r) => r.doc))].filter((d) => !docFilter || d === docFilter);
	const out = [];
	for (const doc of docs) {
		const mine = rows.filter((r) => r.doc === doc);
		out.push(`### ${doc} (sha256 \`${mine[0].doc_sha256}\`)\n`);
		out.push(`| Format | Mode | Runs per engine | typst.ts median s | Austenite median s | Ratio | Limit | Verdict | Flagged runs |`);
		out.push('|---|---|---|---|---|---|---|---|---|');
		const fmts = [...new Set(mine.map((r) => r.fmt))];
		for (const fmt of fmts) {
			for (const mode of ['cold', 'unchanged']) {
				const sel = mine.filter((r) => r.fmt === fmt && r.mode === mode);
				if (!sel.length) continue;
				const runs = (eng) => sel.filter((r) => r.engine === eng && r.ok === true).flatMap((r) => r.runs);
				const a = runs('austenite'), t = runs('typstts');
				const flagged = sel.filter((r) => r.flagged).length;
				if (!a.length || !t.length) {
					out.push(`| ${fmt} | ${mode} | ${a.length}/${t.length} | - | - | - | - | NO DATA | ${flagged} |`);
					continue;
				}
				const ratio = median(a) / median(t);
				out.push(
					`| ${fmt} | ${mode} | ${a.length}/${t.length} | ${median(t).toFixed(3)} | ${median(a).toFixed(3)} | ${ratio.toFixed(2)} | ${limit} | ${ratio <= limit ? 'pass' : 'miss'} | ${flagged} |`
				);
			}
		}
		out.push('');
	}
	return out.join('\n');
}

const args = parseArgs(process.argv);
const which = args._[0];
if (!args.jsonl || !['typing', 'cost'].includes(which)) {
	process.stderr.write('usage: bench_verdict.mjs typing|cost --jsonl=FILE [--doc=NAME] [--limit=1.5]\n');
	process.exit(2);
}
const rows = readJsonl(args.jsonl);
process.stdout.write((which === 'typing' ? typing(rows, args.doc) : cost(rows, args.doc, Number(args.limit || 1.5))) + '\n');
