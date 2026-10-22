// make_lens_adapter.mjs -- builds `lens_daimond.mjs`: what Daimond's `dev/lens.mjs` becomes when it
// adopts the extracted reader core.
//
//   node test/adapter/make_lens_adapter.mjs <path to daimond/dev/lens.mjs>
//   LENS_UNDER_TEST=$PWD/test/adapter/lens_daimond.mjs node <daimond>/dev/lens.test.mjs
//
// The views (`turnRecords`, `deviceStates`, `errorRecords`, `consoleRecords`, the `cmd*` commands
// and their formatting) are Daimond's own text, cut from its own file; they are what its test
// pins. What the core now owns is deleted by name and imported or bound instead: the argument and
// time helpers, the file and range helpers, the archive paths and loaders, the ingest (state,
// dedupe, chunk assembly, scrub on the way in), and the redaction block, which is `redact.js`.
// `cmdPull` keeps Daimond's report and calls the core's `pull`.
//
// The result is committed so the test needs no file of Daimond's but the two it runs. This script
// is how it was made; it refuses to run if a name it deletes is not found exactly once, so a
// change in Daimond's file is noticed rather than half-applied.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
let lines  = readFileSync(process.argv[2], 'utf8').split('\n');

// The top-level function `name`: its line range, its leading comment block included. Daimond's
// style closes a multi-line function with a lone `}` at the margin, which is what is looked for.
function span(name) {
	const re = new RegExp('^(?:async )?function ' + name + '\\(');
	const at = lines.map((l, i) => re.test(l) ? i : -1).filter((i) => i >= 0);
	if (at.length !== 1) throw new Error('expected exactly one top-level function ' + name + ', found ' + at.length);
	let a = at[0], b = a;
	const one = lines[a].trimEnd().endsWith('}') && (lines[a].match(/{/g) || []).length === (lines[a].match(/}/g) || []).length;
	if (!one) {
		b = lines.findIndex((l, i) => i > a && l === '}');
		if (b < 0) throw new Error('no closing brace for ' + name);
	}
	while (a > 0 && /^\/\/\/?( |$)/.test(lines[a - 1])) a--;
	return [a, b];
}

function drop(names) {
	const spans = names.map((n) => span(n)).sort((x, y) => y[0] - x[0]);
	for (const [a, b] of spans) lines.splice(a, b - a + 1);
}

function dropLine(prefix) {
	const at = lines.map((l, i) => l.startsWith(prefix) ? i : -1).filter((i) => i >= 0);
	if (at.length !== 1) throw new Error('expected exactly one line starting ' + prefix + ', found ' + at.length);
	lines.splice(at[0], 1);
}

function indexOfLine(prefix) {
	const at = lines.map((l, i) => l.startsWith(prefix) ? i : -1).filter((i) => i >= 0);
	if (at.length !== 1) throw new Error('expected exactly one line starting ' + prefix + ', found ' + at.length);
	return at[0];
}

// The redaction block (names, content shapes, scrubber): from its section heading to its end marker.
{
	const a = indexOfLine('// ── Redaction ──');
	const b = indexOfLine('// ── END OF THE SHARED SCRUBBER BLOCK');
	if (b <= a) throw new Error('the redaction block is not where it was');
	lines.splice(a, b - a + 1);
}

// What the core owns, by name.
const PURE = ['parseArgs', 'sinceMs', 'ensureDir', 'safeName', 'readJson', 'writeJson', 'readNdjson',
	'rangeHas', 'rangeAdd', 'rangeCount', 'rangeMissing', 'num', 'hm', 'sinceLabel', 'utcLabel', 'covLabel',
	'windowLabel', 'ago', 'dev7', 'idsMatch', 'emit'];
const BOUND = ['appendLine', 'loadState', 'eventsFile', 'telFile', 'snapIndexFile', 'snapBodyFile',
	'archiveDevices', 'wantDevice', 'coverage', 'loadTelemetry', 'loadEvents', 'loadSnapshotIndex', 'pendingSets'];
const INGEST = ['eachBlock', 'traceFiles', 'devState', 'blockSeen', 'blockMark', 'partsFile', 'materialise',
	'expire', 'closeSet', 'ingestRow', 'cmdPull'];
drop([...PURE, ...BOUND, ...INGEST]);
for (const re of ['const HEADER_RE', 'const CHUNK_RE', 'const EV_RE']) dropLine(re);

// The text the core's pull replaces: Daimond's report, over the core's counts.
const pull = `function cmdPull(args) {
	const stat = R.pull({ noPull: !!args.bool['no-rsync'] });
	stat.rsync = stat.pull;
	if (JSONOUT) { console.log(JSON.stringify(stat)); return 0; }
	console.log(\`pull: rsync \${stat.rsync}; \${stat.blocks} new block(s), \${stat.skipped} seen\`);
	console.log(\`  rows \${stat.rows}  events \${stat.events} (+\${stat.dupEvents} dup)\`
		+ \`  chunks \${stat.chunks} (+\${stat.dupChunks} dup)\`);
	console.log(\`  telemetry sets \${stat.telemetry}  snapshots \${stat.snapshots}\`
		+ \`  gapped \${stat.gapped}  broken \${stat.broken}\`);
	return 0;
}
`;
{
	const at = indexOfLine('// ── Reading the archive');
	lines.splice(at, 0, ...pull.split('\n'));
}

// Imports, and the reader built from Daimond's own constants.
{
	const at = indexOfLine("const RELOAD_GRACE_MS");
	const glue = `
// ── The core's reader, handed Daimond's seams ────────────────────────
import { createRedactor, createScrubber } from '../../redact.js';
import {
	ago, covLabel, createReader, dev7, emit, ensureDir, idsMatch, num, parseArgs, rangeAdd, rangeCount,
	rangeHas, rangeMissing, readJson, readNdjson, safeName, sinceLabel, sinceMs, utcLabel, windowLabel, writeJson, hm,
} from '../../reader.mjs';

const R = createReader({
	home:    HOME,
	mirror:  'traces',
	pull:    REMOTE ? [['rsync', '-az', REMOTE, TRACES + '/']] : null,
	// The gateway's own session cookie, and the fields whose values are ids this reader navigates by.
	scrub:   createScrubber({
		pairs:  [{ k: 'cookie', g: 2, re: /(daimond_gw_sess=)([^;,\\s"'\\[\\]]{4,})/g }],
		idKeys: { holder: 1, self: 1, nominated: 1, body: 1 },
	}),
	prepare: (b) => { if (b.config) b.config = createRedactor({ loose: true }).value(b.config); return b; },
	tick:    (b) => ({ iso: b.iso || null, stats: b.stats || null, ledger: b.ledger || [], trail: b.trail || [], diag: b.diag || [] }),
	index:   (b) => summariseSnapshot(b),
	nameFor: (d) => nameFor(d),
	recentMs: RECENT_MS, pendingTtlMs: PENDING_TTL_MS,
});
const { appendLine, archiveDevices, coverage, eventsFile, loadEvents, loadSnapshotIndex, loadState, loadTelemetry,
	pendingSets, snapBodyFile, snapIndexFile, telFile, wantDevice } = R;
`;
	// After the last of Daimond's constants, which the reader is built from. Import declarations are
	// hoisted, so they may stand here.
	lines.splice(at + 1, 0, ...glue.split('\n'));
}

const body = lines.join('\n');
const out = `/* lens_daimond.mjs -- GENERATED by make_lens_adapter.mjs. Daimond's dev/lens.mjs as it would
 * stand after adopting fe2o3_net/lens/js/reader.mjs: its own views over the extracted reader core.
 * Run Daimond's test against it with
 *   LENS_UNDER_TEST=<this file> node <daimond>/dev/lens.test.mjs
 */
${body}`;
writeFileSync(join(HERE, 'lens_daimond.mjs'), out);
console.log('wrote lens_daimond.mjs, ' + out.split('\n').length + ' lines (from ' + readFileSync(process.argv[2], 'utf8').split('\n').length + ')');
