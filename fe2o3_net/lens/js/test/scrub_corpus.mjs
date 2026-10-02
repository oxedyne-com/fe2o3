// scrub_corpus.mjs -- the inputs of `tests/data/lens_scrub.tsv`, as templates, and how to expand them.
//
// A template is built, never typed: a credential-shaped literal in a repository is what a scanner
// is for, so the table holds `{{n:seed:alphabet}}` where a run of characters belongs, and the
// scrubbed output, which holds markers and no credential. Whoever reads the table (the JavaScript
// test, and the Rust `lens::Shapes`) expands a template with the same rule and compares:
//
//   {{n:seed:alphabet}}   n characters; state h = (seed * 2654435761) mod 2^32; for each character,
//                         h = (h * 1103515245 + 12345) mod 2^32 (u32 wrapping arithmetic, exact) and
//                         take alphabet[(h >> 16) mod len]
//   {{dash5}}             five hyphens, which a PEM header is made of
//
// The alphabets are `alnum` (A-Z a-z 0-9), `hex` (0-9 a-f), `b64` (A-Z a-z 0-9 - _), `b64std`
// (A-Z a-z 0-9 / +), `upper36` (A-Z 0-9) and `digit` (0-9), in those orders.
export const ALPHABETS = {
	alnum:   'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789',
	hex:     '0123456789abcdef',
	b64:     'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_',
	b64std:  'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789/+',
	upper36: 'ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789',
	digit:   '0123456789',
};

function run(n, seed, alpha) {
	const a = ALPHABETS[alpha];
	if (!a) throw new Error('no alphabet ' + alpha);
	let s = '', h = (seed * 2654435761) >>> 0;
	for (let i = 0; i < n; i++) { h = (Math.imul(h, 1103515245) + 12345) >>> 0; s += a[(h >>> 16) % a.length]; }
	return s;
}

export function expand(t) {
	return t.replace(/\{\{(\d+):(\d+):(\w+)\}\}/g, (_, n, seed, alpha) => run(Number(n), Number(seed), alpha))
		.replace(/\{\{dash5\}\}/g, '-----');
}

// [template, isId]. Each credential shape at its minimum length, well above it, one short of it,
// in a sentence, twice in a string; the pairs; the entropy catch and what it must leave alone.
let seed = 100;
const q = (n, alpha) => '{{' + n + ':' + (seed++) + ':' + alpha + '}}';
export const TEMPLATES = [];
const add = (t, id) => TEMPLATES.push([t, id ? 1 : 0]);

// By shape.
add('{{dash5}}BEGIN RSA PRIVATE KEY{{dash5}}\n' + q(64, 'b64std') + '\n' + q(64, 'b64std') + '\n{{dash5}}END RSA PRIVATE KEY{{dash5}}');
add('log: {{dash5}}BEGIN PRIVATE KEY{{dash5}}\n' + q(40, 'b64std') + '… (clipped)');
add('{{dash5}}BEGIN CERTIFICATE{{dash5}}\n' + q(40, 'b64std') + '\n{{dash5}}END CERTIFICATE{{dash5}}');
for (const [pre, n] of [['ghp_', 16], ['gho_', 36], ['ghu_', 20], ['ghs_', 40], ['ghr_', 16], ['ghp_', 15]]) add('token ' + pre + q(n, 'alnum') + ' end');
for (const n of [20, 22, 40, 19]) add('github_pat_' + q(n, 'alnum'));
for (const [pre, n] of [['sk_test_', 10], ['sk_live_', 24], ['rk_live_', 24], ['rk_test_', 10], ['pk_test_', 24], ['sk_test_', 9]]) add('key=' + pre + q(n, 'alnum') + '&x=1');
for (const n of [16, 32, 15]) add('hook whsec_' + q(n, 'alnum'));
for (const [pre, n, a] of [['sk-', 16, 'b64'], ['sk-', 48, 'b64'], ['sk-or-v1-', 48, 'hex'], ['sk-ant-api03-', 80, 'b64'], ['sk-', 15, 'b64'], ['task-', 40, 'alnum']]) add(pre + q(n, a));
for (const [pre, n] of [['AKIA', 16], ['ASIA', 12], ['ABIA', 20], ['ACCA', 16], ['AGPA', 16], ['AIDA', 16], ['AIPA', 16], ['ANPA', 16], ['ANVA', 16], ['AROA', 16], ['APKA', 16], ['AKIA', 11], ['AKIB', 16]]) add('id ' + pre + q(n, 'upper36') + ' used');
for (const n of [35, 39, 29]) add('AIza' + q(n, 'b64'));
for (const [a, b] of [[12, 12], [10, 10], [9, 12]]) add('xoxb-' + q(a, 'alnum') + '-' + q(b, 'alnum'));
add('xoxp-' + q(12, 'alnum') + '-' + q(12, 'alnum') + '-' + q(24, 'alnum'));
for (const [a, b, c] of [[6, 6, 4], [20, 40, 43], [36, 120, 43], [5, 6, 4], [6, 5, 4], [6, 6, 3]]) add('Bearer eyJ' + q(a, 'b64') + '.' + q(b, 'b64') + '.' + q(c, 'b64') + ' ok');
for (const n of [32, 31, 40]) add('relay tune-' + q(n, 'hex') + ' up');

// The pairs, where the name survives and the value goes.
for (const w of ['Bearer', 'Basic', 'Token']) add('Authorization: ' + w + ' ' + q(20, 'b64'));
add('Authorization: Bearer ' + q(7, 'b64'));
add('Authorization: Bearer ' + q(8, 'b64'));
for (const k of ['token', 'access_token', 'refresh_token', 'id_token', 'api_key', 'api-key', 'apikey', 'key', 'auth', 'secret', 'password', 'passwd', 'sig', 'signature'])
	add('https://x.example/cb?a=1&' + k + '=' + q(12, 'alnum') + '&z=2');
add('https://x.example/cb#access_token=' + q(30, 'b64') + '#end');
add('https://x.example/cb?key=' + q(4, 'alnum'));
add('https://x.example/cb?key=' + q(3, 'alnum'));
add('https://x.example/cb?Token=' + q(12, 'alnum'));
for (const k of ['token', 'secret', 'apikey', 'api_key', 'api-key', 'auth_token', 'access-token', 'passphrase', 'password', 'master_key', 'private-key', 'mnemonic', 'seed_phrase', 'salt', 'sealed', 'wrapped'])
	add('config ' + k + ': ' + q(16, 'alnum') + ' done');
add('password="' + q(10, 'alnum') + '"');
add("secret = '" + q(12, 'alnum') + "'");
add('token=' + q(7, 'alnum'));
add('AKIA' + q(16, 'upper36') + ' and its secret ' + q(40, 'b64std') + ' here');
add('aws_secret_access_key = ' + q(40, 'b64std'));
add('AWS_SECRET_ACCESS_KEY: "' + q(40, 'b64std') + '"');
add('aws_secret_access_key = ' + q(39, 'b64std'));

// The entropy catch, and what the feed is supposed to carry.
for (const n of [32, 33, 48, 64, 128]) add('blob ' + q(n, 'b64') + ' end');
add(q(31, 'b64'));
for (const n of [32, 40, 64]) add('sha ' + q(n, 'hex') + ' ok');
add(q(8, 'hex') + '-' + q(4, 'hex') + '-' + q(4, 'hex') + '-' + q(4, 'hex') + '-' + q(12, 'hex'));
add('call_' + q(24, 'hex') + '_' + q(12, 'hex'));
add(q(40, 'digit'));
add('worker_pool_seat_000412_alpha_beta_gamma_delta');
add('build-6911be85c0f7-seq-312-stamp-20261002T120000Z-gamma');
add(q(40, 'b64'), true);
add('id sk-' + q(20, 'b64'), true);
add('id eyJ' + q(8, 'b64') + '.' + q(8, 'b64') + '.' + q(8, 'b64'), true);

// Together, and awkward.
add('first sk_test_' + q(24, 'alnum') + ' then eyJ' + q(10, 'b64') + '.' + q(10, 'b64') + '.' + q(10, 'b64') + ' then AKIA' + q(16, 'upper36'));
add('sk-' + q(20, 'b64') + ' sk-' + q(20, 'b64') + ' sk-' + q(20, 'b64'));
add('clé ' + 'sk_test_' + q(12, 'alnum') + ' 日本語 😀 ' + 'ghp_' + q(20, 'alnum'));
add('line one\nBearer ' + q(20, 'b64') + '\nline three ' + q(40, 'b64') + '\n');
add('[redacted stripe #1f2e3d4c/32] already scrubbed');
add('[redacted hi #deadbeef/40] and ?token=[redacted urlarg #12ab34cd/12]');
add('');
add('short');
add('       ');
add('plain prose with no credential at all, only words and a number 12345 and a path /usr/local/bin');
