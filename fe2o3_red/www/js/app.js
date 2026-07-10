/* ============================================================
   Red — AI agent web application
   ------------------------------------------------------------
   Four-panel layout (doc/red mockup): rail (projects + chats),
   center (the conversation), agents (live runs), workspace
   (file tree). Side panel widths are user-pullable via drag
   handles and persisted; the agents and workspace panels can
   be closed/opened. On mobile one panel shows at a time,
   switched with the bottom nav.
   ============================================================ */
(function () {
	'use strict';

	// Render single newlines as <br> in markdown.
	if (typeof marked !== 'undefined') {
		marked.setOptions({ breaks: true });
	}

	// ── Models ─────────────────────────────────────────────────
	// ctx = context window (tokens). Approximate; provider /models
	// endpoint or config will supply exact values later.
	var MODELS = [
		{ id: 'accounts/fireworks/models/glm-5p2',     label: 'GLM-5.2',       ctx: 1048576 },
		{ id: 'accounts/fireworks/models/glm-5p1',     label: 'GLM-5.1',       ctx: 131072 },
		{ id: 'accounts/fireworks/models/gpt-oss-120b', label: 'GPT-OSS 120B', ctx: 131072 },
		{ id: 'accounts/fireworks/models/deepseek-v4-pro', label: 'DeepSeek V4 Pro', ctx: 163840 },
		{ id: 'accounts/fireworks/models/kimi-k2p6',   label: 'Kimi K2.6',     ctx: 262144 },
	];

	function getModelCtx(modelId) {
		for (var i = 0; i < MODELS.length; i++) {
			if (MODELS[i].id === modelId) return MODELS[i].ctx || 131072;
		}
		return 131072;
	}

	function fmtCtx(n) {
		if (n >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, '') + 'M';
		if (n >= 1000) return Math.round(n / 1000) + 'k';
		return '' + n;
	}

	// ── State ──────────────────────────────────────────────────
	var mgmtWs = null;       // Management WS (auth via o3db.js)
	var chatWs = null;       // Chat WS (agent protocol at /chat)
	var username = null;
	var currentSessionId = null;
	var sessions = [];       // Cached session metadata

	// ── DOM refs ───────────────────────────────────────────────
	var loginScreen    = document.getElementById('login-screen');
	var registerScreen = document.getElementById('register-screen');
	var appEl          = document.getElementById('app');
	var sessionList    = document.getElementById('session-list');
	var newSessionBtn  = document.getElementById('new-session-btn');
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
	var brandLogo      = document.querySelector('.brand-logo');
	var topMeter       = document.getElementById('top-meter');
	var centerMeter    = document.getElementById('center-meter');
	var agentsList     = document.getElementById('agents-list');
	var agentsCount    = document.getElementById('agents-count');

	// ── Theme ──────────────────────────────────────────────────
	function initTheme() {
		var saved = localStorage.getItem('red-theme') || 'dark';
		setTheme(saved);
	}

	function setTheme(theme) {
		document.documentElement.setAttribute('data-theme', theme);
		localStorage.setItem('red-theme', theme);
		if (brandLogo) {
			brandLogo.src = theme === 'light'
				? brandLogo.dataset.light
				: brandLogo.dataset.dark;
		}
	}

	themeToggle.addEventListener('click', function () {
		var current = document.documentElement.getAttribute('data-theme');
		setTheme(current === 'dark' ? 'light' : 'dark');
	});

	// ── Panel manager ──────────────────────────────────────────
	// Side panels have a persisted, drag-adjustable width; the
	// agents and workspace panels can be closed and reopened.
	var RedPanels = (function () {
		var DEF = { rail: 220, agents: 240, work: 260 };  // Default widths
		var LIM = { rail: [160, 420], agents: [180, 480], work: [180, 560] };
		var CENTER_MIN = 300;
		var HANDLE_W = 10;

		var main = document.getElementById('main');
		var els = {
			rail:   document.getElementById('panel-rail'),
			agents: document.getElementById('panel-agents'),
			work:   document.getElementById('panel-work'),
		};
		var handles = {
			rail:   document.getElementById('handle-rail'),
			agents: document.getElementById('handle-agents'),
			work:   document.getElementById('handle-work'),
		};
		var toggles = {
			agents: document.getElementById('agents-toggle-btn'),
			work:   document.getElementById('work-toggle-btn'),
		};
		var widths = {};
		var open = { rail: true, agents: true, work: true };   // user intent
		var forced = { rail: false, agents: false, work: false }; // auto-hidden: too narrow

		function shown(name) { return open[name] && !forced[name]; }

		function clamp(name, w) {
			var lim = LIM[name];
			w = Math.max(lim[0], Math.min(lim[1], w));
			// Keep the center panel usable: cap by available space.
			var others = 0, nHandles = 0;
			['rail', 'agents', 'work'].forEach(function (k) {
				if (k !== name && shown(k)) { others += els[k].offsetWidth; nHandles++; }
			});
			nHandles++; // this panel's own handle
			var cap = main.clientWidth - others - nHandles * HANDLE_W - CENTER_MIN;
			return Math.max(lim[0], Math.min(w, cap));
		}

		function setWidth(name, w) {
			widths[name] = w;
			els[name].style.width = w + 'px';
		}

		function save() {
			for (var k in widths) localStorage.setItem('red-panel-w-' + k, widths[k]);
			localStorage.setItem('red-panel-open-agents', open.agents ? '1' : '0');
			localStorage.setItem('red-panel-open-work', open.work ? '1' : '0');
		}

		function syncToggles() {
			for (var k in toggles) {
				if (toggles[k]) toggles[k].classList.toggle('on', !!open[k]);
			}
		}

		// Drag-to-resize. The rail handle sits to the panel's right
		// (drag right = wider); the agents/work handles sit to the
		// panel's left (drag left = wider).
		function initHandle(name) {
			var h = handles[name];
			var sign = (name === 'rail') ? 1 : -1;
			var startX = 0, startW = 0, dragging = false;

			h.addEventListener('pointerdown', function (e) {
				dragging = true;
				startX = e.clientX;
				startW = widths[name];
				h.setPointerCapture(e.pointerId);
				h.classList.add('dragging');
				els[name].classList.add('nt');
				document.body.classList.add('dragging');
				e.preventDefault();
			});
			h.addEventListener('pointermove', function (e) {
				if (!dragging) return;
				setWidth(name, clamp(name, startW + sign * (e.clientX - startX)));
			});
			h.addEventListener('pointerup', function (e) {
				if (!dragging) return;
				dragging = false;
				h.releasePointerCapture(e.pointerId);
				h.classList.remove('dragging');
				els[name].classList.remove('nt');
				document.body.classList.remove('dragging');
				save();
				reflow(false);
			});
			h.addEventListener('dblclick', function () {
				setWidth(name, DEF[name]);
				save();
			});
		}

		// Apply a panel's current visibility (from open + forced) to the DOM.
		function apply(name, animate) {
			var el = els[name], h = handles[name];
			if (shown(name)) {
				h.classList.remove('closed');
				el.classList.remove('closed');
				if (animate) {
					el.classList.add('nt');
					el.style.width = '0px';
					void el.offsetWidth; // reflow
					el.classList.remove('nt');
				}
				el.style.width = widths[name] + 'px';
			} else {
				h.classList.add('closed');
				if (animate) {
					el.style.width = '0px';
					var onEnd = function () {
						el.removeEventListener('transitionend', onEnd);
						if (!shown(name)) el.classList.add('closed');
					};
					el.addEventListener('transitionend', onEnd);
				} else {
					el.style.width = '0px';
					el.classList.add('closed');
				}
			}
		}

		// Recompute which side panels fit and hide the overflow (work first,
		// then agents) so the four-panel layout never spills off-screen. On a
		// wide window nothing is forced; as it narrows, panels drop out and
		// come back when it widens again. The user's open/close intent is
		// preserved across this.
		function reflow(animate) {
			if (isMobile()) return; // mobile CSS owns the layout
			var avail = main.clientWidth;
			if (avail <= 0) return; // not laid out yet (app still hidden)
			forced.agents = false;
			forced.work = false;
			function needed() {
				var w = 0, n = 0;
				['rail', 'agents', 'work'].forEach(function (k) {
					if (shown(k)) { w += widths[k]; n++; }
				});
				return w + n * HANDLE_W + CENTER_MIN;
			}
			if (needed() > avail && open.work) forced.work = true;
			if (needed() > avail && open.agents) forced.agents = true;
			['rail', 'agents', 'work'].forEach(function (k) { apply(k, animate); });
			syncToggles();
		}

		function show(name) {
			open[name] = true;
			save();
			reflow(true);
			if (shown(name) && name === 'work' && window.RedFiles) RedFiles.onOpen();
		}

		function hide(name) {
			open[name] = false;
			save();
			reflow(true);
		}

		function toggle(name) {
			if (open[name]) hide(name); else show(name);
		}

		function init() {
			for (var k in els) {
				var w = parseInt(localStorage.getItem('red-panel-w-' + k) || '0');
				widths[k] = (w >= LIM[k][0] && w <= LIM[k][1]) ? w : DEF[k];
				setWidth(k, widths[k]);
				initHandle(k);
			}
			open.agents = localStorage.getItem('red-panel-open-agents') !== '0';
			open.work = localStorage.getItem('red-panel-open-work') !== '0';
			reflow(false);

			for (var t in toggles) {
				(function (name) {
					toggles[name].addEventListener('click', function () { toggle(name); });
				})(t);
			}
			document.querySelectorAll('.panel-close').forEach(function (btn) {
				btn.addEventListener('click', function () { hide(btn.dataset.close); });
			});

			// Keep the layout fitting as the window resizes.
			var rt = null;
			window.addEventListener('resize', function () {
				if (rt) return;
				rt = setTimeout(function () { rt = null; reflow(false); }, 80);
			});
		}

		return {
			init: init,
			toggle: toggle,
			reflow: function () { reflow(false); },
			isOpen: function (n) { return shown(n); },
		};
	})();
	window.RedPanels = RedPanels;

	// ── Mobile: one panel at a time ────────────────────────────
	var mobileMq = window.matchMedia('(max-width: 760px)');
	function isMobile() { return mobileMq.matches; }

	function mshow(name) {
		document.body.dataset.mpanel = name;
		document.querySelectorAll('#mnav button').forEach(function (b) {
			b.classList.toggle('on', b.dataset.mp === name);
		});
		if (name === 'work' && window.RedFiles) RedFiles.onOpen();
	}

	document.querySelectorAll('#mnav button').forEach(function (b) {
		b.addEventListener('click', function () { mshow(b.dataset.mp); });
	});

	// ── Management WS (auth) ───────────────────────────────────
	function connectMgmt() {
		var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
		return O3db.connect(proto + '//' + location.host + '/');
	}

	// ── Chat WS ────────────────────────────────────────────────
	var wantChat = false;        // Should the chat WS be kept alive?
	var reconnectTimer = null;
	var reconnectDelay = 500;    // Backoff, capped at 8s.
	var generating = false;      // Is an agent turn in flight?

	function connectChat() {
		wantChat = true;
		if (chatWs) { try { chatWs.close(); } catch (e) {} chatWs = null; }
		var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
		chatWs = new WebSocket(proto + '//' + location.host + '/chat');
		chatWs.onopen = function () {
			console.log('Chat WS connected');
			reconnectDelay = 500;
			refreshSessions();
			if (RedPanels.isOpen('work') && window.RedFiles) RedFiles.onOpen();
			// Restore the active session's history after a (re)connect.
			if (currentSessionId) {
				sendChat('session_switch "' + escJdat(currentSessionId) + '"');
			} else {
				renderEmptyState();
			}
		};
		chatWs.onmessage = handleChatMessage;
		chatWs.onclose = function () {
			chatWs = null;
			if (generating) { endGenerating(); }
			if (wantChat) { scheduleReconnect(); }
		};
		chatWs.onerror = function (e) { console.warn('Chat WS error:', e); };
	}

	function scheduleReconnect() {
		if (reconnectTimer) return;
		reconnectTimer = setTimeout(function () {
			reconnectTimer = null;
			if (wantChat) { connectChat(); }
		}, reconnectDelay);
		reconnectDelay = Math.min(reconnectDelay * 2, 8000);
	}

	// Stop the current agent turn.  Closing the chat WS aborts the
	// in-flight LLM stream server-side (the next token send fails and
	// the handler task ends), and onclose auto-reconnects + reloads.
	function stopGeneration() {
		if (chatWs) { try { chatWs.close(); } catch (e) {} }
		endGenerating();
	}

	function beginGenerating() {
		generating = true;
		showSpinner();
		setSendMode('stop');
		Agents.begin();
	}

	function endGenerating() {
		generating = false;
		hideSpinner();
		finalizeAssistantMessage();
		chatInput.disabled = false;
		setSendMode('send');
		Agents.end('done');
	}

	function setSendMode(mode) {
		chatSend.disabled = false;
		if (mode === 'stop') {
			chatSend.innerHTML = '■';
			chatSend.classList.add('stop');
			chatSend.title = 'Stop';
		} else {
			chatSend.innerHTML = '➤';
			chatSend.classList.remove('stop');
			chatSend.title = 'Send';
		}
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

	// Simple syntax parser — parse the response cmd and all values.
	function Msg_new(raw) {
		var firstSpace = raw.indexOf(' ');
		if (firstSpace < 0) {
			handleChatCmd(raw, []);
			return true;
		}
		var cmd = raw.substring(0, firstSpace);
		var rest = raw.substring(firstSpace + 1);
		handleChatCmd(cmd, parseValues(rest));
		return true;
	}

	// Parse a sequence of space-separated values: quoted strings,
	// JSON objects, or bare tokens.  Returns an array.
	function parseValues(rest) {
		var vals = [];
		var i = 0;
		while (i < rest.length) {
			while (i < rest.length && rest.charAt(i) === ' ') i++;
			if (i >= rest.length) break;
			var c = rest.charAt(i);
			if (c === '"') {
				var j = i + 1, s = '';
				while (j < rest.length) {
					var ch = rest.charAt(j);
					if (ch === '\\') {
						var n = rest.charAt(j + 1);
						s += (n === 'n' ? '\n' : n === 't' ? '\t' : n === 'r' ? '\r' : n);
						j += 2; continue;
					}
					if (ch === '"') break;
					s += ch; j++;
				}
				vals.push(s);
				i = j + 1;
			} else if (c === '{') {
				var depth = 0, k = i, inStr = false;
				for (; k < rest.length; k++) {
					var cc = rest.charAt(k);
					if (inStr) {
						if (cc === '\\') { k++; continue; }
						if (cc === '"') inStr = false;
					} else if (cc === '"') { inStr = true; }
					else if (cc === '{') { depth++; }
					else if (cc === '}') { depth--; if (depth === 0) { k++; break; } }
				}
				var objStr = rest.substring(i, k);
				try { vals.push(JSON.parse(objStr)); } catch (e) { vals.push(objStr); }
				i = k;
			} else {
				var sp = rest.indexOf(' ', i);
				if (sp < 0) sp = rest.length;
				vals.push(rest.substring(i, sp));
				i = sp;
			}
		}
		return vals;
	}

	function handleChatCmd(cmd, vals) {
		var val = vals[0];
		if (cmd === 'text') {
			appendAssistantText(val || '');
		} else if (cmd === 'tool_call') {
			renderToolCall(vals[0] || '', vals[1] || '');
		} else if (cmd === 'tool_result') {
			renderToolResult(vals[0] || '', vals[1] || '');
		} else if (cmd === 'fs_tree') {
			if (window.RedFiles) RedFiles.onTree(vals[0]);
		} else if (cmd === 'fs_content') {
			if (window.RedFiles) RedFiles.onContent(vals[0] || '', vals[1] || '');
		} else if (cmd === 'done') {
			endGenerating();
			chatInput.focus();
			refreshSessions();
			if (window.RedFiles) RedFiles.refresh();
		} else if (cmd === 'error') {
			appendError(val || 'Error');
			Agents.end('error');
			endGenerating();
		} else if (cmd === 'data') {
			if (val && typeof val === 'object') {
				handleDataObj(val);
			} else if (typeof val === 'string') {
				// Data value is a JSON string — try to parse it.
				try { handleDataObj(JSON.parse(val)); } catch (e) { /* not JSON */ }
			}
		} else if (cmd === 'info') {
			// Confirmation — may be session rename/close, refresh.
			if (val && val.indexOf('Session') === 0) {
				refreshSessions();
			}
			console.log('Info:', val);
		}
	}

	function handleDataObj(obj) {
		if (!obj) return;
		if (obj.sessions) {
			sessions = obj.sessions;
			renderSessionList(sessions);
		} else if (obj.messages) {
			renderHistory(obj.messages);
		} else if (obj.id && obj.model) {
			// New session created.
			currentSessionId = obj.id;
			sessionNameEl.textContent = obj.name || 'Session';
			refreshSessions();
			clearChat();
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
		content.innerHTML = RedRender.md(currentAssistantText);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}

	function finalizeAssistantMessage() {
		if (currentAssistantDiv && currentAssistantText) {
			var content = currentAssistantDiv.querySelector('.chat-msg-content');
			content.innerHTML = RedRender.md(currentAssistantText);
		}
		currentAssistantDiv = null;
		currentAssistantText = '';
	}

	// Inline tool-call rendering: a collapsible block showing the tool
	// name, its JSON args, and (once it returns) its result.
	var lastToolBlock = null;

	function renderToolCall(name, args) {
		finalizeAssistantMessage(); // close any open text bubble
		var block = document.createElement('div');
		block.className = 'tool-block running collapsed';
		var head = document.createElement('div');
		head.className = 'tool-head';
		head.textContent = '\u{1F527} ' + name;
		head.addEventListener('click', function () { block.classList.toggle('collapsed'); });
		var argsPre = document.createElement('pre');
		argsPre.className = 'tool-args';
		argsPre.textContent = typeof args === 'string' ? args : JSON.stringify(args);
		var resPre = document.createElement('pre');
		resPre.className = 'tool-result';
		resPre.style.display = 'none';
		block.appendChild(head);
		block.appendChild(argsPre);
		block.appendChild(resPre);
		chatOutput.appendChild(block);
		lastToolBlock = block;
		chatOutput.scrollTop = chatOutput.scrollHeight;
		Agents.tool(name, block);
	}

	function renderToolResult(name, result) {
		if (lastToolBlock) {
			lastToolBlock.classList.remove('running');
			var resPre = lastToolBlock.querySelector('.tool-result');
			resPre.textContent = result;
			resPre.style.display = '';
		}
		chatOutput.scrollTop = chatOutput.scrollHeight;
		Agents.toolDone();
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

	// Welcome / empty state shown when no session is active.
	function renderEmptyState() {
		clearChat();
		var wrap = document.createElement('div');
		wrap.className = 'empty-state';
		wrap.innerHTML =
			'<img class="empty-logo" src="assets/oxedyne_red.svg" alt="">' +
			'<h2>Welcome to Red</h2>' +
			'<p>Your workspace for chat and coding. Start a new chat to begin.</p>';
		var btn = document.createElement('button');
		btn.className = 'empty-new-session';
		btn.textContent = '+ New chat';
		btn.addEventListener('click', function () {
			if (isMobile()) mshow('rail');
			showNewSessionPanel();
		});
		wrap.appendChild(btn);
		chatOutput.appendChild(wrap);
	}

	// Relative time from a unix-seconds timestamp.
	function fmtRelTime(sec) {
		if (!sec) return '';
		var d = Math.floor(Date.now() / 1000) - sec;
		if (d < 60) return 'just now';
		if (d < 3600) return Math.floor(d / 60) + 'm ago';
		if (d < 86400) return Math.floor(d / 3600) + 'h ago';
		return Math.floor(d / 86400) + 'd ago';
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
		if (generating) return;
		var text = chatInput.value.trim();
		if (!text) return;
		if (!currentSessionId) {
			if (isMobile()) mshow('rail');
			showNewSessionPanel();
			return;
		}
		appendUserMessage(text);
		chatInput.value = '';
		chatInput.style.height = 'auto';
		chatInput.disabled = true;
		beginGenerating();
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

	function sessionById(id) {
		for (var i = 0; i < sessions.length; i++) {
			if (sessions[i].id === id) return sessions[i];
		}
		return null;
	}

	function sessionCost(s) {
		return estimateCost(s.model || '', s.prompt_tokens || 0, s.completion_tokens || 0);
	}

	// ── Meters (top bar + center header) ───────────────────────
	function updateMeters() {
		var s = sessionById(currentSessionId);
		var total = 0;
		for (var i = 0; i < sessions.length; i++) total += sessionCost(sessions[i]);
		var parts = [];
		if (s) {
			var lpt = s.last_prompt_tokens || 0;
			var pct = Math.min(100, (lpt / getModelCtx(s.model || '')) * 100);
			parts.push('session ' + (lpt > 0 ? pct.toFixed(pct < 10 ? 1 : 0) : '0') + '% ctx');
		}
		if (total > 0) parts.push('$' + total.toFixed(total < 0.1 ? 4 : 2) + (s ? '' : ' total'));
		topMeter.innerHTML = '';
		parts.forEach(function (p, idx) {
			if (idx > 0) {
				var sep = document.createElement('span');
				sep.className = 'sep';
				sep.textContent = '·';
				topMeter.appendChild(sep);
			}
			var span = document.createElement('span');
			span.textContent = p;
			topMeter.appendChild(span);
		});
		if (s) {
			var cost = sessionCost(s);
			var m = getModelLabel(s.model || '');
			var lp = s.last_prompt_tokens || 0;
			centerMeter.textContent = m
				+ (lp > 0 ? ' · ' + fmtCtx(lp) + ' / ' + fmtCtx(getModelCtx(s.model || '')) : '')
				+ (cost > 0 ? ' · $' + cost.toFixed(cost < 0.1 ? 4 : 2) : '');
		} else {
			centerMeter.textContent = '';
		}
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
		updateMeters();
		Agents.render();
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

		var timeLabel = document.createElement('span');
		timeLabel.className = 'session-box-time';
		timeLabel.textContent = fmtRelTime(s.created_at);

		if (totalTok > 0) meta.appendChild(ctxLabel);
		if (cost > 0) meta.appendChild(costLabel);
		if (timeLabel.textContent) meta.appendChild(timeLabel);

		box.appendChild(header);
		box.appendChild(meta);

		// Live context-window meter: current context usage vs the
		// model's limit.
		var lpt = s.last_prompt_tokens || 0;
		if (lpt > 0) {
			var ctx = getModelCtx(s.model || '');
			var pct = Math.min(100, (lpt / ctx) * 100);
			var meter = document.createElement('div');
			meter.className = 'ctx-meter';
			var bar = document.createElement('div');
			bar.className = 'ctx-bar';
			var fill = document.createElement('div');
			fill.className = 'ctx-fill';
			fill.style.width = pct.toFixed(1) + '%';
			if (pct > 85) fill.classList.add('high');
			bar.appendChild(fill);
			var lbl = document.createElement('span');
			lbl.className = 'ctx-meter-label';
			lbl.textContent = fmtCtx(lpt) + ' / ' + fmtCtx(ctx);
			meter.appendChild(bar);
			meter.appendChild(lbl);
			box.appendChild(meter);
		}

		box.addEventListener('click', function (e) {
			if (e.target === closeBtn) return;
			// Clicking the already-active session must not reload it —
			// that would wipe the in-progress tool/process blocks (which
			// aren't part of the persisted history).
			if (s.id === currentSessionId) {
				if (isMobile()) mshow('center');
				return;
			}
			currentSessionId = s.id;
			sessionNameEl.textContent = s.name || 'Session';
			sendChat('session_switch "' + escJdat(s.id) + '"');
			updateActiveSession(s.id);
			updateMeters();
			if (isMobile()) mshow('center');
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

	// ── Agents panel ───────────────────────────────────────────
	// With the single-agent chat loop, each in-flight turn of the
	// active chat is one run: a tile with live tool activity. The
	// run lifecycle here maps onto the brief/fold model in doc/red
	// as the multi-agent layer lands.
	var Agents = {
		runs: [],       // Newest first: {name, model, sid, status, tools:[{name, status, block}]}
		current: null,

		begin: function () {
			var s = sessionById(currentSessionId);
			this.current = {
				name: (s && s.name) || sessionNameEl.textContent || 'run',
				model: s ? getModelLabel(s.model || '') : '',
				sid: currentSessionId,
				status: 'running',
				tools: [],
			};
			this.runs.unshift(this.current);
			if (this.runs.length > 12) this.runs.pop();
			this.render();
		},

		tool: function (name, block) {
			if (!this.current) return;
			this.current.tools.push({ name: name, status: 'running', block: block });
			this.render();
		},

		toolDone: function () {
			if (!this.current) return;
			var t = this.current.tools;
			for (var i = t.length - 1; i >= 0; i--) {
				if (t[i].status === 'running') { t[i].status = 'done'; break; }
			}
			this.render();
		},

		end: function (status) {
			if (!this.current) return;
			this.current.status = status;
			this.current.tools.forEach(function (t) {
				if (t.status === 'running') t.status = 'done';
			});
			this.current = null;
			this.render();
		},

		render: function () {
			if (!agentsList) return;
			agentsList.innerHTML = '';
			var live = 0;
			var self = this;
			this.runs.forEach(function (run) {
				if (run.status === 'running') live++;
				agentsList.appendChild(self.tile(run));
			});
			agentsCount.textContent = live > 0 ? live + ' live' : '';
			if (this.runs.length === 0) {
				var empty = document.createElement('div');
				empty.className = 'agents-empty';
				empty.textContent = 'No agents running. Each turn of the active chat appears here.';
				agentsList.appendChild(empty);
			}
		},

		tile: function (run) {
			var card = document.createElement('div');
			card.className = 'acard ' + run.status;

			var ah = document.createElement('div');
			ah.className = 'ah';
			var an = document.createElement('span');
			an.className = 'an';
			an.textContent = run.name;
			var pill = document.createElement('span');
			pill.className = 'pill ' + (run.status === 'running' ? 'run' : run.status === 'error' ? 'err' : 'ok');
			pill.textContent = run.status;
			ah.appendChild(an);
			ah.appendChild(pill);
			card.appendChild(ah);

			// Context bar + cost for the run's session, live figures.
			var s = sessionById(run.sid);
			if (s) {
				var lpt = s.last_prompt_tokens || 0;
				if (lpt > 0) {
					var ctx = getModelCtx(s.model || '');
					var pct = Math.min(100, (lpt / ctx) * 100);
					var meter = document.createElement('div');
					meter.className = 'ctx-meter';
					var bar = document.createElement('div');
					bar.className = 'ctx-bar';
					var fill = document.createElement('div');
					fill.className = 'ctx-fill' + (pct > 85 ? ' high' : '');
					fill.style.width = pct.toFixed(1) + '%';
					bar.appendChild(fill);
					var lbl = document.createElement('span');
					lbl.className = 'ctx-meter-label';
					lbl.textContent = pct.toFixed(pct < 10 ? 1 : 0) + '%';
					meter.appendChild(bar);
					meter.appendChild(lbl);
					card.appendChild(meter);
				}
				var cost = sessionCost(s);
				var arow = document.createElement('div');
				arow.className = 'arow';
				var left = document.createElement('span');
				left.textContent = run.tools.length ? run.tools.length + ' tool' + (run.tools.length === 1 ? '' : 's') : '';
				var right = document.createElement('span');
				right.textContent = cost > 0 ? '$' + cost.toFixed(cost < 0.1 ? 4 : 2) : '';
				arow.appendChild(left);
				arow.appendChild(right);
				card.appendChild(arow);
			}

			// Tool rows (most recent last); clicking one scrolls to its
			// inline block in the thread.
			if (run.tools.length) {
				var wrap = document.createElement('div');
				wrap.className = 'atools';
				run.tools.slice(-8).forEach(function (t) {
					var row = document.createElement('div');
					row.className = 'atool';
					var dot = document.createElement('span');
					dot.className = t.status === 'running' ? 'live' : 'tick';
					dot.textContent = t.status === 'running' ? '●' : '✓';
					var nm = document.createElement('span');
					nm.textContent = t.name;
					row.appendChild(dot);
					row.appendChild(nm);
					row.addEventListener('click', function () {
						if (t.block && t.block.isConnected) {
							if (isMobile()) mshow('center');
							t.block.scrollIntoView({ behavior: 'smooth', block: 'center' });
						}
					});
					wrap.appendChild(row);
				});
				card.appendChild(wrap);
			}

			return card;
		},
	};

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
			if (isMobile()) mshow('center');
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
		// The app was display:none during RedPanels.init(), so the panel
		// fit couldn't be measured; recompute now that it is laid out.
		if (window.RedPanels) RedPanels.reflow();
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
		wantChat = false;
		if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
		O3db.logout().then(function () {
			username = null;
			currentSessionId = null;
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
	chatSend.addEventListener('click', function () {
		if (generating) { stopGeneration(); } else { sendUserMessage(); }
	});

	// ── Files panel ────────────────────────────────────────────
	if (window.RedFiles) { RedFiles.init(sendChat); }

	// ── Show/hide agent process (tool) blocks in the thread ────
	var toolsHidden = localStorage.getItem('red-hide-tools') === '1';
	var stepsToggleBtn = document.getElementById('steps-toggle-btn');
	function applyToolsVisibility() {
		chatOutput.classList.toggle('hide-tools', toolsHidden);
		if (stepsToggleBtn) stepsToggleBtn.classList.toggle('dim', toolsHidden);
	}
	if (stepsToggleBtn) {
		stepsToggleBtn.addEventListener('click', function () {
			toolsHidden = !toolsHidden;
			localStorage.setItem('red-hide-tools', toolsHidden ? '1' : '0');
			applyToolsVisibility();
		});
	}
	applyToolsVisibility();

	// ── Init ───────────────────────────────────────────────────
	initTheme();
	RedPanels.init();
	Agents.render();
	mshow(document.body.dataset.mpanel || 'center');
	checkSession();
})();
