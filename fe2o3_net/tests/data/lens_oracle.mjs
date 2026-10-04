// Regenerates the two oracle tables `tests/lens.rs` reads, from the JavaScript the Rust code mirrors.
//
//   node lens_oracle.mjs <path to daimond/www/js/debugshare.js>
//
// It lifts `SECRET_RE`, `SECRET_RE_LOOSE` and `fingerprint` out of the source text, so a
// transcription error cannot hide in the Rust, and runs them over a generated set of names and
// strings. `lens_secret_re.tsv` holds `name <TAB> strict <TAB> loose`, one per line, and
// `lens_fingerprint.tsv` holds `input <TAB> output`, each JSON-encoded, because a fingerprint's
// head can hold a line break.
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const src  = readFileSync(process.argv[2], 'utf8');

const grab = (re, what) => {
	const m = re.exec(src);
	if (!m) throw new Error('not found in the source: ' + what);
	return m[0];
};
const strictLine = grab(/var SECRET_RE = \/.*\/i;/, 'SECRET_RE');
const looseLine  = grab(/var SECRET_RE_LOOSE = \/.*\/i;/, 'SECRET_RE_LOOSE');
const fpFn       = grab(/function fingerprint\(v\) \{[\s\S]*?\n\t\}/, 'fingerprint');
const mod = new Function(strictLine + '\n' + looseLine + '\n' + fpFn +
	'\nreturn { SECRET_RE: SECRET_RE, SECRET_RE_LOOSE: SECRET_RE_LOOSE, fingerprint: fingerprint };')();

const words = [
	'apikey', 'api_key', 'key', 'token', 'secret', 'passphrase', 'password', 'salt', 'wrapped',
	'wrappedpriv', 'sealed', 'seal', 'privatekey', 'priv', 'mnemonic', 'seed', 'masterkey',
	// Decoys, and the camelCase shapes the loose companion exists for.
	'keys', 'tokens', 'tokenStats', 'pushToken', 'refreshSecret', 'monkey', 'keyboard', 'apiKey',
	'ApiKey', 'API_KEY', 'Wrapped', 'sealedBox', 'oxenymKeys', 'durable', 'x',
];
const prefixes = ['', 'x', 'x_', 'x.', 'x-', 'X_', '_'];
const suffixes = ['', '_', '.', '-', 'x', 'enc', 'ENC', 'Enc', 'enc_', 'enc.', 'encx', 'enc1', 'é', '_x'];

const rows = [];
for (const w of words) for (const p of prefixes) for (const s of suffixes) {
	const name = p + w + s;
	const strict = mod.SECRET_RE.test(name);
	const loose  = strict || mod.SECRET_RE_LOOSE.test(name);
	rows.push(name + '\t' + (strict ? 1 : 0) + '\t' + (loose ? 1 : 0));
}
writeFileSync(join(HERE, 'lens_secret_re.tsv'), rows.join('\n') + '\n');

const inputs = [
	'', 'a', 'abc', 'abcdef', 'abcdefg', 'zz-not-a-key-0123456789', 'hello world',
	'correct horse battery staple', 'tab\there', 'line\nbreak', 'quote"back\\slash', 'é', 'éèêëàâ',
	'日本語のパスフレーズ', 'ab😀cd', '😀😀😀', 'x'.repeat(300), '12345', 'true',
];
const fps = inputs.map(i => JSON.stringify(i) + '\t' + JSON.stringify(mod.fingerprint(i)));
fps.push(JSON.stringify(null) + '\t' + JSON.stringify(mod.fingerprint(null)));
fps.push(JSON.stringify(12345) + '\t' + JSON.stringify(mod.fingerprint(12345)));
writeFileSync(join(HERE, 'lens_fingerprint.tsv'), fps.join('\n') + '\n');
console.log('wrote ' + rows.length + ' names and ' + fps.length + ' fingerprints');
