/* Drives the site console end to end against a real Steel.
 *
 * The console is gated on a *member* session, not the operator's, so this must
 * do what a member's browser does: get an anonymous session, sign in over the
 * WebSocket, and carry that one session's cookie into the console's HTTP pages.
 *
 * The WebSocket handshake is done by hand over the https module, rather than
 * through a library, for one reason: the session cookie the console reads is the
 * one issued on the upgrade, and only a hand-rolled upgrade lets this read the
 * Set-Cookie off the 101 and then reuse it. A browser does this for free; here it
 * is explicit.
 *
 * Env: RIG_PORT (default 9443), RIG_PASS (the member-admin's passphrase).
 */

import https from 'node:https';
import crypto from 'node:crypto';

const PORT = process.env.RIG_PORT || '9443';
const PASS = process.env.RIG_PASS || 'rig member admin passphrase not a secret';
// The vhost answers to 'localhost'; a request with any other Host is a 421. curl
// sends Host from the URL, so it never hit this -- a hand-built request must set
// it deliberately.
const HOST = 'localhost';

const norm = s => s.trim().split(/\s+/).join(' ');
const USER = crypto.createHash('sha256').update(norm(PASS)).digest('hex');

let pass = 0, fail = 0;
const ok = m => { pass++; console.log(`  PASS  ${m}`); };
const no = (m, d) => { fail++; console.log(`  FAIL  ${m}${d ? ' -- ' + d : ''}`); };
const check = (m, got, want) => got === want ? ok(m) : no(m, `expected '${want}', got '${got}'`);
const has = (m, hay, needle) => hay.includes(needle) ? ok(m) : no(m, `missing '${needle}'`);
const hasnt = (m, hay, needle) => hay.includes(needle) ? no(m, `contained '${needle}'`) : ok(m);

/* --- a plain HTTP call, self-signed cert allowed, cookie carried by hand --- */
function call(method, path, { cookie, body, form, headers: extra } = {}) {
	return new Promise((resolve, reject) => {
		const headers = Object.assign({}, extra);
		let payload = null;
		if (cookie) headers['Cookie'] = cookie;
		if (form) {
			payload = Object.entries(form)
				.map(([k, v]) => encodeURIComponent(k) + '=' + encodeURIComponent(v)).join('&');
			headers['Content-Type'] = 'application/x-www-form-urlencoded';
			headers['Content-Length'] = Buffer.byteLength(payload);
		} else if (body != null) {
			payload = body;
			headers['Content-Length'] = Buffer.byteLength(payload);
		}
		const req = https.request(
			{ host: HOST, port: PORT, path, method, headers, rejectUnauthorized: false },
			res => {
				let data = '';
				res.on('data', d => data += d);
				res.on('end', () => resolve({
					status: res.statusCode,
					headers: res.headers,
					body: data,
				}));
			});
		req.on('error', reject);
		if (payload != null) req.write(payload);
		req.end();
	});
}

/* --- the WebSocket upgrade, by hand, carrying the session cookie so login binds
   the sid this side already knows --- */
function upgrade(cookie) {
	return new Promise((resolve, reject) => {
		const key = crypto.randomBytes(16).toString('base64');
		const headers = {
			'Connection': 'Upgrade',
			'Upgrade': 'websocket',
			'Sec-WebSocket-Version': '13',
			'Sec-WebSocket-Key': key,
		};
		if (cookie) headers['Cookie'] = cookie;
		const req = https.request({
			host: HOST, port: PORT, path: '/', method: 'GET', rejectUnauthorized: false, headers,
		});
		req.on('upgrade', (_res, socket) => resolve(socket));
		// A non-upgrade response means the handshake did not happen (a 421 on a
		// bad Host, say). Reject loudly rather than let the promise hang, which
		// would empty the event loop and exit 0 as though nothing was wrong.
		req.on('response', res => reject(new Error(`upgrade got ${res.statusCode}, not 101`)));
		req.on('error', reject);
		req.end();
	});
}

/* --- client WS text frame, masked as the protocol requires of a client --- */
function frame(text) {
	const payload = Buffer.from(text, 'utf8');
	const len = payload.length; // our commands are short; no extended length needed
	const mask = crypto.randomBytes(4);
	const head = Buffer.from([0x81, 0x80 | len]);
	const masked = Buffer.alloc(len);
	for (let i = 0; i < len; i++) masked[i] = payload[i] ^ mask[i % 4];
	return Buffer.concat([head, mask, masked]);
}

/* --- read one server text frame (server frames are not masked) --- */
function readFrame(socket) {
	return new Promise((resolve, reject) => {
		let buf = Buffer.alloc(0);
		const onData = d => {
			buf = Buffer.concat([buf, d]);
			if (buf.length < 2) return;
			let len = buf[1] & 0x7f;
			let off = 2;
			if (len === 126) { if (buf.length < 4) return; len = buf.readUInt16BE(2); off = 4; }
			if (buf.length < off + len) return;
			socket.removeListener('data', onData);
			resolve(buf.slice(off, off + len).toString('utf8'));
		};
		socket.on('data', onData);
		socket.on('error', reject);
		setTimeout(() => { socket.removeListener('data', onData); reject(new Error('ws read timeout')); }, 8000);
	});
}

async function send(socket, text) {
	socket.write(frame(text));
	return readFrame(socket);
}

/* --- pull the csrf token out of an edit form --- */
function csrfOf(html) {
	const m = html.match(/name="csrf" value="([0-9a-f]+)"/);
	return m ? m[1] : null;
}

async function main() {
	console.log(`member username ${USER.slice(0, 12)}…`);

	console.log('\n== the console is closed to the anonymous ==');
	let r = await call('GET', '/manage/status');
	has('status answers the anonymous', r.body, '"admin":false');
	r = await call('GET', '/manage');
	check('the console redirects the anonymous away', r.status, 303);
	r = await call('POST', '/manage/save', { form: { slug: 'x', source: 'y', csrf: 'z' } });
	check('an anonymous write is turned away', r.status, 303);

	console.log('\n== sign in as a member over the websocket ==');
	// The console reads a member's session cookie over HTTP, so the sid must be
	// one this side knows. It is issued on a normal HTTP request, not the upgrade,
	// so: get it first, then carry it onto the upgrade so login binds it.
	const anon = await call('GET', '/');
	let cookie = null;
	for (const c of (anon.headers['set-cookie'] || [])) {
		const m = c.match(/session_id=([^;]+)/);
		if (m) cookie = 'session_id=' + m[1];
	}
	if (!cookie) { no('a normal request issued a session cookie'); finish(); return; }
	ok('a normal request issued a session cookie');
	const socket = await upgrade(cookie);
	let reply = await send(socket, `register "${USER}" "${PASS}"`);
	ok(`register replied (${reply.split('"')[0].trim() || reply.slice(0, 12)})`);
	reply = await send(socket, `login "${USER}" "${PASS}"`);
	// The authoritative proof of login is the status check below (the console reads
	// the same session); the WS reply only needs to not be a refusal.
	if (reply.startsWith('error')) no('login was refused', reply); else ok('login is not refused');
	socket.end();

	console.log('\n== a signed-in member who is not on the list learns their id ==');
	// The bootstrap: a member who is not an admin is shown their own id and told
	// to ask for it, rather than sent silently home. A second account, on no list.
	{
		const other = 'rig second member not an admin';
		const otherUser = crypto.createHash('sha256').update(norm(other)).digest('hex');
		const a2 = await call('GET', '/');
		let c2 = null;
		for (const c of (a2.headers['set-cookie'] || [])) {
			const m = c.match(/session_id=([^;]+)/);
			if (m) c2 = 'session_id=' + m[1];
		}
		const s2 = await upgrade(c2);
		await send(s2, `register "${otherUser}" "${other}"`);
		await send(s2, `login "${otherUser}" "${other}"`);
		s2.end();
		const r2 = await call('GET', '/manage', { cookie: c2 });
		check('a non-admin member is refused', r2.status, 403);
		has('but is shown their own id', r2.body, otherUser);
		has('and told what to ask for', r2.body, 'site_admins');
		hasnt('status does not call them an admin', (await call('GET', '/manage/status', { cookie: c2 })).body, '"admin":true');
	}

	console.log('\n== now a member who is on the list is an admin ==');
	r = await call('GET', '/manage/status', { cookie });
	has('status now says admin', r.body, '"admin":true');
	r = await call('GET', '/manage', { cookie });
	check('the console serves its page', r.status, 200);
	has('the page is the console', r.body, 'Posts');
	has('in the site’s own chrome', r.body, 'manage');

	console.log('\n== the app-facing JSON endpoints ==');
	// The Manage tab renders from these, and writes with the token status hands it.
	r = await call('GET', '/manage/status', { cookie });
	has('status gives an admin the csrf token', r.body, '"csrf"');
	const statusCsrf = (r.body.match(/"csrf":"([0-9a-f]+)"/) || [])[1];
	if (statusCsrf) ok('the token is a sha3 hex'); else no('no token in status');
	r = await call('GET', '/manage/list.json', { cookie });
	has('list.json returns a posts array', r.body, '"posts"');
	// A JSON write, as the app makes it: Accept application/json, token from status.
	r = await call('POST', '/manage/save', {
		cookie, headers: { Accept: 'application/json' },
		form: { slug: 'json-made', kind: 'note', state: 'draft', source: '# Via JSON\n\nx.', csrf: statusCsrf },
	});
	check('a json save answers 200, not a redirect', r.status, 200);
	has('and says ok', r.body, '"ok":true');
	r = await call('GET', '/manage/post.json?slug=json-made', { cookie });
	has('post.json returns the source to edit', r.body, 'Via JSON');
	has('and it is a draft', r.body, '"state": "draft"');
	// A json save with a bad token is a json error, not a redirect.
	r = await call('POST', '/manage/save', {
		cookie, headers: { Accept: 'application/json' },
		form: { slug: 'json-made', source: 'x', csrf: 'bad' },
	});
	check('a bad-token json write is refused as json', r.status, 403);
	has('with an error the app can read', r.body, 'error');
	await call('POST', '/manage/delete', {
		cookie, headers: { Accept: 'application/json' }, form: { slug: 'json-made', csrf: statusCsrf } });

	console.log('\n== write a post through the console ==');
	r = await call('GET', '/manage/edit', { cookie });
	const csrf = csrfOf(r.body);
	if (!csrf) { no('the editor carried a csrf token'); } else { ok('the editor carried a csrf token'); }

	// A save without the token is refused; with it, it goes through.
	r = await call('POST', '/manage/save', { cookie, form: {
		slug: 'console-made', date: '2026-07-20 09:15', kind: 'essay', state: 'live',
		source: '# Made in the console\n\nWords, and a [link](https://example.com).', csrf: 'wrong',
	}});
	check('a save with a bad token is refused (redirect, not written)', r.status, 303);
	r = await call('GET', '/posts/console-made');
	check('and nothing was written', r.status, 404);

	r = await call('POST', '/manage/save', { cookie, form: {
		slug: 'console-made', date: '2026-07-20 09:15', kind: 'essay', state: 'live',
		source: '# Made in the console\n\nWords, and a [link](https://example.com).', csrf,
	}});
	check('a save with the token redirects back', r.status, 303);

	console.log('\n== the post is live, and is what was written ==');
	r = await call('GET', '/posts/console-made');
	check('the post is served to a reader', r.status, 200);
	has('with its prose', r.body, 'Made in the console');
	has('and its Open Graph card', r.body, 'og:title');
	r = await call('GET', '/posts/index.json');
	has('the json says it is an essay', r.body, '"kind": "essay"');
	has('and dates it to the minute', r.body, '2026-07-20T09:15');
	r = await call('GET', '/posts/feed.xml');
	has('the feed dates it to the minute, not midnight', r.body, '2026-07-20T09:15:00Z');

	console.log('\n== a Djot post names a box Markdown cannot ==');
	// The whole reason Djot exists here: `:::` becomes a div, `{.class}` a span.
	r = await call('POST', '/manage/save', { cookie, headers: { Accept: 'application/json' }, form: {
		slug: 'djot-made', kind: 'note', state: 'live', markup: 'djot', csrf,
		source: '# A Djot note\n\n::: warning\nMind the gap.\n:::\n\nA [bright]{.hl} word.',
	}});
	check('a Djot save answers ok', r.status, 200);
	r = await call('GET', '/posts/djot-made');
	has('the div became a box', r.body, '<div class="warning">');
	has('the span became a styled span', r.body, '<span class="hl">');
	r = await call('GET', '/manage/post.json?slug=djot-made', { cookie });
	has('post.json reports the markup', r.body, '"markup": "djot"');
	// The live-preview endpoint renders unsaved source the same way.
	r = await call('POST', '/manage/render', { cookie, headers: { Accept: 'application/json' }, form: {
		source: '::: tip\nHello.\n:::', markup: 'djot', csrf,
	}});
	check('render answers 200', r.status, 200);
	has('and returns the rendered box', r.body, '<div class=\\"tip\\">');
	r = await call('POST', '/manage/render', { cookie, headers: { Accept: 'application/json' }, form: {
		source: '*bold* not swapped', markup: 'markdown', csrf,
	}});
	has('markdown render keeps its markers', r.body, '<em>bold</em>');
	await call('POST', '/manage/delete', { cookie, headers: { Accept: 'application/json' }, form: { slug: 'djot-made', csrf } });

	console.log('\n== a slug cannot leave its key ==');
	r = await call('POST', '/manage/save', { cookie, form: {
		slug: '../../publish/index', source: 'x', csrf,
	}});
	check('a slug with a path in it is refused', r.status, 303);
	hasnt('and wrote nothing under that key', (await call('GET', '/posts')).body, 'publish/index');

	console.log('\n== import the directory ==');
	r = await call('POST', '/manage/import', { cookie, form: { csrf } });
	check('the import redirects back', r.status, 303);
	has('the directory post is now served', (await call('GET', '/posts')).body, 'The first post');

	console.log('\n== delete a store-only post, and an import does not bring it back ==');
	// console-made was written here, not from a file, so a re-import cannot re-add
	// it. A directory post that is still a file *is* re-added -- that is import
	// doing its job, not a resurrection. The bug the store guards against is a
	// deleted key returning from a scan; a store-only post is the way to see it.
	r = await call('POST', '/manage/delete', { cookie, form: { slug: 'console-made', csrf } });
	check('the delete redirects back', r.status, 303);
	check('the store-only post is gone', (await call('GET', '/posts/console-made')).status, 404);
	await call('POST', '/manage/import', { cookie, form: { csrf } });
	const after = await call('GET', '/posts');
	hasnt('the deleted store-only post is not resurrected', after.body, 'Made in the console');
	has('and the directory post is re-added, as import should', after.body, 'The first post');

	finish();
}

function finish() {
	console.log(`\n${pass} passed, ${fail} failed`);
	process.exit(fail === 0 ? 0 : 1);
}

main().catch(e => { console.error('rig error:', e.message); process.exit(1); });
