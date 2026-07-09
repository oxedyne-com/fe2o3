/* ============================================================
   Red — AI agent web application
   ============================================================ */
(function () {
	'use strict';

	// Render single newlines as <br> in markdown.
	if (typeof marked !== 'undefined') {
		marked.setOptions({ breaks: true });
	}

	// ── Models ─────────────────────────────────────────────────
	var MODELS = [
		{ id: 'accounts/fireworks/models/glm-5p2',     label: 'GLM-5.2' },
		{ id: 'accounts/fireworks/models/glm-5p1',     label: 'GLM-5.1' },
		{ id: 'accounts/fireworks/models/gpt-oss-120b', label: 'GPT-OSS 120B' },
		{ id: 'accounts/fireworks/models/deepseek-v4-pro', label: 'DeepSeek V4 Pro' },
		{ id: 'accounts/fireworks/models/kimi-k2p6',   label: 'Kimi K2.6' },
	];

	// ── State ──────────────────────────────────────────────────
	var mgmtWs = null;       // Management WS (auth via o3db.js)
	var chatWs = null;       // Chat WS (agent protocol at /chat)
	var username = null;
	var currentSessionId = null;
	var sessions = [];       // Cached session metadata
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
		chatWs.onopen = function () {
			console.log('Chat WS connected');
			refreshSessions();
		};
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
		var msgrx = Msg_new(e.data);
		if (!msgrx) return;
	}

	// Simple syntax parser — parse the response cmd and vals.
	function Msg_new(raw) {
		var firstSpace = raw.indexOf(' ');
		if (firstSpace < 0) {
			handleChatCmd(raw, null);
			return true;
		}
		var cmd = raw.substring(0, firstSpace);
		var rest = raw.substring(firstSpace + 1);
		var val = null;
		if (rest.charAt(0) === '"') {
			var end = 1;
			while (end < rest.length) {
				if (rest.charAt(end) === '\\') { end += 2; continue; }
				if (rest.charAt(end) === '"') break;
				end++;
			}
			val = rest.substring(1, end)
				.replace(/\\"/g, '"')
				.replace(/\\n/g, '\n')
				.replace(/\\t/g, '\t')
				.replace(/\\\\/g, '\\');
		} else if (rest.charAt(0) === '{') {
			try { val = JSON.parse(rest); } catch (e) { val = rest; }
		}
		handleChatCmd(cmd, val);
		return true;
	}

	function handleChatCmd(cmd, val) {
		if (cmd === 'text') {
			appendAssistantText(val || '');
		} else if (cmd === 'done') {
			chatSend.disabled = false;
			chatInput.disabled = false;
			chatInput.focus();
			finalizeAssistantMessage();
			hideSpinner();
			refreshSessions();
		} else if (cmd === 'error') {
			appendError(val || 'Error');
			chatSend.disabled = false;
			chatInput.disabled = false;
			hideSpinner();
		} else if (cmd === 'data') {
			if (val && typeof val === 'object') {
				if (val.sessions) {
					sessions = val.sessions;
					renderSessionList(sessions);
				} else if (val.messages) {
					renderHistory(val.messages);
				} else if (val.id && val.model) {
					// New session created.
					currentSessionId = val.id;
					sessionNameEl.textContent = val.name || 'Session';
					refreshSessions();
					clearChat();
				}
			} else if (typeof val === 'string') {
				// Data value is a JSON string — try to parse it.
				try {
					var obj = JSON.parse(val);
					if (obj && obj.sessions) {
						sessions = obj.sessions;
						renderSessionList(sessions);
					} else if (obj && obj.messages) {
						renderHistory(obj.messages);
					} else if (obj && obj.id && obj.model) {
						// New session created.
						currentSessionId = obj.id;
						sessionNameEl.textContent = obj.name || 'Session';
						refreshSessions();
						clearChat();
					}
				} catch (e) { /* not JSON */ }
			}
		} else if (cmd === 'info') {
			// Confirmation — may be session rename/close, refresh.
			if (val && val.indexOf('Session') === 0) {
				refreshSessions();
			}
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
		var content = currentAssistantDiv.querySelector('.chat-msg-content');
		content.innerHTML = marked.parse(currentAssistantText);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}

	function finalizeAssistantMessage() {
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
		showSpinner();
		var escaped = text.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
		sendChat('chat "' + escaped + '"');
	}

	// ── Spinner ────────────────────────────────────────────────
	var spinnerEl = null;

	function showSpinner() {
		if (spinnerEl) return;
		spinnerEl = document.createElement('div');
		spinnerEl.className = 'chat-spinner';
		spinnerEl.innerHTML = '<span class="chat-spinner-dot"></span>\
<span class="chat-spinner-dot"></span>\
<span class="chat-spinner-dot"></span>';
		chatOutput.appendChild(spinnerEl);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}

	function hideSpinner() {
		if (spinnerEl) {
			spinnerEl.remove();
			spinnerEl = null;
		}
	}

	// ── Session management ─────────────────────────────────────
	function refreshSessions() {
		sendChat('session_list');
	}

	function getModelLabel(modelId) {
		for (var i = 0; i < MODELS.length; i++) {
			if (MODELS[i].id === modelId) return MODELS[i].label;
		}
		return modelId.split('/').pop() || modelId;
	}

	// ── Pricing (USD per 1M tokens) ────────────────────────────
	// Source: Fireworks.ai pricing page.  These are approximate
	// and should be updated when official pricing changes.
	var PRICING = {
		'accounts/fireworks/models/glm-5p2':         { in: 0.90, out: 0.90 },
		'accounts/fireworks/models/glm-5p1':         { in: 0.90, out: 0.90 },
		'accounts/fireworks/models/gpt-oss-120b':    { in: 0.15, out: 0.15 },
		'accounts/fireworks/models/deepseek-v4-pro': { in: 0.90, out: 0.90 },
		'accounts/fireworks/models/kimi-k2p6':       { in: 0.60, out: 2.50 },
	};

	function estimateCost(modelId, promptTok, completionTok) {
		var p = PRICING[modelId];
		if (!p) return 0;
		return (promptTok / 1e6 * p.in) + (completionTok / 1e6 * p.out);
	}

	function fmtTok(n) {
		if (n >= 1000) return (n / 1000).toFixed(1) + 'k tok';
		return n + ' tok';
	}

	function renderSessionList(list) {
		// Remove any existing new-session panel first.
		var existingPanel = sessionList.querySelector('.new-session-panel');
		if (existingPanel) existingPanel.remove();

		sessionList.innerHTML = '';
		if (!Array.isArray(list)) list = [];
		list.forEach(function (s) {
			sessionList.appendChild(createSessionBox(s));
		});
	}

	function createSessionBox(s) {
		var box = document.createElement('div');
		box.className = 'session-box' + (s.id === currentSessionId ? ' active' : '');
		box.dataset.id = s.id;

		var header = document.createElement('div');
		header.className = 'session-box-header';

		var name = document.createElement('span');
		name.className = 'session-box-name';
		name.textContent = s.name || s.id;
		name.title = 'Click to rename';
		name.addEventListener('click', function (e) {
			e.stopPropagation();
			startRenameSession(s.id, name);
		});

		var closeBtn = document.createElement('button');
		closeBtn.className = 'session-box-close';
		closeBtn.textContent = '×';
		closeBtn.title = 'Archive session';
		closeBtn.addEventListener('click', function (e) {
			e.stopPropagation();
			sendChat('session_close "' + escJdat(s.id) + '"');
		});

		header.appendChild(name);
		header.appendChild(closeBtn);

		var meta = document.createElement('div');
		meta.className = 'session-box-meta';

		var modelLabel = document.createElement('span');
		modelLabel.className = 'session-box-model';
		modelLabel.textContent = getModelLabel(s.model || '');

		var ctxLabel = document.createElement('span');
		ctxLabel.className = 'session-box-ctx';
		var pt = s.prompt_tokens || 0;
		var ct = s.completion_tokens || 0;
		var totalTok = pt + ct;
		ctxLabel.textContent = totalTok > 0 ? fmtTok(totalTok) : '';

		var costLabel = document.createElement('span');
		costLabel.className = 'session-box-cost';
		var cost = estimateCost(s.model || '', pt, ct);
		if (cost > 0) {
			costLabel.textContent = '$' + cost.toFixed(4);
		}

		meta.appendChild(modelLabel);
		if (totalTok > 0) meta.appendChild(ctxLabel);
		if (cost > 0) meta.appendChild(costLabel);

		box.appendChild(header);
		box.appendChild(meta);

		box.addEventListener('click', function (e) {
			if (e.target === closeBtn) return;
			currentSessionId = s.id;
			sessionNameEl.textContent = s.name || 'Session';
			sendChat('session_switch "' + escJdat(s.id) + '"');
			updateActiveSession(s.id);
		});

		return box;
	}

	function startRenameSession(sessionId, nameEl) {
		var oldName = nameEl.textContent;
		var input = document.createElement('input');
		input.type = 'text';
		input.className = 'session-box-rename';
		input.value = oldName;
		nameEl.replaceWith(input);
		input.focus();
		input.select();

		function commit() {
			var newName = input.value.trim() || oldName;
			if (newName !== oldName) {
				sendChat('session_rename "' + escJdat(sessionId) + '" "' + escJdat(newName) + '"');
				// Update header if this is the active session.
				if (sessionId === currentSessionId) {
					sessionNameEl.textContent = newName;
				}
			}
			// Restore span immediately; refreshSessions will update.
			var span = document.createElement('span');
			span.className = 'session-box-name';
			span.textContent = newName;
			span.title = 'Click to rename';
			span.addEventListener('click', function () {
				startRenameSession(sessionId, span);
			});
			input.replaceWith(span);
		}

		input.addEventListener('blur', commit);
		input.addEventListener('keydown', function (e) {
			if (e.key === 'Enter') { e.preventDefault(); input.blur(); }
			if (e.key === 'Escape') { input.value = oldName; input.blur(); }
		});
	}

	function updateActiveSession(id) {
		sessionList.querySelectorAll('.session-box').forEach(function (box) {
			box.classList.toggle('active', box.dataset.id === id);
		});
	}

	// ── New session panel ──────────────────────────────────────
	function showNewSessionPanel() {
		// Remove any existing panel.
		var existing = sessionList.querySelector('.new-session-panel');
		if (existing) { existing.remove(); return; }

		var panel = document.createElement('div');
		panel.className = 'new-session-panel';

		// Model selector.
		var select = document.createElement('select');
		select.className = 'new-session-model';
		MODELS.forEach(function (m) {
			var opt = document.createElement('option');
			opt.value = m.id;
			opt.textContent = m.label;
			select.appendChild(opt);
		});

		// Start button.
		var startBtn = document.createElement('button');
		startBtn.className = 'new-session-start';
		startBtn.textContent = 'Start';

		startBtn.addEventListener('click', function () {
			var model = select.value;
			// Generate a default session name from the model label + timestamp.
			var label = getModelLabel(model);
			var now = new Date();
			var ts = now.getHours().toString().padStart(2, '0') + ':' +
				now.getMinutes().toString().padStart(2, '0');
			var name = label + ' ' + ts;
			sendChat('session_new "' + escJdat(name) + '" "' + escJdat(model) + '"');
			panel.remove();
		});

		panel.appendChild(select);
		panel.appendChild(startBtn);

		// Prepend to session list.
		sessionList.insertBefore(panel, sessionList.firstChild);
		select.focus();
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
		}).catch(function (e) {
			document.getElementById('register-error').textContent = 'Connection error: ' + e;
		}).then(function (r) {
			if (r && r.cmd === 'info') {
				attemptLogin(user, pass);
			} else if (r) {
				document.getElementById('register-error').textContent =
					r.cmd === 'error' ? (r.val || 'Registration failed') : 'Registration failed';
			}
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

	newSessionBtn.addEventListener('click', showNewSessionPanel);

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
