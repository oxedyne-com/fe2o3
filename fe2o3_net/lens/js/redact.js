// redact.js -- what keeps a secret out of a lens row. Pure functions, no host globals, so the
// browser writer (`writer.js`) and the node reader (`reader.mjs`) import one copy.
//
// Extracted from Daimond's `www/js/debugshare.js` and `dev/lens.mjs` (2026-10-02). Three guards,
// in the order a value meets them:
//
//   1. By NAME: a field called `apiKey`, `token`, `passphrase` ... is replaced by a fingerprint
//      whatever it holds (`SECRET_RE`, and `SECRET_RE_LOOSE` for camelCase names).
//   2. By the caller: a deny-list of names and paths, and a string test, both supplied by the
//      app, which alone knows its own secrets (Oxegen's eight-word passphrase is one).
//   3. By CONTENT: the scrubber below finds the shapes a credential wears inside free text.
//
// The name rule, the loose rule and the fingerprint are the ones `fe2o3_net::lens::redact` mirrors
// in Rust; `test/crosscheck.test.mjs` runs the JS over that crate's oracle tables.

// ── By name ──────────────────────────────────────────────────────────

// Field names whose VALUE is a secret. Matched on the key name, case-insensitively, at any depth.
export const SECRET_RE = /(?:^|[_.-])(?:apikey|api_key|key|token|secret|passphrase|password|salt|wrapped|wrappedpriv|sealed|seal|privatekey|priv|mnemonic|seed|masterkey)(?:$|[_.-]|enc\b)/i;

// `SECRET_RE` only fires on a separator or the start of the name, so a camelCase-joined field
// (`pushToken`, `refreshSecret`) slides past it. This companion fires anywhere the word appears,
// with `key` anchored to the end of the name. It is opt-in, because a bare "token" match would
// also take a field such as `tokenStats`.
export const SECRET_RE_LOOSE = /token|secret|passphrase|password|key$/i;

/// Does this field name look like a secret? `loose` adds `SECRET_RE_LOOSE`.
export function secretName(name, loose) {
	var n = String(name);
	return SECRET_RE.test(n) || (!!loose && SECRET_RE_LOOSE.test(n));
}

/// A short, non-reversible rendering of a secret for a person's eye: its first `head` characters
/// (six by default; none is safer for a passphrase), a stable hash and its length. Enough to tell
/// two values apart and nothing an attacker could run with. A non-string is rendered as text
/// first, and an absent one is a typed marker.
export function fingerprint(v, head) {
	if (v == null) return '[redacted:absent]';
	if (typeof v !== 'string') {
		try { v = String(v); } catch (e) { return '[redacted:' + typeof v + ']'; }
	}
	if (v === '') return '[redacted:empty]';
	var shown = v.slice(0, head == null ? 6 : head);
	// djb2, as unsigned hex. It only has to separate two values stably; the raw one never travels.
	var h = 5381;
	for (var i = 0; i < v.length; i++) { h = ((h << 5) + h + v.charCodeAt(i)) >>> 0; }
	return '[redacted ' + shown + '…#' + h.toString(16) + '/' + v.length + ']';
}

/// The UTF-8 byte length of a string, which is what a transport pays and so what a cap counts.
export function byteLen(s) {
	try { return new TextEncoder().encode(s).length; }
	catch (e) {
		try { return unescape(encodeURIComponent(s)).length; }
		catch (e2) { return s.length; }
	}
}

// ── By content ───────────────────────────────────────────────────────

// ── THE CONTENT SCRUBBER (SHARED BLOCK) ──────────────────────
//
// Lifted from Daimond's `debugshare.js` and `dev/lens.mjs`, where it stood twice and a test held
// the copies together; here it is the one copy, imported by the writer and the reader alike.
// It stays in the `var` and `function` dialect of the classic scripts it came from, so a caller
// that still wants a text copy can cut it at the two sentinel lines.
//
// WHY IT EXISTS. `redact`/`redactConfig` match a field NAME. That is the whole
// guarantee for `config`, and it is worth nothing for a credential sitting in FREE
// TEXT: a console line that printed one, a tile of the daimon's own answer quoting
// one back, a failed fetch whose URL carries `?token=`, a tool argument, a stack
// frame. So this matches by CONTENT. Every string in a payload is searched for the
// shapes a credential actually wears, and each hit is replaced by a marker naming
// the shape, a stable hash and the length -- so a reader still sees that a key was
// there, and can tell two occurrences apart, while the value never leaves the
// device.
//
// AND A CATCH-ALL BEHIND THEM. A shape nobody has enumerated is still a long,
// unbroken, high-entropy run of characters, and nothing the feed is meant to carry
// looks like that: a digest, a build id and a device id are hex, which tops out at
// exactly 4 bits a character, and ordinary prose and identifiers are far below it.
// So a run of `SCRUB_MIN_RUN` or more `[A-Za-z0-9_-]` whose Shannon entropy is
// above `SCRUB_MIN_BITS` bits a character, and which is not one of the shapes
// `scrubSafeRun` names, goes the same way.

var SCRUB_MIN_RUN   = 32;		// shortest unbroken run the entropy catch considers
var SCRUB_MIN_BITS  = 4.0;		// ... and the bits per character above which it fires
var SCRUB_MAX_DEPTH = 40;		// the walk's bound, as in `redact` beside it

// The shapes, each matched whole and replaced whole.
var SCRUB_SHAPES = [
	// A PEM private key -- whole, or clipped by elision, in which case everything
	// from the header to the end of the string goes.
	{ k: 'pem',    re: /-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY-----|$)/g },
	{ k: 'gh',     re: /\bgh[pousr]_[A-Za-z0-9]{16,}/g },
	{ k: 'ghpat',  re: /\bgithub_pat_[A-Za-z0-9_]{20,}/g },
	{ k: 'stripe', re: /\b[sr]k_(?:live|test)_[A-Za-z0-9]{10,}/g },
	{ k: 'whsec',  re: /\bwhsec_[A-Za-z0-9]{16,}/g },
	// `sk-`, `sk-or-v1-`, `sk-ant-api03-`: one rule, because every provider that
	// took the prefix kept the same alphabet after it.
	{ k: 'sk',     re: /\bsk-[A-Za-z0-9_-]{16,}/g },
	{ k: 'aws',    re: /\b(?:AKIA|ASIA|ABIA|ACCA|AGPA|AIDA|AIPA|ANPA|ANVA|AROA|APKA)[0-9A-Z]{12,}/g },
	{ k: 'gcp',    re: /\bAIza[0-9A-Za-z_-]{30,}/g },
	{ k: 'slack',  re: /\bxox[abeprs]-[A-Za-z0-9-]{10,}/g },
	{ k: 'jwt',    re: /\beyJ[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{4,}/g },
	// The tune relay mints one of these per run and the page carries it as its
	// provider key (`dev/tune/run.mjs`), so it reaches the feed by exactly the path
	// a real provider key would.
	{ k: 'tune',   re: /\btune-[0-9a-f]{32}/g },
];

// The shapes where the NAME must survive and only the value goes. A scrubbed
// `?token=` is still a legible URL, and a console line with the value cut out of it
// still says what the code was doing -- which fingerprinting the whole line, as
// `looksSecret` did, did not.
var SCRUB_PAIRS = [
	// An `Authorization` header, however it was spelled into the text.
	{ k: 'bearer', g: 3, re: /\b(Bearer|Basic|Token)(\s+)([A-Za-z0-9._~+/=-]{8,})/g },
	// A credential in a URL's query or fragment. `[` and `]` are excluded from the
	// value so a marker already placed here is not taken for a value and marked a
	// second time: the lens runs this block AGAIN on ingest, over output the client
	// has already scrubbed, and a rule that is not idempotent nests its own markers.
	{ k: 'urlarg', g: 2, re: /([?&#](?:access_token|refresh_token|id_token|token|api[_-]?key|apikey|key|auth|secret|password|passwd|sig|signature)=)([^&\s"'#<>\[\]]{4,})/gi },
	// A secret NAME sitting against a value. This is the shape `looksSecret` used
	// to answer yes or no about; answering with the value cut out keeps the message
	// readable instead.
	{ k: 'named',  g: 2, re: /((?:api[_-]?key|apikey|auth[_-]?token|access[_-]?token|token|secret|passphrase|password|master[_-]?key|private[_-]?key|mnemonic|seed[_-]?phrase|salt|sealed|wrapped)["'\s]{0,3}[:=]["'\s]{0,3})([A-Za-z0-9_\-./+=]{8,})/gi },
	// An AWS secret is forty characters of base64 wearing no prefix at all, so it
	// is only recognisable NEXT TO the access-key id or the name it belongs to.
	{ k: 'awssec', g: 2, re: /((?:AKIA|ASIA)[0-9A-Z]{12,}[\s\S]{0,200}?["'\s:=,])([A-Za-z0-9/+=]{40})(?![A-Za-z0-9/+=])/g },
	{ k: 'awssec', g: 2, re: /((?:aws)?_?secret_?access_?key["'\s]{0,3}[:=]["'\s]{0,3})([A-Za-z0-9/+=]{40})(?![A-Za-z0-9/+=])/gi },
];

// Built from the constant rather than repeating it, so the two cannot drift.
var SCRUB_RUN_RE = new RegExp('[A-Za-z0-9_-]{' + SCRUB_MIN_RUN + ',}', 'g');

/// A short, non-reversible marker for one scrubbed value: the shape it wore, a
/// stable hash and its length. The hash is what lets a reader say "the same thing
/// again" across two events without the value travelling; it is djb2, the one
/// `fingerprint` already uses, because the two markers sit side by side in a log
/// and a reader should not have to learn both.
function scrubMark(kind, v) {
	var h = 5381;
	for (var i = 0; i < v.length; i++) { h = ((h << 5) + h + v.charCodeAt(i)) >>> 0; }
	return '[redacted ' + kind + ' #' + h.toString(16) + '/' + v.length + ']';
}

/// Shannon entropy of a string, in bits per character. Hex tops out at 4.0 by
/// construction and base64url at 6.0, so the threshold sits exactly where a digest
/// stops and a key begins.
function scrubEntropy(s) {
	var counts = {}, i, c;
	for (i = 0; i < s.length; i++) {
		c = s.charAt(i);
		counts[c] = (counts[c] || 0) + 1;
	}
	var keys = Object.keys(counts), h = 0, p;
	for (i = 0; i < keys.length; i++) {
		p = counts[keys[i]] / s.length;
		h -= p * (Math.log(p) / Math.LN2);
	}
	return h;
}

/// Is this unbroken run one of the shapes the feed is SUPPOSED to carry? Named
/// explicitly rather than left to the entropy threshold, so the feed stays legible
/// by rule and not by luck. Measured against the real archive rather than guessed
/// at: a sha256 digest, a build id and a device id are hex; a tool call's
/// correlation id and a message id are hex joined by `-` or `_`, which is why the
/// separators are stripped before the hex test rather than tested around; a count
/// or a stamp is decimal; and an identifier built out of words and a number
/// (`worker_pool_seat_000412`) is every segment a word or a number and nothing else.
function scrubSafeRun(s) {
	var bare = s.replace(/[-_]/g, '');
	if (/^[0-9a-fA-F]+$/.test(bare)) return true;	// digests, build ids, device ids, UUIDs
	if (/^[0-9]+$/.test(bare)) return true;			// counts, stamps, sequence numbers
	if (/^[A-Za-z]+$/.test(bare)) return true;		// identifiers and runs of prose
	// Every `-`/`_` separated segment a word or a number: an identifier a person
	// wrote, not a value a generator produced.
	var parts = s.split(/[-_]/), i;
	if (parts.length > 1) {
		for (i = 0; i < parts.length; i++) {
			if (!/^(?:[A-Za-z]{2,}|[0-9]+)$/.test(parts[i])) return false;
		}
		return true;
	}
	return false;
}

// The fields whose VALUE is a correlation id the reader navigates by -- a tool
// call matched to its result, a turn to its rounds, a device to its events. A
// provider mints some of them out of an alphabet indistinguishable from a key's,
// so the entropy catch alone would take the feed's whole index with it. They are
// exempt from THAT catch and from nothing else: an `id` holding an `sk-` key is
// still scrubbed by the shape rules, which run first and do not consult this.
var SCRUB_ID_KEYS = {
	id: 1, mid: 1, callId: 1, call_id: 1, tool_call_id: 1, toolCallId: 1,
	turn: 1, chat: 1, chatId: 1, device: 1, deviceId: 1, build: 1, d: 1, b: 1, n: 1, w: 1,
	// Daimond added `holder`, `self`, `nominated` and `body` (its archive's snapshot filename);
	// a caller adds its own through `createScrubber({ idKeys })`.
};

/// One string, with every credential shape in it replaced by a marker. Returns the
/// string unchanged when it holds none -- which is the overwhelmingly common case,
/// and the one the per-tick cost is measured on. `isId` suppresses the entropy
/// catch alone, for the correlation-id fields `SCRUB_ID_KEYS` names.
function scrubText(s, isId, cfg) {
	if (!s || s.length < 8) return s;
	var i, r;
	var shapes = (cfg && cfg.shapes) ? SCRUB_SHAPES.concat(cfg.shapes) : SCRUB_SHAPES;
	var pairs  = (cfg && cfg.pairs)  ? SCRUB_PAIRS.concat(cfg.pairs)   : SCRUB_PAIRS;
	for (i = 0; i < shapes.length; i++) {
		r = shapes[i];
		s = s.replace(r.re, (function (kind) {
			return function (m) { return scrubMark(kind, m); };
		})(r.k));
	}
	for (i = 0; i < pairs.length; i++) {
		r = pairs[i];
		s = s.replace(r.re, (function (rule) {
			return function () {
				var args = arguments, keep = '', j;
				for (j = 1; j < rule.g; j++) keep += (args[j] == null ? '' : args[j]);
				return keep + scrubMark(rule.k, String(args[rule.g]));
			};
		})(r));
	}
	// The catch-all runs LAST, so a run already turned into a marker above is not
	// weighed a second time.
	if (isId) return s;
	s = s.replace(SCRUB_RUN_RE, function (m) {
		if (scrubSafeRun(m)) return m;
		if (scrubEntropy(m) <= SCRUB_MIN_BITS) return m;
		return scrubMark('hi', m);
	});
	return s;
}

/// A deep copy of a payload with `scrubText` applied to every string in it, keys
/// included -- a key can be a chat id or a model name, and nothing stops a future
/// one being a token. Cycle-safe and depth-bounded like the redactors beside it,
/// since what it walks is arbitrary state.
function scrubDeep(obj, seen, depth, isId, cfg) {
	seen = seen || [];
	depth = depth || 0;
	if (typeof obj === 'string') return scrubText(obj, isId, cfg);
	if (obj == null || typeof obj !== 'object') return obj;
	if (depth > SCRUB_MAX_DEPTH || seen.indexOf(obj) !== -1) return '[scrubbed:cycle-or-deep]';
	seen = seen.concat([obj]);
	var i;
	if (Array.isArray(obj)) {
		var arr = [];
		// An array inherits its parent's field name: `callIds: [...]` is still ids.
		for (i = 0; i < obj.length; i++) arr[i] = scrubDeep(obj[i], seen, depth + 1, isId, cfg);
		return arr;
	}
	var out = {}, keys = Object.keys(obj);
	for (i = 0; i < keys.length; i++) {
		out[scrubText(keys[i], false, cfg)] = scrubDeep(obj[keys[i]], seen, depth + 1,
			SCRUB_ID_KEYS[keys[i]] === 1 || !!(cfg && cfg.idKeys && cfg.idKeys[keys[i]] === 1), cfg);
	}
	return out;
}
// ── END OF THE SHARED SCRUBBER BLOCK ─────────────────────────

/// A scrubber with the caller's additions: `shapes` and `pairs` (rules in the form of
/// `SCRUB_SHAPES` and `SCRUB_PAIRS`, where a pair's `g` is the group holding the value) and
/// `idKeys` (field names whose values are correlation ids, exempt from the entropy catch).
/// Everything the base carries still applies. Returns `{ text(s, isId), deep(obj) }`.
export function createScrubber(cfg) {
	cfg = cfg || {};
	var c = { shapes: cfg.shapes || null, pairs: cfg.pairs || null, idKeys: cfg.idKeys || null };
	return {
		text: function (s, isId) { return scrubText(s, isId, c); },
		deep: function (obj) { return scrubDeep(obj, undefined, undefined, undefined, c); },
	};
}

export {
	SCRUB_ID_KEYS,
	SCRUB_PAIRS,
	SCRUB_SHAPES,
	scrubDeep,
	scrubEntropy,
	scrubMark,
	scrubSafeRun,
	scrubText,
};

// ── The word-run test ────────────────────────────────────────────────

/// A test over a caller's word list: `n` of its words in a row (eight unless told) are a secret.
///
/// An app whose passphrases are drawn from a list of words (Oxegen's are eight words of the EFF large
/// list) cannot name the secret by shape, since nothing about a word is secret, but a string in which
/// `n` of the list's words stand together is a passphrase or near enough to one that no row may keep
/// it. A word is a run of ASCII letters, whatever separates it from the next (a space, a hyphen, a
/// comma, a line break, a digit, a letter outside ASCII) and in any case; a run of letters that is not
/// on the list breaks the run. A list entry may hold hyphens (the EFF list has `drop-down`, `felt-tip`,
/// `t-shirt` and `yo-yo`): where the text spells such an entry, hyphens and all, it is the one word it
/// is, and the longest entry wins. Splitting on every non-letter cannot see those four, and breaks a
/// passphrase that holds one into runs shorter than eight. An entry whose every part is a list word
/// counts for its parts, since a dashed passphrase joins its words with the same hyphen.
///
/// `words` is an iterable of words, or a function that answers one (or null while the list is not
/// there yet, when the guard finds nothing). Returns `{ spans, has, mask, deep }`:
///
///   spans(s)         the `[start, end)` of each stretch of `n` words or more, from the first word's
///                    first letter to the last word's last, with what stood between the words;
///   has(s)           is there one;
///   mask(s, mark)    `s` with each such stretch replaced by `mark(stretch)`, which must show nothing of it;
///   deep(v, mark)    a deep copy of any JSON-shaped value with every string, and every key, masked.
///
/// The Rust half is `fe2o3_net::lens::Phrase`; `tests/data/lens_phrase.tsv` holds the two to the same
/// answers.
export function createRunGuard(words, n) {
	var RUN = n == null ? 8 : Math.max(1, n | 0);
	var set = null, parts = 1;

	function list() {
		if (set) return set;
		var w = null;
		try { w = typeof words === 'function' ? words() : words; } catch (e) { w = null; }
		// An array has a length, a Set does not: either may serve, and a list of nothing is no list.
		if (w && (w.length === undefined || w.length > 0)) {
			var s = new Set();
			w.forEach(function (x) {
				x = String(x).trim().toLowerCase();
				if (x) { s.add(x); parts = Math.max(parts, x.split('-').length); }
			});
			set = s;
		}
		return set;
	}

	// The word starting at token `i`, as [tokens it takes up, words it counts for], or [0, 0]. The longest
	// hyphen-joined entry the text spells wins; it counts for one word, or for as many as it has parts
	// where each part is itself a word, since hyphens also join the words of a dashed passphrase and the
	// safer count is the larger.
	function wordAt(s, t, i) {
		var chain = 1, k, j, each;
		while (chain < parts && i + chain < t.length && s.slice(t[i + chain - 1][1], t[i + chain][0]) === '-') chain++;
		for (k = chain; k >= 2; k--) {
			if (set.has(s.slice(t[i][0], t[i + k - 1][1]).toLowerCase())) {
				each = true;
				for (j = i; j < i + k; j++) if (!set.has(s.slice(t[j][0], t[j][1]).toLowerCase())) { each = false; break; }
				return [k, each ? k : 1];
			}
		}
		return set.has(s.slice(t[i][0], t[i][1]).toLowerCase()) ? [1, 1] : [0, 0];
	}

	function spans(s) {
		if (!list() || typeof s !== 'string' || s.length < RUN * 2 - 1) return [];
		var t = [], re = /[A-Za-z]+/g, m;
		while ((m = re.exec(s)) !== null) t.push([m.index, m.index + m[0].length]);
		var out = [], from = 0, to = 0, run = 0, i = 0, w, took;
		while (i < t.length) {
			w = wordAt(s, t, i);
			took = w[0];
			if (!took) {
				if (run >= RUN) out.push([from, to]);
				run = 0; i += 1;
				continue;
			}
			if (!run) from = t[i][0];
			to = t[i + took - 1][1];
			run += w[1]; i += took;
		}
		if (run >= RUN) out.push([from, to]);
		return out;
	}

	function has(s) { return spans(s).length > 0; }

	function mask(s, mark) {
		var sp = spans(s);
		if (!sp.length) return s;
		var out = '', at = 0;
		sp.forEach(function (x) { out += s.slice(at, x[0]) + mark(s.slice(x[0], x[1])); at = x[1]; });
		return out + s.slice(at);
	}

	function deep(v, mark, d) {
		d = d || 0;
		if (typeof v === 'string') return mask(v, mark);
		if (v == null || typeof v !== 'object' || d > 40) return v;
		if (Array.isArray(v)) return v.map(function (x) { return deep(x, mark, d + 1); });
		var o = {};
		Object.keys(v).forEach(function (k) { o[mask(k, mark)] = deep(v[k], mark, d + 1); });
		return o;
	}

	return { spans: spans, has: has, mask: mask, deep: deep };
}

// ── The redactor: names, the caller's hooks, and the walk ────────────

var MAX_DEPTH = 40;
var DEEP      = '[redacted:cycle-or-deep]';
var OBJECT    = '[redacted:object]';

// Is this already a fingerprint, so that fingerprinting it again would be a fingerprint of one?
function isMarker(s) {
	return typeof s === 'string' && s.indexOf('[redacted') === 0 && s.charAt(s.length - 1) === ']';
}

/// A redactor over any JSON-shaped value. Options, all optional:
///
///   deny   an array of names (a string, matched at any depth) and paths (an array of names,
///          where `*` stands for any one name, matched from the root);
///   test   a function `string -> bool`; a string it calls secret is replaced whole, as are keys;
///   head   the characters of a value a fingerprint shows (six, or 0 for a passphrase);
///   loose  also apply `SECRET_RE_LOOSE` (for a free-form config object).
///
/// `value(x)` returns a deep copy and never mutates; `text(s)` answers a fingerprint when the
/// caller's test calls `s` secret and null otherwise. The walk is cycle-safe and stops at forty
/// levels, as the Rust `lens::Redact` does.
export function createRedactor(opts) {
	opts = opts || {};
	var head  = opts.head == null ? 6 : opts.head;
	var loose = !!opts.loose;
	var test  = typeof opts.test === 'function' ? opts.test : null;
	var names = [], paths = [];
	(opts.deny || []).forEach(function (d) {
		if (Array.isArray(d)) paths.push(d.map(String)); else names.push(String(d));
	});

	function denied(path) {
		if (names.length && names.indexOf(path[path.length - 1]) !== -1) return true;
		for (var i = 0; i < paths.length; i++) {
			var p = paths[i];
			if (p.length !== path.length) continue;
			var all = true;
			for (var j = 0; j < p.length; j++) { if (p[j] !== '*' && p[j] !== path[j]) { all = false; break; } }
			if (all) return true;
		}
		return false;
	}

	function fp(v) { return fingerprint(v, head); }

	// A secret-named or denied field: fingerprint a scalar, note a structured value as present.
	function cover(v) {
		if (v != null && typeof v === 'object') return OBJECT;
		if (isMarker(v)) return v;
		return fp(v);
	}

	function key(k) { return (test && !isMarker(k) && test(k)) ? fp(k) : k; }

	function value(v, path, seen, depth) {
		if (typeof v === 'string') return (test && !isMarker(v) && test(v)) ? fp(v) : v;
		if (v == null || typeof v !== 'object') return v;
		if (depth > MAX_DEPTH || seen.indexOf(v) !== -1) return DEEP;
		seen = seen.concat([v]);
		if (Array.isArray(v)) return v.map(function (x) { return value(x, path, seen, depth + 1); });
		var out = {};
		Object.keys(v).forEach(function (k) {
			var nk = key(k);
			while (Object.prototype.hasOwnProperty.call(out, nk)) nk += '~';
			path.push(k);
			out[nk] = (secretName(k, loose) || denied(path)) ? cover(v[k]) : value(v[k], path, seen, depth + 1);
			path.pop();
		});
		return out;
	}

	return {
		value:       function (x) { return value(x, [], [], 0); },
		text:        function (s) { return (test && typeof s === 'string' && !isMarker(s) && test(s)) ? fp(s) : null; },
		fingerprint: fp,
		secret:      function (name) { return secretName(name, loose); },
	};
}
