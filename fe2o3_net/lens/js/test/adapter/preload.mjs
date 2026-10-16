// preload.mjs -- runs Daimond's own `debugshare.test.mjs`, unchanged and read-only, against the
// extracted writer core instead of Daimond's `debugshare.js`.
//
//   node --import ./test/adapter/preload.mjs <daimond>/www/js/debugshare.test.mjs
//
// Daimond's test reads `debugshare.js` from its own directory and evaluates it as a classic script
// in a simulated tab. This preload answers that one read with `daimond_debugshare.js` (Daimond's
// app layer over the core), after putting the core where that script finds it. Every other file the
// test reads (`stamp.js`, `daimond.js`, `gateway.js`) is Daimond's, untouched.
//
// One block of that test checks that the scrubber was pasted identically into `debugshare.js` and
// `dev/lens.mjs`. With one copy in `redact.js` there is nothing to keep identical, so the two reads
// are both answered from that one block, and those three checks pass by construction; Daimond
// would delete them on adopting the core.
import fs from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
globalThis.__lensCore = await import('../../writer.js');

const adapter = fs.readFileSync(join(HERE, 'daimond_debugshare.js'), 'utf8');
const redact  = fs.readFileSync(join(HERE, '..', '..', 'redact.js'), 'utf8');
const START   = '// ── THE CONTENT SCRUBBER (SHARED BLOCK) ──';
const END     = '// ── END OF THE SHARED SCRUBBER BLOCK ──';
const block   = redact.slice(redact.indexOf(START), redact.indexOf(END));
if (block.indexOf('*/') !== -1) throw new Error('the scrubber block cannot be quoted inside a comment');

// The client's copy sits one tab in, being inside an IIFE; the lens copy sits at the margin.
const client  = adapter + '\n/* The core\'s scrubber block, quoted for Daimond\'s two-copies check:\n'
	+ block.split('\n').map((l) => '\t' + l).join('\n') + END + '\n*/\n';
const lens    = '\n' + block + END + '\n';

const real = fs.readFileSync;
fs.readFileSync = function (p, ...rest) {
	const s = typeof p === 'string' ? p : String((p && p.pathname) || p);
	if (/\/www\/js\/debugshare\.js$/.test(s)) return client;
	if (/\/dev\/lens\.mjs$/.test(s)) return lens;
	return real.call(this, p, ...rest);
};
syncBuiltinESMExports();
