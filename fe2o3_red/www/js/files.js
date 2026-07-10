/* ============================================================
   Red — workspace file browser (WS-F)
   ------------------------------------------------------------
   A vanilla-JS file panel over the chat WebSocket.  Protocol
   (client -> server):
     fs_list  "path"                list a directory
     fs_read  "path"                read a text file
     fs_delete "path"               delete a file
     fs_write "path" "content"      create/overwrite a file
   (server -> client):
     fs_tree    "<json entries>"    { path, entries:[{name,dir,size}] }
     fs_content "path" "content"    file contents
   app.js wires: RedFiles.init(sendChat); routes fs_tree ->
   RedFiles.onTree, fs_content -> RedFiles.onContent; and calls
   RedFiles.toggle() from the Files button.
   ============================================================ */
(function () {
	'use strict';

	var send = null;      // injected sendChat(cmd)
	var panel = null;     // the files panel element
	var treeEl = null;
	var viewEl = null;
	var curDir = '';      // current directory (workspace-relative)
	var curFile = null;   // currently open file path
	var curContent = '';

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

	function build() {
		if (panel) return;
		panel = document.createElement('div');
		panel.className = 'files-panel';
		panel.innerHTML =
			'<div class="files-head">' +
			'  <span class="files-title">Files</span>' +
			'  <span class="files-actions">' +
			'    <button class="files-btn" data-act="up" title="Up">↑</button>' +
			'    <button class="files-btn" data-act="refresh" title="Refresh">⟳</button>' +
			'    <label class="files-btn" title="Upload"><input type="file" style="display:none">⤒</label>' +
			'    <button class="files-btn" data-act="close" title="Close">×</button>' +
			'  </span>' +
			'</div>' +
			'<div class="files-path"></div>' +
			'<div class="files-tree"></div>' +
			'<div class="files-view" style="display:none"></div>';
		document.body.appendChild(panel);
		treeEl = panel.querySelector('.files-tree');
		viewEl = panel.querySelector('.files-view');
		panel.querySelector('[data-act="close"]').addEventListener('click', hide);
		panel.querySelector('[data-act="refresh"]').addEventListener('click', function () { list(curDir); });
		panel.querySelector('[data-act="up"]').addEventListener('click', function () {
			if (!curDir) return;
			var parts = curDir.split('/').filter(Boolean);
			parts.pop();
			list(parts.join('/'));
		});
		panel.querySelector('input[type="file"]').addEventListener('change', onUpload);
	}

	function list(dir) {
		curDir = dir || '';
		curFile = null;
		viewEl.style.display = 'none';
		treeEl.style.display = '';
		send('fs_list "' + escJdat(curDir || '.') + '"');
	}

	function onTree(obj) {
		if (!panel) build();
		if (typeof obj === 'string') { try { obj = JSON.parse(obj); } catch (e) { return; } }
		curDir = (obj.path === '.' ? '' : obj.path) || '';
		panel.querySelector('.files-path').textContent = '/' + curDir;
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
		if (!panel) build();
		curFile = path;
		curContent = content;
		treeEl.style.display = 'none';
		viewEl.style.display = '';
		viewEl.innerHTML =
			'<div class="files-view-head">' +
			'  <span class="files-view-name">' + esc(path) + '</span>' +
			'  <span>' +
			'    <button class="files-btn" data-act="download">⤓</button>' +
			'    <button class="files-btn" data-act="back">← Back</button>' +
			'  </span>' +
			'</div>' +
			'<pre class="files-view-body"></pre>';
		viewEl.querySelector('.files-view-body').textContent = content;
		viewEl.querySelector('[data-act="back"]').addEventListener('click', function () {
			viewEl.style.display = 'none'; treeEl.style.display = '';
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

	function onUpload(e) {
		var file = e.target.files && e.target.files[0];
		if (!file) return;
		var reader = new FileReader();
		reader.onload = function () {
			send('fs_write "' + escJdat(joinPath(curDir, file.name)) + '" "' + escJdat(reader.result) + '"');
			setTimeout(function () { list(curDir); }, 200);
		};
		reader.readAsText(file);
		e.target.value = '';
	}

	function show() { if (!panel) build(); panel.classList.add('open'); list(curDir); }
	function hide() { if (panel) panel.classList.remove('open'); }
	function toggle() { if (panel && panel.classList.contains('open')) hide(); else show(); }

	window.RedFiles = {
		init: function (sendFn) { send = sendFn; },
		toggle: toggle,
		show: show,
		hide: hide,
		onTree: onTree,
		onContent: onContent
	};
})();
