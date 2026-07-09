/* ============================================================
   Ozone database client for Steel WebSocket commands.

   Exposes window.O3db with methods that send syntax commands
   to the Steel server and parse the typed responses. All values
   are stored as jdat strings (stringified JSON); the client
   serialises/deserialises transparently.

   Load this script before app.js.
   ============================================================ */
(function () {
	'use strict';

	var ws      = null;
	var pending = null; // { resolve, reject, timer }

	/* --- Wire helpers --- */

	function escJdat(s) {
		return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
	}

	function parseResponse(raw) {
		// Server responses are syntax messages: `data <value>`, `info <str>`,
		// `error <str>`.  We inspect the first word to decide how to handle.
		var first = raw.indexOf(' ');
		if (first < 0) return { cmd: raw, val: null };
		var cmd = raw.substring(0, first);
		var rest = raw.substring(first + 1);
		if (cmd === 'error') return { cmd: 'error', val: rest };
		if (cmd === 'data') {
			// The value is a jdat literal.  For strings it is `"..."`;
			// for maps it is `{...}` or `(map|{...})`.  For our usage
			// (JSON-in-a-string) we extract the quoted content.
			if (rest.charAt(0) === '"') {
				var inner = rest.slice(1, -1)
					.replace(/\\"/g, '"')
					.replace(/\\\\/g, '\\');
				try { return { cmd: 'data', val: JSON.parse(inner) }; }
				catch (e) { return { cmd: 'data', val: inner }; }
			}
			// Non-string data (maps, empty, etc.) returned as-is.
			return { cmd: 'data', val: rest };
		}
		return { cmd: cmd, val: rest };
	}

	/* --- Transport --- */

	function connect(url) {
		return new Promise(function (resolve, reject) {
			ws = new WebSocket(url);
			ws.addEventListener('open', function () { resolve(); });
			ws.addEventListener('error', function (e) { reject(e); });
			ws.addEventListener('message', onMessage);
			ws.addEventListener('close', function () { ws = null; });
		});
	}

	function onMessage(e) {
		if (!pending) return;
		var p = pending;
		pending = null;
		clearTimeout(p.timer);
		p.resolve(parseResponse(e.data));
	}

	function send(cmd, timeout) {
		timeout = timeout || 5000;
		if (!ws || ws.readyState !== WebSocket.OPEN) {
			return Promise.reject(new Error('WebSocket not connected'));
		}
		return new Promise(function (resolve, reject) {
			var timer = setTimeout(function () {
				if (pending && pending.reject === reject) {
					pending = null;
					reject(new Error('O3db: reply timeout'));
				}
			}, timeout);
			pending = { resolve: resolve, reject: reject, timer: timer };
			ws.send(cmd);
		});
	}

	/* --- Session-scoped storage --- */

	function sessGet(key) {
		return send('sess_get "' + escJdat(key) + '"');
	}

	function sessPut(key, value) {
		var json = escJdat(JSON.stringify(value));
		return send('sess_put "' + escJdat(key) + '" "' + json + '"');
	}

	/* --- User-scoped storage --- */

	function userGet(key) {
		return send('user_get "' + escJdat(key) + '"');
	}

	function userPut(key, value) {
		var json = escJdat(JSON.stringify(value));
		return send('user_put "' + escJdat(key) + '" "' + json + '"');
	}

	/* --- Auth --- */

	function register(username, passphrase) {
		return send('register "' + escJdat(username) + '" "' + escJdat(passphrase) + '"');
	}

	function login(username, passphrase) {
		return send('login "' + escJdat(username) + '" "' + escJdat(passphrase) + '"');
	}

	function logout() {
		return send('logout');
	}

	function whoami() {
		return send('whoami');
	}

	/* --- Public surface --- */

	window.O3db = {
		connect:    connect,
		send:       send,       // Generic — send any syntax command string.
		sessGet:    sessGet,
		sessPut:    sessPut,
		userGet:    userGet,
		userPut:    userPut,
		register:   register,
		login:      login,
		logout:     logout,
		whoami:     whoami,
	};
})();
