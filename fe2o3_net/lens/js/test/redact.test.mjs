// redact.test.mjs -- the redactor and the content scrubber of `../redact.js`.
//
// Two halves. The first is the scrubber section of Daimond's `www/js/debugshare.test.mjs`
// (one fixture per credential shape, the entropy catch, what the feed is supposed to carry),
// lifted with only these changes: `DS._scrubText` and `DS._scrubDeep` became the scrubber
// below, and that scrubber is built the way Daimond would build it, naming its own session
// cookie and its own correlation-id fields as parameters (the core knows neither).
// The second half tests what the extraction added: the caller deny-list hook, the caller string
// test, the fingerprint head, and the name rule that `lens_secret_re.tsv` pins for Rust.
//
//   node test/redact.test.mjs
import {
	SECRET_RE,
	SECRET_RE_LOOSE,
	byteLen,
	createRedactor,
	createScrubber,
	fingerprint,
	scrubDeep as baseDeep,
	scrubText as baseText,
	secretName,
} from '../redact.js';
import { check, finish, section } from './harness.mjs';

const NL = String.fromCharCode(10);

// Daimond's own additions to the scrubber: the gateway's session cookie, and the fields whose
// values are ids it navigates by.
const D = createScrubber({
	pairs:  [{ k: 'cookie', g: 2, re: /(daimond_gw_sess=)([^;,\s"'\[\]]{4,})/g }],
	idKeys: { holder: 1, self: 1, nominated: 1, body: 1 },
});
const scrubText = D.text;
const scrubDeep = D.deep;

function lifted() {
	// ══ THE CONTENT SCRUBBER ══════════════════════════════════════════
	//
	// The name-based redactors answer for `config`, whose fields are NAMED. These
	// blocks answer the owner's general question -- "the debug feed does not
	// contain keys" -- for everything else the feed carries, which is free text: a
	// console line, the tile of the daimon's own answer, a failed fetch's URL, a
	// tool argument, a stack frame. Every fixture below is INVENTED here and wears
	// only the SHAPE of a real credential; none has ever been a live key, and each
	// is BUILT from a seed rather than typed out, so no line of this file is a
	// paste-able credential and a scanner reading the repository finds none.

	// A deterministic run of characters from a seed. Not random: a fixture that
	// changes between runs cannot be asserted about.
	const fakeRun = (n, seed, alpha) => {
		alpha = alpha || 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
		let s = '', h = (seed * 2654435761) >>> 0;
		for (let i = 0; i < n; i++) { h = (h * 1103515245 + 12345) >>> 0; s += alpha[(h >>> 8) % alpha.length]; }
		return s;
	};
	const fakeHex = (n, seed) => fakeRun(n, seed, '0123456789abcdef');
	const fakeB64 = (n, seed) => fakeRun(n, seed, 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_');

	console.log('debugshare: scrubber — one fixture per credential shape, none survives');
	{

		// One entry per shape the scrubber is asked to know.
		const CORPUS = [
			['github classic',		'ghp_' + fakeRun(36, 11)],
			['github oauth',		'gho_' + fakeRun(36, 12)],
			['github user',			'ghu_' + fakeRun(36, 13)],
			['github server',		'ghs_' + fakeRun(36, 14)],
			['github refresh',		'ghr_' + fakeRun(36, 15)],
			['github fine PAT',		'github_pat_' + fakeRun(22, 16) + '_' + fakeRun(59, 17)],
			['openai',				'sk-' + fakeRun(48, 21)],
			['openrouter',			'sk-or-v1-' + fakeHex(64, 22)],
			['anthropic',			'sk-ant-api03-' + fakeB64(95, 23)],
			['stripe live',			'sk_live_' + fakeRun(24, 31)],
			['stripe test',			'sk_test_' + fakeRun(24, 32)],
			['stripe restricted',	'rk_live_' + fakeRun(24, 33)],
			['stripe webhook',		'whsec_' + fakeRun(32, 34)],
			['aws access key',		'AKIA' + fakeRun(16, 41).toUpperCase()],
			['google api key',		'AIza' + fakeB64(35, 51)],
			['slack bot',			'xoxb-' + fakeRun(12, 61) + '-' + fakeRun(12, 62) + '-' + fakeRun(24, 63)],
			['jwt',					'eyJ' + fakeB64(28, 71) + '.eyJ' + fakeB64(40, 72) + '.' + fakeB64(43, 73)],
			['tune relay run token',	'tune-' + fakeHex(32, 81)],
			['high-entropy blob',	fakeB64(48, 91)],
		];
		const survived = [];
		for (const [name, val] of CORPUS) {
			if (scrubText('the call failed: ' + val + ' was refused').indexOf(val) !== -1) survived.push(name);
		}
		if (survived.length) console.log('       survived: ' + survived.join(', '));
		check('every enumerated shape is replaced, not passed through', survived.length === 0);

		// The three whose shape only exists NEXT TO something else.
		const AWS_ID  = 'AKIA' + fakeRun(16, 42).toUpperCase();
		const AWS_SEC = fakeRun(40, 43);
		const PEM_BODY = fakeB64(64, 101);
		// The PEM ARMOUR only -- the body between the lines is generated above and
		// is not key material. allowlist secret
		const PEM = '-----BEGIN RSA PRIVATE KEY-----' + NL + PEM_BODY + NL + '-----END RSA PRIVATE KEY-----';
		check('a PEM private key block goes whole',
			scrubText('key material:' + NL + PEM + NL + 'done').indexOf(PEM_BODY) === -1);
		check('a PEM block clipped by elision still goes',	// allowlist secret
			scrubText('-----BEGIN EC PRIVATE KEY-----' + NL + fakeB64(64, 103)).indexOf(fakeB64(64, 103)) === -1);
		check('an AWS secret beside its access-key id goes',
			scrubText('creds: ' + AWS_ID + ', "' + AWS_SEC + '"').indexOf(AWS_SEC) === -1);
		check('an AWS secret beside its own field name goes',
			scrubText('aws_secret_access_key = ' + AWS_SEC).indexOf(AWS_SEC) === -1);

		// The marker: the shape, a stable hash and the length -- so two sightings of
		// one key are known to be one key, and nothing more.
		const GH = 'ghp_' + fakeRun(36, 11);
		check('the marker keeps the redactConfig style',
			/^\[redacted gh #[0-9a-f]+\/40\]$/.test(scrubText(GH)));
		check('the same value marks identically in two places',
			scrubText('a ' + GH) === 'a ' + scrubText('b ' + GH).slice(2));
		check('two different values mark differently',
			scrubText(GH) !== scrubText('ghp_' + fakeRun(36, 12)));

		// The name-against-a-value rules, which keep the LINE and lose the VALUE --
		// the thing fingerprinting a whole console message could not do.
		const BEAR = fakeRun(40, 111);
		const line = scrubText('[gw] Authorization: Bearer ' + BEAR + ' rejected');
		check('a bearer token goes and the line survives',
			line.indexOf(BEAR) === -1 && line.indexOf('[gw]') === 0 && line.indexOf('rejected') > 0);
		const TOK = fakeRun(30, 112);
		const url = scrubText('POST /api/sync?access_token=' + TOK + '&v=2 failed 401');
		check('a token in a URL query goes and the URL survives',
			url.indexOf(TOK) === -1 && url.indexOf('POST /api/sync?access_token=') === 0
			&& url.indexOf('&v=2 failed 401') > 0);
		check('an `&key=` argument goes too',
			scrubText('GET /x?a=1&key=' + fakeRun(30, 113)).indexOf(fakeRun(30, 113)) === -1);
		check('the gateway session cookie goes',
			scrubText('Cookie: daimond_gw_sess=' + fakeRun(43, 114) + '; Path=/').indexOf(fakeRun(43, 114)) === -1);
		check('a secret name sitting against a value goes',
			scrubText('config apiKey="' + fakeRun(30, 115) + '" loaded').indexOf(fakeRun(30, 115)) === -1);

		// IDEMPOTENCE, which is not a nicety here: `dev/lens.mjs` runs this same
		// block AGAIN on ingest, over output the client has already scrubbed. A rule
		// whose replacement its own pattern can match nests its markers one deep per
		// pass -- which the `?token=` rule did, until `[` and `]` were taken out of
		// its value class.
		const twice = [
			'POST /api/sync?access_token=' + fakeRun(30, 121) + ' failed',
			'Cookie: daimond_gw_sess=' + fakeRun(43, 122) + '; Path=/',
			'Authorization: Bearer ' + fakeRun(40, 123),
			'ghp_' + fakeRun(36, 124),
			'apiKey=' + fakeRun(30, 125),
			'blob ' + fakeB64(48, 126) + ' end',
		];
		const nested = twice.filter((t) => scrubText(scrubText(t)) !== scrubText(t));
		check('scrubbing twice is scrubbing once, for every rule', nested.length === 0);
	}

	console.log('debugshare: scrubber — what the feed is SUPPOSED to carry still reads');
	{
		// The legibility half. A scrubber that eats the archive's own index is no
		// use, so every shape the reader navigates by is whitelisted BY SHAPE.
		const KEEP = [
			['a sha256 digest',	'3b1f8c2d4e5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1c'],
			['a build id',		'dae1ed646c3f'],
			['a device id',		'a7b34e2d5181e710301a30ebbc3ee062'],
			['a UUID',			'0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9'],
			['a message id',	'mgk3j2a-1f-x7q2p'],
			['a model id',		'anthropic/claude-sonnet-4-5-20250929'],
			['a chunk tag',		'ds snapshot smtxxyvgy988 41/2377'],
			['a sentence',		'[sync] chunk index not merged on this device; refusing to commit'],
			['a constant',		'TRANSCRIPT_BUDGET_BYTES exceeded by 41 bytes'],
			['a worded id',		'worker_pool_seat_000412'],
			['a stack frame',	'sync.js:1511'],
			['a long decimal',	'178930080399617893008039961789300803996'],
		];
		const lost = [];
		for (const [name, val] of KEEP) if (scrubText(val) !== val) lost.push(name);
		if (lost.length) console.log('       lost: ' + lost.join(', '));
		check('every legible shape survives untouched', lost.length === 0);

		// And the correlation-id FIELDS, whose values a provider mints out of an
		// alphabet a key would be at home in.
		const callId = 'call_00_AbC9dEf1GhI2jKl3MnO4pQr5';
		const walked = scrubDeep({ id: callId, tool_call_id: callId, callId: callId,
			turn: 'smtxxyvgy988', notAnId: callId });
		check('a tool-call id survives in an id field',
			walked.id === callId && walked.tool_call_id === callId && walked.callId === callId);
		check('but the same run in an ordinary field does not', walked.notAnId !== callId);
		check('and an id field holding a real key shape is STILL scrubbed',
			scrubDeep({ id: 'ghp_' + fakeRun(36, 11) }).id.indexOf(fakeRun(36, 11)) === -1);
	}
}
lifted();

// ── What the extraction added ────────────────────────────────────────

section('redact: the core carries nothing of the app that lent it');
{
	check('the base scrubber does not know Daimond\'s session cookie',
		baseText('cookie: daimond_gw_sess=abcd1234efgh') === 'cookie: daimond_gw_sess=abcd1234efgh');
	check('the configured scrubber does', D.text('cookie: daimond_gw_sess=abcd1234efgh').indexOf('abcd1234efgh') === -1);
	check('the base scrubber still takes a key shape', baseText('x ghp_' + 'A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8') .indexOf('A1b2C3d4') === -1);
	check('the base and a default-built scrubber agree',
		createScrubber().text('Bearer abcdefghijkl') === baseText('Bearer abcdefghijkl'));
	check('the base deep walk copies and does not mutate',
		(() => { const o = { a: ['sk-' + 'x'.repeat(20) + 'Q9'] }; const c = baseDeep(o); return c !== o && o.a[0].startsWith('sk-x'); })());
}

section('redact: the name rule, strict and loose');
{
	check('apikey, key, token are strict hits', ['apikey', 'key', 'token', 'x_secret', 'a.password', 'b-salt'].every((n) => SECRET_RE.test(n)));
	check('a separator-free camelCase name slips the strict rule', !SECRET_RE.test('pushToken') && !SECRET_RE.test('refreshSecret'));
	check('and the loose rule catches it', SECRET_RE_LOOSE.test('pushToken') && SECRET_RE_LOOSE.test('refreshSecret'));
	check('keys and monkey are not secrets', !SECRET_RE.test('keys') && !SECRET_RE.test('monkey'));
	check('secretName(name) is the strict rule, secretName(name, true) the strict-or-loose',
		secretName('apikeyenc') === true && secretName('pushToken') === false && secretName('pushToken', true) === true);
	check('an "enc" tail counts only at a word edge', secretName('keyenc') === true && secretName('keyencx') === false);
}

section('redact: fingerprints');
{
	check('empty, absent and typed markers', fingerprint('') === '[redacted:empty]' && fingerprint(null) === '[redacted:absent]'
		&& fingerprint(undefined) === '[redacted:absent]');
	check('a number is fingerprinted as its text', fingerprint(12345) === '[redacted 12345…#' + (function () {
		var h = 5381; '12345'.split('').forEach(function (c) { h = ((h << 5) + h + c.charCodeAt(0)) >>> 0; }); return h.toString(16); })() + '/5]');
	check('the default head is six characters', /^\[redacted abcdef…#[0-9a-f]+\/9\]$/.test(fingerprint('abcdefghi')));
	check('a head of zero shows no character of the value', /^\[redacted …#[0-9a-f]+\/9\]$/.test(fingerprint('abcdefghi', 0)));
	check('the hash is the same whatever the head, so two sightings still match',
		fingerprint('abcdefghi', 0).split('#')[1] === fingerprint('abcdefghi', 6).split('#')[1]);
	check('two different values differ', fingerprint('aaaaaaa') !== fingerprint('aaaaaab'));
}

section('redact: the walk, the deny-list hook and the string test');
{
	const R = createRedactor();
	const src = { name: 'ok', apiKey: 'sk-or-v1-abcdefghij', cfg: { token: 'T0K3Nvalue', n: 3 },
		list: [{ salt: 'saltsalt' }, 'plain'], wrapped: { a: 1 } };
	const out = R.value(src);
	check('a secret-named scalar becomes a fingerprint', /^\[redacted sk-or-…#[0-9a-f]+\/19\]$/.test(out.apiKey));
	check('a nested one too, and the sibling is left alone', /^\[redacted T0K3Nv/.test(out.cfg.token) && out.cfg.n === 3);
	check('an array is walked', /^\[redacted /.test(out.list[0].salt) && out.list[1] === 'plain');
	check('a secret-named object is noted as present, not walked', out.wrapped === '[redacted:object]');
	check('the source is not mutated', src.apiKey === 'sk-or-v1-abcdefghij' && src.cfg.token === 'T0K3Nvalue');
	check('an already-redacted marker in a secret field is not fingerprinted again',
		R.value({ apiKey: out.apiKey }).apiKey === out.apiKey);

	const cyc = { a: 1 }; cyc.self = cyc;
	check('a cycle ends in a marker, not a stack overflow', R.value(cyc).self === '[redacted:cycle-or-deep]');
	let deep = { x: 1 }; for (let i = 0; i < 60; i++) deep = { d: deep };
	check('past forty levels the walk ends in a marker', JSON.stringify(R.value(deep)).indexOf('[redacted:cycle-or-deep]') !== -1);

	const L = createRedactor({ loose: true });
	check('loose catches camelCase names', /^\[redacted /.test(L.value({ pushToken: 'abcdefgh' }).pushToken)
		&& R.value({ pushToken: 'abcdefgh' }).pushToken === 'abcdefgh');

	const N = createRedactor({ deny: ['pass'] });
	check('a deny-listed NAME is covered at any depth',
		/^\[redacted /.test(N.value({ a: { b: { pass: 'hunter22' } } }).a.b.pass) && N.value({ passage: 'x' }).passage === 'x');
	const P = createRedactor({ deny: [['user', '*', 'phrase']] });
	const pp = P.value({ user: { u1: { phrase: 'correct horse', other: 'k' }, u2: 'z' }, phrase: 'top-level stays' });
	check('a deny-listed PATH is covered, with * for any one name', /^\[redacted /.test(pp.user.u1.phrase) && pp.user.u1.other === 'k');
	check('and the same name off the path is not', pp.phrase === 'top-level stays');
	check('a path of the wrong length does not match', P.value({ user: { phrase: 'x' } }).user.phrase === 'x');

	const eight = /^(?:[a-z]+ ){7}[a-z]+$/;
	const T = createRedactor({ test: (s) => eight.test(s) });
	const tt = T.value({ note: 'one two three four five six seven eight', short: 'one two', list: ['a b c d e f g h'] });
	check('a string the caller\'s test calls secret is fingerprinted whole', /^\[redacted one tw/.test(tt.note));
	check('a string it does not is left', tt.short === 'one two');
	check('the test sees strings inside arrays', /^\[redacted a b c/.test(tt.list[0]));
	check('the test sees keys too', /^\[redacted /.test(Object.keys(T.value({ ['one two three four five six seven eight']: 1 }))[0]));
	check('a marker is never tested again', T.text('[redacted x…#1/1]') === null);
	check('text() answers a fingerprint for a hit and null for a miss',
		/^\[redacted /.test(T.text('one two three four five six seven eight')) && T.text('fine') === null);
	check('head applies to the string test as well',
		createRedactor({ test: (s) => s === 'abcdef', head: 0 }).value({ a: 'abcdef' }).a.startsWith('[redacted …#'));
	check('a byte length counts UTF-8, not characters', byteLen('é') === 2 && byteLen('abc') === 3);
}

finish('redact');
