/* ============================================================
   Red — browser-only agent UI (Stage 5a)
   ------------------------------------------------------------
   The whole application now runs in the browser: an ES module
   that drives the wasm `RedApp` (fe2o3_red compiled to wasm)
   directly, with no server. It reuses the existing four-panel
   shell, CSS and `RedRender` from the retiring server UI:
     - Rail    : chats list + new chat.
     - Center  : the conversation, streamed live.
     - Agents  : per-turn tool activity.
     - Workspace: an OPFS file tree over `run_tool`.

   Security (H5): the frontend is the whole app, so every
   interpolation of model output, file names or file contents
   is HTML-escaped, and markdown passes through the sanitiser in
   render.js. No untrusted string reaches innerHTML unescaped.

   Bring-your-own-key settings (base URL, key, model, max
   tokens) live in localStorage for now; passphrase-wrapping is
   a later hardening stage (see the TODO in red.html).
   ============================================================ */
import init, { RedApp } from '../pkg/oxedyne_fe2o3_red.js';

(function () {
	'use strict';

	if (typeof marked !== 'undefined') {
		marked.setOptions({ breaks: true });
	}

	var esc = (window.RedRender && RedRender.escapeHtml) || function (s) {
		return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;')
			.replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;');
	};

	var SYSTEM_PROMPT = 'You are Red, a helpful coding assistant running entirely '
		+ 'in the user\'s browser with an OPFS-backed workspace.';

	// ── Settings (BYOK, localStorage) ──────────────────────────
	var CFG_KEY = 'red-byok';

	function loadCfg() {
		var raw = localStorage.getItem(CFG_KEY);
		var cfg = { baseUrl: '', apiKey: '', model: '', maxTokens: 4096, tools: true };
		if (raw) {
			try {
				var j = JSON.parse(raw);
				if (typeof j.baseUrl === 'string') cfg.baseUrl = j.baseUrl;
				if (typeof j.apiKey === 'string') cfg.apiKey = j.apiKey;
				if (typeof j.model === 'string') cfg.model = j.model;
				if (typeof j.maxTokens === 'number') cfg.maxTokens = j.maxTokens;
				if (typeof j.tools === 'boolean') cfg.tools = j.tools;
			} catch (e) { /* keep defaults */ }
		}
		return cfg;
	}

	function saveCfg(cfg) {
		localStorage.setItem(CFG_KEY, JSON.stringify(cfg));
	}

	function cfgReady(cfg) {
		return !!(cfg.baseUrl && cfg.model);
	}

	// ── State ──────────────────────────────────────────────────
	var cfg = loadCfg();
	var chats = [];             // { id, name, app, messages:[{role,content}], promptTokens, completionTokens }
	var current = null;         // active chat object
	var generating = false;
	var seq = 1;

	// ── DOM refs ───────────────────────────────────────────────
	var appEl         = document.getElementById('app');
	var sessionList   = document.getElementById('session-list');
	var newSessionBtn = document.getElementById('new-session-btn');
	var chatOutput    = document.getElementById('chat-output');
	var chatInput     = document.getElementById('chat-input');
	var chatSend      = document.getElementById('chat-send');
	var sessionNameEl = document.getElementById('current-session-name');
	var settingsBtn   = document.getElementById('settings-btn');
	var settingsModal = document.getElementById('settings-modal');
	var settingsClose = document.getElementById('settings-close');
	var themeToggle   = document.getElementById('theme-toggle');
	var brandLogo     = document.querySelector('.brand-logo');
	var topMeter      = document.getElementById('top-meter');
	var centerMeter   = document.getElementById('center-meter');
	var agentsList    = document.getElementById('agents-list');
	var agentsCount   = document.getElementById('agents-count');

	// ── Theme ──────────────────────────────────────────────────
	function initTheme() { setTheme(localStorage.getItem('red-theme') || 'dark'); }
	function setTheme(theme) {
		document.documentElement.setAttribute('data-theme', theme);
		localStorage.setItem('red-theme', theme);
		if (brandLogo) {
			brandLogo.src = theme === 'light' ? brandLogo.dataset.light : brandLogo.dataset.dark;
		}
	}
	themeToggle.addEventListener('click', function () {
		var cur = document.documentElement.getAttribute('data-theme');
		setTheme(cur === 'dark' ? 'light' : 'dark');
	});

	// ── Mobile: one panel at a time ────────────────────────────
	var mobileMq = window.matchMedia('(max-width: 760px)');
	function isMobile() { return mobileMq.matches; }
	function mshow(name) {
		document.body.dataset.mpanel = name;
		document.querySelectorAll('#mnav button').forEach(function (b) {
			b.classList.toggle('on', b.dataset.mp === name);
		});
		if (name === 'work') Files.onOpen();
	}
	document.querySelectorAll('#mnav button').forEach(function (b) {
		b.addEventListener('click', function () { mshow(b.dataset.mp); });
	});

	// ── Panel manager (drag-resize + close/open) ───────────────
	var RedPanels = (function () {
		var DEF = { rail: 220, agents: 240, work: 260 };
		var LIM = { rail: [160, 420], agents: [180, 480], work: [180, 560] };
		var CENTER_MIN = 300, HANDLE_W = 10;
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
		var open = { rail: true, agents: true, work: true };
		var forced = { rail: false, agents: false, work: false };

		function shown(name) { return open[name] && !forced[name]; }

		function clamp(name, w) {
			var lim = LIM[name];
			w = Math.max(lim[0], Math.min(lim[1], w));
			var others = 0, nHandles = 0;
			['rail', 'agents', 'work'].forEach(function (k) {
				if (k !== name && shown(k)) { others += els[k].offsetWidth; nHandles++; }
			});
			nHandles++;
			var cap = main.clientWidth - others - nHandles * HANDLE_W - CENTER_MIN;
			return Math.max(lim[0], Math.min(w, cap));
		}
		function setWidth(name, w) { widths[name] = w; els[name].style.width = w + 'px'; }
		function save() {
			for (var k in widths) localStorage.setItem('red-panel-w-' + k, widths[k]);
			localStorage.setItem('red-panel-open-agents', open.agents ? '1' : '0');
			localStorage.setItem('red-panel-open-work', open.work ? '1' : '0');
		}
		function syncToggles() {
			for (var k in toggles) if (toggles[k]) toggles[k].classList.toggle('on', !!open[k]);
		}
		function initHandle(name) {
			var h = handles[name];
			var sign = (name === 'rail') ? 1 : -1;
			var startX = 0, startW = 0, dragging = false;
			h.addEventListener('pointerdown', function (e) {
				dragging = true; startX = e.clientX; startW = widths[name];
				h.setPointerCapture(e.pointerId); h.classList.add('dragging');
				els[name].classList.add('nt'); document.body.classList.add('dragging');
				e.preventDefault();
			});
			h.addEventListener('pointermove', function (e) {
				if (!dragging) return;
				setWidth(name, clamp(name, startW + sign * (e.clientX - startX)));
			});
			h.addEventListener('pointerup', function (e) {
				if (!dragging) return;
				dragging = false; h.releasePointerCapture(e.pointerId);
				h.classList.remove('dragging'); els[name].classList.remove('nt');
				document.body.classList.remove('dragging'); save(); reflow(false);
			});
			h.addEventListener('dblclick', function () { setWidth(name, DEF[name]); save(); });
		}
		function apply(name, animate) {
			var el = els[name], h = handles[name];
			if (shown(name)) {
				h.classList.remove('closed'); el.classList.remove('closed');
				if (animate) {
					el.classList.add('nt'); el.style.width = '0px';
					void el.offsetWidth; el.classList.remove('nt');
				}
				el.style.width = widths[name] + 'px';
			} else {
				h.classList.add('closed');
				el.style.width = '0px'; el.classList.add('closed');
			}
		}
		function reflow(animate) {
			if (isMobile()) return;
			var avail = main.clientWidth;
			if (avail <= 0) return;
			forced.agents = false; forced.work = false;
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
			open[name] = true; save(); reflow(true);
			if (shown(name) && name === 'work') Files.onOpen();
		}
		function hide(name) { open[name] = false; save(); reflow(true); }
		function toggle(name) { if (open[name]) hide(name); else show(name); }
		function init() {
			for (var k in els) {
				var w = parseInt(localStorage.getItem('red-panel-w-' + k) || '0');
				widths[k] = (w >= LIM[k][0] && w <= LIM[k][1]) ? w : DEF[k];
				setWidth(k, widths[k]); initHandle(k);
			}
			open.agents = localStorage.getItem('red-panel-open-agents') !== '0';
			open.work = localStorage.getItem('red-panel-open-work') !== '0';
			reflow(false);
			for (var t in toggles) (function (name) {
				toggles[name].addEventListener('click', function () { toggle(name); });
			})(t);
			document.querySelectorAll('.panel-close').forEach(function (btn) {
				btn.addEventListener('click', function () { hide(btn.dataset.close); });
			});
			var rt = null;
			window.addEventListener('resize', function () {
				if (rt) return;
				rt = setTimeout(function () { rt = null; reflow(false); }, 80);
			});
		}
		return { init: init, toggle: toggle, reflow: function () { reflow(false); },
			isOpen: function (n) { return shown(n); } };
	})();

	// ── Chat rendering ─────────────────────────────────────────
	var curAsstDiv = null;
	var curAsstText = '';

	function appendUserMessage(text) {
		var div = document.createElement('div');
		div.className = 'chat-msg chat-msg-user';
		div.innerHTML = '<div class="chat-msg-content"></div>';
		div.querySelector('.chat-msg-content').textContent = text; // escaped
		chatOutput.appendChild(div);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}

	function appendAssistantText(text) {
		if (!curAsstDiv) {
			curAsstDiv = document.createElement('div');
			curAsstDiv.className = 'chat-msg chat-msg-assistant';
			curAsstDiv.innerHTML = '<div class="chat-msg-content"></div>';
			chatOutput.appendChild(curAsstDiv);
			curAsstText = '';
		}
		curAsstText += text;
		// RedRender.md sanitises the model markup (H5).
		curAsstDiv.querySelector('.chat-msg-content').innerHTML = RedRender.md(curAsstText);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}

	function finalizeAssistant() {
		if (curAsstDiv && curAsstText) {
			curAsstDiv.querySelector('.chat-msg-content').innerHTML = RedRender.md(curAsstText);
		}
		curAsstDiv = null; curAsstText = '';
	}

	var lastToolBlock = null;

	function renderToolCall(name, args) {
		finalizeAssistant();
		var block = document.createElement('div');
		block.className = 'tool-block running collapsed';
		var head = document.createElement('div');
		head.className = 'tool-head';
		head.textContent = '\u{1F527} ' + name;      // escaped via textContent
		head.addEventListener('click', function () { block.classList.toggle('collapsed'); });
		var argsPre = document.createElement('pre');
		argsPre.className = 'tool-args';
		argsPre.textContent = typeof args === 'string' ? args : JSON.stringify(args);
		var resPre = document.createElement('pre');
		resPre.className = 'tool-result';
		resPre.style.display = 'none';
		block.appendChild(head); block.appendChild(argsPre); block.appendChild(resPre);
		chatOutput.appendChild(block);
		lastToolBlock = block;
		chatOutput.scrollTop = chatOutput.scrollHeight;
		Agents.tool(name, block);
	}

	function renderToolResult(name, result) {
		if (lastToolBlock) {
			lastToolBlock.classList.remove('running');
			var resPre = lastToolBlock.querySelector('.tool-result');
			resPre.textContent = result;              // escaped via textContent
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

	function clearChat() { chatOutput.innerHTML = ''; curAsstDiv = null; curAsstText = ''; }

	function renderEmptyState() {
		clearChat();
		var wrap = document.createElement('div');
		wrap.className = 'empty-state';
		wrap.innerHTML =
			'<img class="empty-logo" src="assets/oxedyne_red.svg" alt="">' +
			'<h2>Welcome to Red</h2>' +
			'<p>Runs entirely in your browser. Start a new chat to begin.</p>';
		var btn = document.createElement('button');
		btn.className = 'empty-new-session';
		btn.textContent = '+ New chat';
		btn.addEventListener('click', function () { newChat(); });
		wrap.appendChild(btn);
		chatOutput.appendChild(wrap);
	}

	function renderHistory(messages) {
		clearChat();
		if (!Array.isArray(messages)) return;
		messages.forEach(function (m) {
			if (m.role === 'user') appendUserMessage(m.content);
			else if (m.role === 'assistant') { appendAssistantText(m.content); finalizeAssistant(); }
		});
	}

	// ── Spinner ────────────────────────────────────────────────
	var spinnerEl = null;
	function showSpinner() {
		if (spinnerEl) return;
		spinnerEl = document.createElement('div');
		spinnerEl.className = 'chat-spinner';
		spinnerEl.innerHTML = '<span class="chat-spinner-dot"></span>'
			+ '<span class="chat-spinner-dot"></span><span class="chat-spinner-dot"></span>';
		chatOutput.appendChild(spinnerEl);
		chatOutput.scrollTop = chatOutput.scrollHeight;
	}
	function hideSpinner() { if (spinnerEl) { spinnerEl.remove(); spinnerEl = null; } }

	function setSendMode(mode) {
		chatSend.disabled = false;
		if (mode === 'stop') { chatSend.innerHTML = '■'; chatSend.classList.add('stop'); chatSend.title = 'Stop'; }
		else { chatSend.innerHTML = '➤'; chatSend.classList.remove('stop'); chatSend.title = 'Send'; }
	}

	// ── Meters ─────────────────────────────────────────────────
	function fmtCtx(n) {
		if (n >= 1e6) return (n / 1e6).toFixed(1).replace(/\.0$/, '') + 'M';
		if (n >= 1000) return Math.round(n / 1000) + 'k';
		return '' + n;
	}
	function updateMeters() {
		if (!current) { topMeter.textContent = ''; centerMeter.textContent = ''; return; }
		var pt = current.promptTokens || 0, ct = current.completionTokens || 0;
		var total = pt + ct;
		topMeter.textContent = total > 0 ? (fmtCtx(total) + ' tok') : '';
		var parts = [];
		if (current.model) parts.push(current.model.split('/').pop());
		if (pt > 0) parts.push(fmtCtx(pt) + ' ctx');
		if (total > 0) parts.push(fmtCtx(total) + ' tok');
		centerMeter.textContent = parts.join(' · ');
	}

	// ── Chats ──────────────────────────────────────────────────
	function newChat() {
		if (!cfgReady(cfg)) { openSettings('Set a base URL and model to start chatting.'); return; }
		var chat = {
			id: 'c' + (seq++),
			name: 'Chat ' + timeLabel(),
			app: null,
			messages: [],
			model: cfg.model,
			promptTokens: 0,
			completionTokens: 0,
		};
		chats.unshift(chat);
		selectChat(chat);
		renderSessionList();
		if (isMobile()) mshow('center');
		chatInput.focus();
	}

	function timeLabel() {
		var d = new Date();
		return ('0' + d.getHours()).slice(-2) + ':' + ('0' + d.getMinutes()).slice(-2);
	}

	function selectChat(chat) {
		current = chat;
		sessionNameEl.textContent = chat.name;
		renderHistory(chat.messages);
		updateActiveSession();
		updateMeters();
	}

	function updateActiveSession() {
		sessionList.querySelectorAll('.session-box').forEach(function (box) {
			box.classList.toggle('active', current && box.dataset.id === current.id);
		});
	}

	function renderSessionList() {
		sessionList.innerHTML = '';
		if (chats.length === 0) {
			var note = document.createElement('div');
			note.className = 'rail-note';
			note.textContent = 'No chats yet.';
			sessionList.appendChild(note);
			return;
		}
		chats.forEach(function (s) { sessionList.appendChild(sessionBox(s)); });
		updateActiveSession();
	}

	function sessionBox(s) {
		var box = document.createElement('div');
		box.className = 'session-box' + (current && s.id === current.id ? ' active' : '');
		box.dataset.id = s.id;
		var header = document.createElement('div');
		header.className = 'session-box-header';
		var name = document.createElement('span');
		name.className = 'session-box-name';
		name.textContent = s.name;
		var closeBtn = document.createElement('button');
		closeBtn.className = 'session-box-close';
		closeBtn.textContent = '×';
		closeBtn.title = 'Remove chat';
		closeBtn.addEventListener('click', function (e) {
			e.stopPropagation();
			chats = chats.filter(function (c) { return c.id !== s.id; });
			if (current === s) { current = chats[0] || null; if (current) selectChat(current); else { sessionNameEl.textContent = 'No chat'; renderEmptyState(); updateMeters(); } }
			renderSessionList();
		});
		header.appendChild(name); header.appendChild(closeBtn);
		var meta = document.createElement('div');
		meta.className = 'session-box-meta';
		var tok = (s.promptTokens || 0) + (s.completionTokens || 0);
		if (tok > 0) {
			var ctxLabel = document.createElement('span');
			ctxLabel.className = 'session-box-ctx';
			ctxLabel.textContent = fmtCtx(tok) + ' tok';
			meta.appendChild(ctxLabel);
		}
		box.appendChild(header); box.appendChild(meta);
		box.addEventListener('click', function (e) {
			if (e.target === closeBtn) return;
			if (current && s.id === current.id) { if (isMobile()) mshow('center'); return; }
			selectChat(s);
			if (isMobile()) mshow('center');
		});
		return box;
	}

	// ── Send a turn ────────────────────────────────────────────
	function ensureApp(chat) {
		if (chat.app) return chat.app;
		chat.app = new RedApp(cfg.baseUrl, cfg.apiKey, cfg.model, cfg.maxTokens || 4096,
			SYSTEM_PROMPT, !!cfg.tools);
		chat.model = cfg.model;
		return chat.app;
	}

	async function sendUserMessage() {
		if (generating) return;
		var text = chatInput.value.trim();
		if (!text) return;
		if (!cfgReady(cfg)) { openSettings('Set a base URL and model to start chatting.'); return; }
		if (!current) { newChat(); }
		var chat = current;

		appendUserMessage(text);
		chat.messages.push({ role: 'user', content: text });
		chatInput.value = ''; chatInput.style.height = 'auto'; chatInput.disabled = true;

		generating = true;
		showSpinner(); setSendMode('stop'); Agents.begin(chat);

		var app;
		try { app = ensureApp(chat); }
		catch (e) {
			hideSpinner(); appendError('Could not start agent: ' + String(e));
			generating = false; chatInput.disabled = false; setSendMode('send'); Agents.end('error');
			return;
		}

		var sawText = false;
		var onEvent = function (ev) {
			if (!ev || !ev.type) return;
			if (ev.type === 'text') {
				if (!sawText) { hideSpinner(); sawText = true; }
				appendAssistantText(ev.content || '');
			} else if (ev.type === 'tool_call') {
				hideSpinner();
				renderToolCall(ev.name || '', ev.args || '');
			} else if (ev.type === 'tool_result') {
				renderToolResult(ev.name || '', ev.content || '');
			} else if (ev.type === 'error') {
				hideSpinner();
				appendError(ev.content || 'Error');
			}
			// 'done' handled after run_turn resolves.
		};

		try {
			await app.run_turn(text, onEvent);
			if (curAsstText) chat.messages.push({ role: 'assistant', content: curAsstText });
			finalizeAssistant();
			chat.promptTokens = app.prompt_tokens;
			chat.completionTokens = app.completion_tokens;
			Agents.end('done');
		} catch (e) {
			finalizeAssistant();
			appendError('Turn failed: ' + String(e));
			Agents.end('error');
		} finally {
			hideSpinner();
			generating = false; chatInput.disabled = false; setSendMode('send');
			updateMeters(); renderSessionList();
			Files.refresh();      // a turn may have written files
			chatInput.focus();
		}
	}

	// ── Agents panel ───────────────────────────────────────────
	var Agents = {
		runs: [],
		current: null,
		begin: function (chat) {
			this.current = { name: (chat && chat.name) || 'run', status: 'running', tools: [] };
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
			for (var i = t.length - 1; i >= 0; i--) if (t[i].status === 'running') { t[i].status = 'done'; break; }
			this.render();
		},
		end: function (status) {
			if (!this.current) return;
			this.current.status = status;
			this.current.tools.forEach(function (t) { if (t.status === 'running') t.status = 'done'; });
			this.current = null;
			this.render();
		},
		render: function () {
			if (!agentsList) return;
			agentsList.innerHTML = '';
			var live = 0, self = this;
			this.runs.forEach(function (run) { if (run.status === 'running') live++; agentsList.appendChild(self.tile(run)); });
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
			var ah = document.createElement('div'); ah.className = 'ah';
			var an = document.createElement('span'); an.className = 'an'; an.textContent = run.name;
			var pill = document.createElement('span');
			pill.className = 'pill ' + (run.status === 'running' ? 'run' : run.status === 'error' ? 'err' : 'ok');
			pill.textContent = run.status;
			ah.appendChild(an); ah.appendChild(pill); card.appendChild(ah);
			var arow = document.createElement('div'); arow.className = 'arow';
			var left = document.createElement('span');
			left.textContent = run.tools.length ? run.tools.length + ' tool' + (run.tools.length === 1 ? '' : 's') : '';
			arow.appendChild(left); arow.appendChild(document.createElement('span'));
			card.appendChild(arow);
			if (run.tools.length) {
				var wrap = document.createElement('div'); wrap.className = 'atools';
				run.tools.slice(-8).forEach(function (t) {
					var row = document.createElement('div'); row.className = 'atool';
					var dot = document.createElement('span');
					dot.className = t.status === 'running' ? 'live' : 'tick';
					dot.textContent = t.status === 'running' ? '●' : '✓';
					var nm = document.createElement('span'); nm.textContent = t.name;
					row.appendChild(dot); row.appendChild(nm);
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

	// ── Workspace (OPFS over run_tool) ─────────────────────────
	var Files = (function () {
		var pathEl, treeEl, viewEl;
		var curDir = '';
		var curFile = null, curContent = '';
		var listed = false;
		var showLineNos = localStorage.getItem('red-files-lineno') !== '0';

		// A tools-only RedApp; run_tool never contacts the LLM, so a
		// placeholder base URL is fine when no provider is configured.
		var toolsApp = null;
		function tools() {
			if (toolsApp) return toolsApp;
			var base = cfg.baseUrl || 'http://127.0.0.1/v1/chat/completions';
			try { toolsApp = new RedApp(base, cfg.apiKey || '', cfg.model || 'none', 256, SYSTEM_PROMPT, true); }
			catch (e) { toolsApp = new RedApp('http://127.0.0.1/v1/chat/completions', '', 'none', 256, SYSTEM_PROMPT, true); }
			return toolsApp;
		}

		function fmtBytes(n) {
			if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
			if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
			return n + ' B';
		}
		function joinPath(dir, name) { return dir ? (dir + '/' + name) : name; }

		function bind() {
			var panel = document.getElementById('panel-work');
			if (!panel) return;
			pathEl = panel.querySelector('.files-path');
			treeEl = panel.querySelector('.files-tree');
			viewEl = panel.querySelector('.files-view');
			panel.querySelector('[data-act="refresh"]').addEventListener('click', function () { list(curDir); });
			panel.querySelector('[data-act="up"]').addEventListener('click', function () {
				if (curFile) { closeView(); return; }
				if (!curDir) return;
				var parts = curDir.split('/').filter(Boolean); parts.pop();
				list(parts.join('/'));
			});
		}

		function isOpen() {
			if (isMobile()) return document.body.dataset.mpanel === 'work';
			return RedPanels.isOpen('work');
		}

		// Parse the plain-text file_list output into entries. Lines are
		// "name/" for a directory and "name  (N bytes)" for a file; an
		// empty directory yields "<path> is empty.".
		function parseListing(text) {
			var out = [];
			if (/ is empty\.$/.test(text.trim())) return out;
			text.split('\n').forEach(function (line) {
				if (!line) return;
				if (line.charAt(line.length - 1) === '/') {
					out.push({ name: line.slice(0, -1), dir: true, size: 0 });
				} else {
					var m = /^(.*?)\s{2}\((\d+) bytes\)$/.exec(line);
					if (m) out.push({ name: m[1], dir: false, size: parseInt(m[2], 10) });
					else out.push({ name: line, dir: false, size: 0 });
				}
			});
			return out;
		}

		async function list(dir) {
			curDir = dir || '';
			curFile = null; listed = true;
			viewEl.style.display = 'none'; treeEl.style.display = '';
			pathEl.textContent = '/' + curDir;
			treeEl.innerHTML = '<div class="files-empty">…</div>';
			var res = await tools().run_tool('file_list', JSON.stringify({ path: curDir || '.' }));
			if (typeof res === 'string' && res.indexOf('Error') === 0) {
				treeEl.innerHTML = '';
				var err = document.createElement('div');
				err.className = 'files-empty';
				err.textContent = res;         // escaped
				treeEl.appendChild(err);
				return;
			}
			renderTree(parseListing(res));
		}

		function renderTree(entries) {
			entries.sort(function (a, b) { return (b.dir - a.dir) || a.name.localeCompare(b.name); });
			treeEl.innerHTML = '';
			if (entries.length === 0) { treeEl.innerHTML = '<div class="files-empty">empty</div>'; return; }
			entries.forEach(function (e) {
				var row = document.createElement('div');
				row.className = 'files-row' + (e.dir ? ' dir' : '');
				var name = document.createElement('span');
				name.className = 'files-name';
				name.textContent = (e.dir ? '📁 ' : '📄 ') + e.name;   // escaped
				row.appendChild(name);
				if (!e.dir) {
					var size = document.createElement('span');
					size.className = 'files-size';
					size.textContent = fmtBytes(e.size || 0);
					row.appendChild(size);
				}
				var del = document.createElement('button');
				del.className = 'files-del'; del.textContent = '×'; del.title = 'Delete';
				del.addEventListener('click', async function (ev) {
					ev.stopPropagation();
					if (!confirm('Delete ' + e.name + '?')) return;
					await tools().run_tool('file_delete', JSON.stringify({ path: joinPath(curDir, e.name) }));
					list(curDir);
				});
				row.appendChild(del);
				row.addEventListener('click', function () {
					var p = joinPath(curDir, e.name);
					if (e.dir) list(p); else openFile(p);
				});
				treeEl.appendChild(row);
			});
		}

		async function openFile(path) {
			var content = await tools().run_tool('file_read', JSON.stringify({ path: path }));
			curFile = path; curContent = content;
			treeEl.style.display = 'none'; viewEl.style.display = '';
			viewEl.innerHTML =
				'<div class="files-view-head">' +
				'  <span class="files-view-name"></span>' +
				'  <span>' +
				'    <button class="files-btn" data-act="lineno" title="Line numbers">#</button>' +
				'    <button class="files-btn" data-act="download" title="Download">⤓</button>' +
				'    <button class="files-btn" data-act="back">← Back</button>' +
				'  </span>' +
				'</div>' +
				'<pre class="files-view-body"></pre>';
			viewEl.querySelector('.files-view-name').textContent = path;   // escaped
			renderFileBody();
			viewEl.querySelector('[data-act="back"]').addEventListener('click', closeView);
			viewEl.querySelector('[data-act="lineno"]').addEventListener('click', function () {
				showLineNos = !showLineNos;
				localStorage.setItem('red-files-lineno', showLineNos ? '1' : '0');
				renderFileBody();
			});
			viewEl.querySelector('[data-act="download"]').addEventListener('click', function () {
				var blob = new Blob([curContent], { type: 'text/plain' });
				var a = document.createElement('a');
				a.href = URL.createObjectURL(blob);
				a.download = path.split('/').pop() || 'file.txt';
				a.click(); URL.revokeObjectURL(a.href);
			});
		}

		function renderFileBody() {
			var body = viewEl.querySelector('.files-view-body');
			if (!body) return;
			var btn = viewEl.querySelector('[data-act="lineno"]');
			if (btn) btn.classList.toggle('active', showLineNos);
			if (showLineNos) {
				var lines = curContent.split('\n');
				var html = '';
				for (var i = 0; i < lines.length; i++) {
					html += '<span class="ln">' + (i + 1) + '</span>' + esc(lines[i]) + '\n';
				}
				body.innerHTML = html;        // only line numbers + escaped text
				body.classList.add('with-lineno');
			} else {
				body.textContent = curContent;
				body.classList.remove('with-lineno');
			}
		}

		function closeView() { viewEl.style.display = 'none'; treeEl.style.display = ''; curFile = null; }
		function onOpen() { if (!curFile) list(curDir); }
		function refresh() { if (isOpen() && listed && !curFile) list(curDir); }

		return { init: bind, onOpen: onOpen, refresh: refresh };
	})();

	// ── Settings modal ─────────────────────────────────────────
	function fillSettings() {
		document.getElementById('cfg-base-url').value = cfg.baseUrl || '';
		document.getElementById('cfg-api-key').value = cfg.apiKey || '';
		document.getElementById('cfg-model').value = cfg.model || '';
		document.getElementById('cfg-max-tokens').value = cfg.maxTokens || 4096;
		document.getElementById('cfg-tools').checked = !!cfg.tools;
	}
	function openSettings(note) {
		fillSettings();
		document.getElementById('byok-note').textContent = note || '';
		settingsModal.style.display = 'flex';
	}
	settingsBtn.addEventListener('click', function () { openSettings(''); });
	settingsClose.addEventListener('click', function () { settingsModal.style.display = 'none'; });
	document.getElementById('byok-form').addEventListener('submit', function (e) {
		e.preventDefault();
		cfg = {
			baseUrl: document.getElementById('cfg-base-url').value.trim(),
			apiKey: document.getElementById('cfg-api-key').value,
			model: document.getElementById('cfg-model').value.trim(),
			maxTokens: parseInt(document.getElementById('cfg-max-tokens').value, 10) || 4096,
			tools: document.getElementById('cfg-tools').checked,
		};
		saveCfg(cfg);
		document.getElementById('byok-note').textContent = 'Saved.';
		// New settings imply fresh app instances for existing chats.
		chats.forEach(function (c) { c.app = null; });
		setTimeout(function () { settingsModal.style.display = 'none'; }, 400);
	});

	// ── Input wiring ───────────────────────────────────────────
	chatInput.addEventListener('input', function () {
		chatInput.style.height = 'auto';
		chatInput.style.height = Math.min(chatInput.scrollHeight, 120) + 'px';
	});
	chatInput.addEventListener('keydown', function (e) {
		if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendUserMessage(); }
	});
	chatSend.addEventListener('click', function () { sendUserMessage(); });
	newSessionBtn.addEventListener('click', newChat);

	// Show/hide tool blocks in the thread.
	var toolsHidden = localStorage.getItem('red-hide-tools') === '1';
	var stepsBtn = document.getElementById('steps-toggle-btn');
	function applyToolsVisibility() {
		chatOutput.classList.toggle('hide-tools', toolsHidden);
		if (stepsBtn) stepsBtn.classList.toggle('dim', toolsHidden);
	}
	if (stepsBtn) stepsBtn.addEventListener('click', function () {
		toolsHidden = !toolsHidden;
		localStorage.setItem('red-hide-tools', toolsHidden ? '1' : '0');
		applyToolsVisibility();
	});
	applyToolsVisibility();

	// ── Boot ───────────────────────────────────────────────────
	async function boot() {
		initTheme();
		RedPanels.init();
		Files.init();
		Agents.render();
		mshow(document.body.dataset.mpanel || 'center');
		try {
			await init();               // instantiate the wasm module
			window.__RED_READY = true;
		} catch (e) {
			appEl.classList.add('wasm-failed');
			appendError('Failed to load the browser engine: ' + String(e));
			window.__RED_READY = false;
			return;
		}
		renderSessionList();
		renderEmptyState();
		RedPanels.reflow();
		if (!isMobile() && RedPanels.isOpen('work')) Files.onOpen();  // initial listing
		if (!cfgReady(cfg)) {
			// Prompt for BYOK on first run.
			openSettings('Welcome. Add a provider base URL, key and model to begin.');
		}
	}
	boot();
})();
