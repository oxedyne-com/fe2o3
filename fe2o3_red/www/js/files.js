/* ============================================================
   Red — workspace file browser (WS-F)
   ------------------------------------------------------------
   A vanilla-JS file panel over the chat WebSocket, docked as
   the workspace panel (#panel-work in index.html). Protocol
   (client -> server):
     fs_list  "path"                list a directory
     fs_read  "path"                read a text file
     fs_delete "path"               delete a file
     fs_write "path" "content"      create/overwrite a file
   (server -> client):
     fs_tree    "<json entries>"    { path, entries:[{name,dir,size}] }
     fs_content "path" "content"    file contents
   app.js wires: RedFiles.init(sendChat); routes fs_tree ->
   RedFiles.onTree, fs_content -> RedFiles.onContent; calls
   RedFiles.onOpen() when the panel opens and RedFiles.refresh()
   after each agent turn. Panel visibility itself is owned by
   RedPanels (desktop) and the mobile nav.
   ============================================================ */
(function () {
	'use strict';

	var send = null;      // injected sendChat(cmd)
	var pathEl = null;
	var treeEl = null;
	var viewEl = null;
	var curDir = '';      // current directory (workspace-relative)
	var curFile = null;   // currently open file path
	var curContent = '';
	var listed = false;   // has an initial listing been requested?
	var showLineNos = localStorage.getItem('red-files-lineno') !== '0'; // default on

	// Hex-encode a string (UTF-8) so arbitrary file bytes survive the
	// WS syntax parser (newlines, quotes, $, etc.).
	function toHex(str) {
		var bytes = new TextEncoder().encode(str);
		var h = '';
		for (var i = 0; i < bytes.length; i++) {
			h += (bytes[i] < 16 ? '0' : '') + bytes[i].toString(16);
		}
		return h;
	}

	function esc(s) {
		return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;')
			.replace(/>/g, '&gt;').replace(/"/g, '&quot;');
	}
	function escJdat(s) { return String(s).replace(/\\/g, '\\\\').replace(/"/g, '\\"'); }

	function fmtBytes(n) {
		if (n >= 1048576) return (n / 1048576).toFixed(1) + ' MB';
		if (n >= 1024) return (n / 1024).toFixed(1) + ' KB';
		return n + ' B';
	}

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
			var parts = curDir.split('/').filter(Boolean);
			parts.pop();
			list(parts.join('/'));
		});
		panel.querySelector('input[type="file"]').addEventListener('change', onUpload);
	}

	function isOpen() {
		if (window.matchMedia('(max-width: 760px)').matches) {
			return document.body.dataset.mpanel === 'work';
		}
		return !window.RedPanels || RedPanels.isOpen('work');
	}

	function list(dir) {
		curDir = dir || '';
		curFile = null;
		listed = true;
		viewEl.style.display = 'none';
		treeEl.style.display = '';
		send('fs_list "' + escJdat(curDir || '.') + '"');
	}

	function closeView() {
		viewEl.style.display = 'none';
		treeEl.style.display = '';
		curFile = null;
	}

	function onTree(obj) {
		if (typeof obj === 'string') { try { obj = JSON.parse(obj); } catch (e) { return; } }
		curDir = (obj.path === '.' ? '' : obj.path) || '';
		pathEl.textContent = '/' + curDir;
		var entries = obj.entries || [];
		entries.sort(function (a, b) { return (b.dir - a.dir) || a.name.localeCompare(b.name); });
		treeEl.innerHTML = '';
		if (entries.length === 0) {
			treeEl.innerHTML = '<div class="files-empty">empty</div>';
			return;
		}
		entries.forEach(function (e) {
			var row = document.createElement('div');
			row.className = 'files-row' + (e.dir ? ' dir' : '');
			var name = document.createElement('span');
			name.className = 'files-name';
			name.textContent = (e.dir ? '📁 ' : '📄 ') + e.name;
			row.appendChild(name);
			if (!e.dir) {
				var size = document.createElement('span');
				size.className = 'files-size';
				size.textContent = fmtBytes(e.size || 0);
				row.appendChild(size);
			}
			var del = document.createElement('button');
			del.className = 'files-del';
			del.textContent = '×';
			del.title = 'Delete';
			del.addEventListener('click', function (ev) {
				ev.stopPropagation();
				if (confirm('Delete ' + e.name + '?')) {
					send('fs_delete "' + escJdat(joinPath(curDir, e.name)) + '"');
					setTimeout(function () { list(curDir); }, 150);
				}
			});
			row.appendChild(del);
			row.addEventListener('click', function () {
				var p = joinPath(curDir, e.name);
				if (e.dir) { list(p); } else { send('fs_read "' + escJdat(p) + '"'); }
			});
			treeEl.appendChild(row);
		});
	}

	function joinPath(dir, name) { return dir ? (dir + '/' + name) : name; }

	function onContent(path, content) {
		curFile = path;
		curContent = content;
		treeEl.style.display = 'none';
		viewEl.style.display = '';
		viewEl.innerHTML =
			'<div class="files-view-head">' +
			'  <span class="files-view-name">' + esc(path) + '</span>' +
			'  <span>' +
			'    <button class="files-btn" data-act="lineno" title="Line numbers">#</button>' +
			'    <button class="files-btn" data-act="download" title="Download">⤓</button>' +
			'    <button class="files-btn" data-act="back">← Back</button>' +
			'  </span>' +
			'</div>' +
			'<pre class="files-view-body"></pre>';
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
			a.click();
			URL.revokeObjectURL(a.href);
		});
	}

	// Render the open file's body, optionally with a line-number gutter.
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
			body.innerHTML = html;
			body.classList.add('with-lineno');
		} else {
			body.textContent = curContent;
			body.classList.remove('with-lineno');
		}
	}

	function onUpload(e) {
		var file = e.target.files && e.target.files[0];
		if (!file) return;
		var reader = new FileReader();
		reader.onload = function () {
			send('fs_write "' + escJdat(joinPath(curDir, file.name)) + '" "' + toHex(reader.result) + '"');
			setTimeout(function () { list(curDir); }, 200);
		};
		reader.readAsText(file);
		e.target.value = '';
	}

	// Called when the workspace panel is opened (desktop toggle or
	// mobile nav) — fetch the initial or refreshed listing.
	function onOpen() {
		if (!curFile) { list(curDir); }
	}

	// Re-list the current directory if the panel is open and browsing
	// (not viewing a file) — used to reflect files the agent just wrote.
	function refresh() {
		if (isOpen() && listed && !curFile) { list(curDir); }
	}

	window.RedFiles = {
		init: function (sendFn) { send = sendFn; bind(); },
		onOpen: onOpen,
		refresh: refresh,
		onTree: onTree,
		onContent: onContent
	};
})();
