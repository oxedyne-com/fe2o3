// gen_scrub_oracle.mjs -- writes `fe2o3_net/tests/data/lens_scrub.tsv`, the table that holds the
// content scrubber of `redact.js` to the same answers in Rust (`lens::Shapes`) and anywhere else.
//
//   node test/gen_scrub_oracle.mjs
//
// One line a case: `template <TAB> output <TAB> isId`. The template and the output are JSON
// strings. A template is expanded by the rule in `scrub_corpus.mjs` (the table holds no credential,
// only the recipe for one), the expansion is scrubbed by the base scrubber of `redact.js` (no
// caller shapes, pairs or id fields), and `isId` is 1 where the entropy catch is suppressed. A
// reader holds itself to the table by expanding each template, scrubbing it, and comparing.
import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { scrubText } from '../redact.js';
import { TEMPLATES, expand } from './scrub_corpus.mjs';

const OUT = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..', 'tests', 'data', 'lens_scrub.tsv');
const rows = TEMPLATES.map(([t, id]) => JSON.stringify(t) + '\t' + JSON.stringify(scrubText(expand(t), id === 1)) + '\t' + id);
writeFileSync(OUT, rows.join('\n') + '\n');
console.log('wrote ' + rows.length + ' cases to ' + OUT);
