/* ============================================================
   Red — AI agent web application
   ============================================================ */
(function () {
	'use strict';

	// ── State ──────────────────────────────────────────────────
	var mgmtWs = null;       // Management WS (auth via o3db.js)
	var chatWs = null;       // Chat WS (agent protocol at /chat)
	var username = null;
	var currentSessionId = null;
	var sidebarWidth = 260;

	// ── DOM refs ───────────────────────────────────────────────
	var loginScreen    = document.getElementById('login-screen');
	var registerScreen = document.getElementById('register-screen');
	var appEl          = document.getElementById('app');
	var sessionList    = document.getElementById('session-list');
	var newSessionBtn  = document.getElementById('new-session-btn');
	var sidebar        = document.getElementById('sidebar');
	var resizeHandle   = document.getElementById('resize-handle');
	var chatOutput     = document.getElementById('chat-output');
	var chatInput      = document.getElementById('chat-input');
	var chatSend       = document.getElementById('chat-send');
	var sessionNameEl  = document.getElementById('current-session-name');
	var userInfo       = document.getElementById('user-info');
	var settingsBtn    = document.getElementById('settings-btn');
	var logoutBtn      = document.getElementById('logout-btn');
	var settingsModal  = document.getElementById('settings-modal');
	var settingsClose  = document.getElementById('settings-close');
	var themeToggle    = document.getElementById('theme-toggle');
	var sidebarLogo    = document.querySelector('.sidebar-logo');

	// ── Theme ──────────────────────────────────────────────────
	function initTheme() {
		var saved = localStorage.getItem('red-theme') || 'dark';
		setTheme(saved);
	}

	function setTheme(theme) {
		document.documentElement.setAttribute('data-theme', theme);
		localStorage.setItem('red-theme', theme);
		if (sidebarLogo) {
			sidebarLogo.src = theme === 'light'
				? sidebarLogo.dataset.light
				: sidebarLogo.dataset.dark;
		}
	}

	themeToggle.addEventListener('click', function () {
		var current = document.documentElement.getAttribute('data-theme');
		setTheme(current === 'dark' ? 'light' : 'dark');
	});

	// ── Sidebar resize ─────────────────────────────────────────
	function initSidebarResize() {
		sidebarWidth = parseInt(localStorage.getItem('red-sidebar-width') || '260');
		sidebar.style.width = sidebarWidth + 'px';
		var dragging = false;
		var startX, startW;

		resizeHandle.addEventListener('mousedown', function (e) {
			dragging = true;
			startX = e.clientX;
			startW = sidebarWidth;
			resizeHandle.classList.add('dragging');
			document.body.style.cursor = 'col-resize';
			e.preventDefault();
		});

		document.addEventListener('mousemove', function (e) {
			if (!dragging) return;
			var w = startW + (e.clientX - startX);
			if (w >= 180 && w <= 500) {
				sidebarWidth = w;
				sidebar.style.width = w + 'px';
			}
		});

		document.addEventListener('mouseup', function () {
			if (dragging) {
				dragging = false;
				resizeHandle.classList.remove('dragging');
				document.body.style.cursor = '';
				localStorage.setItem('red-sidebar-width', sidebarWidth);
			}
		});

		// Double-click to toggle.
		resizeHandle.addEventListener('dblclick', function () {
			if (sidebarWidth > 0) {
				sidebar.style.width = '0px';
				sidebarWidth = 0;
			} else {
				sidebarWidth = 260;
				sidebar.style.width = '260px';
			}
			localStorage.setItem('red-sidebar-width', sidebarWidth);
		});
	}

	// ── Management WS (auth) ───────────────────────────────────
	function connectMgmt() {
		var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
		return O3db.connect(proto + '//' + location.host + '/');
	}

	// ── Chat WS ────────────────────────────────────────────────
	function connectChat() {
		if (chatWs) { chatWs.close(); chatWs = null; }
		var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
		chatWs = new WebSocket(proto + '//' + location.host + '/chat');
		chatWs.onopen = function () { console.log('Chat WS connected'); };
		chatWs.onmessage = handleChatMessage;
		chatWs.onclose = function () { chatWs = null; };
		chatWs.onerror = function (e) { console.warn('Chat WS error:', e); };
	}

	function sendChat(cmd) {
		if (chatWs && chatWs.readyState === WebSocket.OPEN) {
			chatWs.send(cmd);
		}
	}

	function handleChatMessage(e) {
		// Parse syntax response.
		var msgrx = Msg_new(e.data);
		if (!msgrx) return;
	}

	// Simple syntax parser — parse the response cmd and vals.
	function Msg_new(raw) {
		// The response is a syntax message like:
		//   data "content"
		//   info "message"
		//   error "message"
		//   text "content"
		//   done
		var firstSpace = raw.indexOf(' ');
		if (firstSpace < 0) {
			// Single-word command like "done".
			handleChatCmd(raw, null);
			return true;
		}
		var cmd = raw.substring(0, firstSpace);
		var rest = raw.substring(firstSpace + 1);
		// Try to parse the value as a JDAT string (starts with ").
		var val = null;
		if (rest.charAt(0) === '"') {
			// Extract quoted string, handling escapes.
			var end = 1;
			while (end < rest.length) {
				if (rest.charAt(end) === '\\') { end += 2; continue; }
				if (rest.charAt(end) === '"') break;
				end++;
			}
			val = rest.substring(1, end)
				.replace(/\\"/g, '"')
				.replace(/\\\\/g, '\\');
		} else if (rest.charAt(0) === '{') {
			// JDAT map — try JSON parse (close enough for our data).
			try { val = JSON.parse(rest); } catch (e) { val = rest; }
		}
		handleChatCmd(cmd, val);
		return true;
	}

	function handleChatCmd(cmd, val) {
		if (cmd === 'text') {
			// Streamed text token.
			appendAssistantText(val || '');
		} else if (cmd === 'done') {
			// Turn complete — re-enable input.
			chatSend.disabled = false;
			chatInput.disabled = false;
			chatInput.focus();
			finalizeAssistantMessage();
		} else if (cmd === 'error') {
			appendError(val || 'Error');
			chatSend.disabled = false;
			chatInput.disabled = false;
		} else if (cmd === 'data') {
			// Session data or session list.
			if (val && typeof val === 'object') {
				if (val.id) {
					// New session created.
					currentSessionId = val.id;
					sessionNameEl.textContent = val.name || 'Session';
					refreshSessions();
					clearChat();
				} else if (val.sessions) {
					renderSessionList(val.sessions);
				} else if (val.messages) {
					// Session switched — render history.
					renderHistory(val.messages);
				}
			}
		} else if (cmd === 'info') {
			// Confirmation message — ignore or log.
			console.log('Info:', val);
		}
	}

	// ── Chat rendering ─────────────────────────────────────────
	var currentAssistantDiv = null;
	var currentAssistantText = '';

	function appendUserMessage(text) {
		var div = document.createElement('div');
		div.className = 'chat-msg chat-msg-user';
		div.innerHTML = '<div class="chat-msg-content"></div>';
		div.querySelector('.chat-msg-content').textContent = text;
		chatOutput.appendChild(div);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}

	function appendAssistantText(text) {
		if (!currentAssistantDiv) {
			currentAssistantDiv = document.createElement('div');
			currentAssistantDiv.className = 'chat-msg chat-msg-assistant';
			currentAssistantDiv.innerHTML = '<div class="chat-msg-content"></div>';
			chatOutput.appendChild(currentAssistantDiv);
			currentAssistantText = '';
		}
		currentAssistantText += text;
		// Render as markdown.
		var content = currentAssistantDiv.querySelector('.chat-msg-content');
		content.innerHTML = marked.parse(currentAssistantText);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}

	function finalizeAssistantMessage() {
		// Re-render final markdown.
		if (currentAssistantDiv && currentAssistantText) {
			var content = currentAssistantDiv.querySelector('.chat-msg-content');
			content.innerHTML = marked.parse(currentAssistantText);
		}
		currentAssistantDiv = null;
		currentAssistantText = '';
	}

	function appendError(msg) {
		var div = document.createElement('div');
		div.className = 'chat-msg chat-msg-error';
		div.innerHTML = '<div class="chat-msg-content" style="color: var(--danger);"></div>';
		div.querySelector('.chat-msg-content').textContent = msg;
		chatOutput.appendChild(div);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}

	function clearChat() {
		chatOutput.innerHTML = '';
		currentAssistantDiv = null;
		currentAssistantText = '';
	}

	function renderHistory(messages) {
		clearChat();
		if (!Array.isArray(messages)) return;
		messages.forEach(function (msg) {
			if (msg.role === 'user') {
				appendUserMessage(msg.content);
			} else if (msg.role === 'assistant') {
				appendAssistantText(msg.content);
				finalizeAssistantMessage();
			}
		});
	}

	function sendUserMessage() {
		var text = chatInput.value.trim();
		if (!text || chatSend.disabled) return;
		appendUserMessage(text);
		chatInput.value = '';
		chatInput.style.height = 'auto';
		chatSend.disabled = true;
		chatInput.disabled = true;
		// Send as syntax command: chat "content"
		var escaped = text.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
		sendChat('chat "' + escaped + '"');
	}

	// ── Session management ─────────────────────────────────────
	function refreshSessions() {
		sendChat('session_list');
	}

	function renderSessionList(sessions) {
		sessionList.innerHTML = '';
		if (!Array.isArray(sessions)) sessions = [];
		sessions.forEach(function (s) {
			var item = document.createElement('div');
			item.className = 'session-item' + (s.id === currentSessionId ? ' active' : '');
			item.innerHTML = '<span class="session-name">' + escapeHtml(s.name || s.id) + '</span>' +
				'<button class="session-close" data-id="' + escapeHtml(s.id) + '">×</button>';
			item.addEventListener('click', function (e) {
				if (e.target.classList.contains('session-close')) {
					e.stopPropagation();
					sendChat('session_close "' + e.target.dataset.id + '"');
				} else {
					currentSessionId = s.id;
					sessionNameEl.textContent = s.name || 'Session';
					sendChat('session_switch "' + s.id + '"');
					updateActiveSession(s.id);
				}
			});
			sessionList.appendChild(item);
		});
	}

	function updateActiveSession(id) {
		sessionList.querySelectorAll('.session-item').forEach(function (item) {
			item.classList.toggle('active', item.querySelector('.session-close').dataset.id === id);
		});
	}

	// ── Auth ───────────────────────────────────────────────────
	function showLogin() {
		loginScreen.style.display = 'flex';
		registerScreen.style.display = 'none';
		appEl.style.display = 'none';
	}

	function showApp() {
		loginScreen.style.display = 'none';
		registerScreen.style.display = 'none';
		appEl.style.display = 'flex';
		connectMgmt().then(function () {
			connectChat();
			refreshSessions();
		}).catch(function (e) {
			console.warn('Mgmt WS connect failed:', e);
		});
	}

	function attemptLogin(user, pass) {
		connectMgmt().then(function () {
			return O3db.login(user, pass);
		}).then(function (r) {
			if (r.cmd === 'info') {
				username = user;
				userInfo.textContent = user;
				showApp();
			} else {
				document.getElementById('login-error').textContent =
					r.cmd === 'error' ? (r.val || 'Login failed') : 'Login failed';
			}
		}).catch(function (e) {
			document.getElementById('login-error').textContent = 'Connection error: ' + e;
		});
	}

	function attemptRegister(user, pass) {
		connectMgmt().then(function () {
			return O3db.register(user, pass);
		}).then(function (r) {
			if (r.cmd === 'info') {
				attemptLogin(user, pass);
			} else {
				document.getElementById('register-error').textContent =
					r.cmd === 'error' ? (r.val || 'Registration failed') : 'Registration failed';
			}
		}).catch(function (e) {
			document.getElementById('register-error').textContent = 'Connection error: ' + e;
		});
	}

	function logout() {
		O3db.logout().then(function () {
			username = null;
			if (chatWs) { chatWs.close(); chatWs = null; }
			showLogin();
		}).catch(function () { showLogin(); });
	}

	function checkSession() {
		connectMgmt().then(function () {
			return O3db.whoami();
		}).then(function (r) {
			if (r.cmd === 'data' && r.val && r.val.authenticated) {
				username = r.val.user || 'user';
				userInfo.textContent = username;
				showApp();
			} else {
				showLogin();
			}
		}).catch(function () { showLogin(); });
	}

	// ── Password change ────────────────────────────────────────
	function changePass(oldPass, newPass) {
		O3db.send('change_pass "' + escJdat(oldPass) + '" "' + escJdat(newPass) + '"').then(function (r) {
			if (r.cmd === 'info') {
				document.getElementById('change-pass-error').textContent = 'Password updated.';
				document.getElementById('change-pass-form').reset();
			} else {
				document.getElementById('change-pass-error').textContent =
					r.cmd === 'error' ? (r.val || 'Failed') : 'Failed';
			}
		}).catch(function (e) {
			document.getElementById('change-pass-error').textContent = 'Error: ' + e;
		});
	}

	// ── Helpers ────────────────────────────────────────────────
	function escapeHtml(s) {
		return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;').replace(/'/g, '&#39;');
	}

	function escJdat(s) {
		return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
	}

	// ── Event wiring ───────────────────────────────────────────
	document.getElementById('login-form').addEventListener('submit', function (e) {
		e.preventDefault();
		var user = document.getElementById('login-user').value.trim();
		var pass = document.getElementById('login-pass').value;
		document.getElementById('login-error').textContent = '';
		attemptLogin(user, pass);
	});

	document.getElementById('register-form').addEventListener('submit', function (e) {
		e.preventDefault();
		var user = document.getElementById('reg-user').value.trim();
		var pass = document.getElementById('reg-pass').value;
		var pass2 = document.getElementById('reg-pass2').value;
		document.getElementById('register-error').textContent = '';
		if (pass !== pass2) {
			document.getElementById('register-error').textContent = 'Passwords do not match';
			return;
		}
		attemptRegister(user, pass);
	});

	document.getElementById('back-to-login').addEventListener('click', function (e) {
		e.preventDefault();
		loginScreen.style.display = 'flex';
		registerScreen.style.display = 'none';
	});

	document.getElementById('show-register').addEventListener('click', function (e) {
		e.preventDefault();
		loginScreen.style.display = 'none';
		registerScreen.style.display = 'flex';
	});

	newSessionBtn.addEventListener('click', function () {
		sendChat('session_new');
	});

	logoutBtn.addEventListener('click', logout);

	settingsBtn.addEventListener('click', function () {
		settingsModal.style.display = 'flex';
	});
	settingsClose.addEventListener('click', function () {
		settingsModal.style.display = 'none';
		document.getElementById('change-pass-error').textContent = '';
	});

	document.getElementById('change-pass-form').addEventListener('submit', function (e) {
		e.preventDefault();
		var oldP = document.getElementById('old-pass').value;
		var newP = document.getElementById('new-pass').value;
		var newP2 = document.getElementById('new-pass2').value;
		document.getElementById('change-pass-error').textContent = '';
		if (newP !== newP2) {
			document.getElementById('change-pass-error').textContent = 'New passwords do not match';
			return;
		}
		changePass(oldP, newP);
	});

	// Chat input.
	chatInput.addEventListener('input', function () {
		chatInput.style.height = 'auto';
		chatInput.style.height = Math.min(chatInput.scrollHeight, 120) + 'px';
	});
	chatInput.addEventListener('keydown', function (e) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			sendUserMessage();
		}
	});
	chatSend.addEventListener('click', sendUserMessage);

	// ── Init ───────────────────────────────────────────────────
	initTheme();
	initSidebarResize();
	checkSession();
})();
