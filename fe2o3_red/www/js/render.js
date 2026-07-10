/* ============================================================
   Red — rich markdown rendering + lightweight code highlighting
   ------------------------------------------------------------
   Self-contained, dependency-free (relies only on the already
   global `marked`).  Exposes:

       window.RedRender = { md, escapeHtml }

   `RedRender.md(text)` renders markdown to an HTML string with:
     - single-newline line breaks (marked `breaks: true`),
     - a small built-in syntax highlighter for common languages,
     - a "Copy" button on every fenced code block.

   Tables (and all other non-code markup) pass through untouched.
   ============================================================ */
(function () {
	'use strict';

	// ── HTML entity helpers ────────────────────────────────────

	/// Escape the five significant HTML characters.
	function escapeHtml(s) {
		return String(s)
			.replace(/&/g, '&amp;')
			.replace(/</g, '&lt;')
			.replace(/>/g, '&gt;')
			.replace(/"/g, '&quot;')
			.replace(/'/g, '&#39;');
	}

	/// Reverse the entity encoding that `marked` applies to fenced
	/// code bodies, recovering the raw source text.
	function unescapeHtml(s) {
		return String(s)
			.replace(/&lt;/g, '<')
			.replace(/&gt;/g, '>')
			.replace(/&quot;/g, '"')
			.replace(/&#0?39;/g, "'")
			.replace(/&#x27;/gi, "'")
			.replace(/&amp;/g, '&');
	}

	// ── Language specifications ────────────────────────────────

	/// Build a lookup object from an array of words.
	function wordSet(arr) {
		var o = Object.create(null);
		for (var i = 0; i < arr.length; i++) o[arr[i]] = true;
		return o;
	}

	// Shared token regexes (sticky, matched at an explicit offset).
	var RE = {
		lineSlash:  /\/\/[^\n]*/y,
		blockC:     /\/\*[\s\S]*?\*\//y,
		lineHash:   /#[^\n]*/y,
		dStr:       /"(?:\\.|[^"\\\n])*"?/y,
		sStr:       /'(?:\\.|[^'\\\n])*'?/y,
		tplStr:     /`(?:\\.|[^`\\])*`?/y,
		pyTripleD:  /"""[\s\S]*?(?:"""|$)/y,
		pyTripleS:  /'''[\s\S]*?(?:'''|$)/y,
		num:        /0[xXbBoO][0-9a-fA-F_]+|(?:\d[\d_]*)(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?/y,
		ident:      /[A-Za-z_$][A-Za-z0-9_$]*/y,
		bashVar:    /\$\{[^}]*\}|\$[A-Za-z0-9_]+|\$[@*#?$!0-9]/y,
	};

	// Each rule is { re, cls }.  cls "ident" is classified further.
	var LANGS = {
		javascript: {
			upperType: true,
			keywords: wordSet([
				'var', 'let', 'const', 'function', 'return', 'if', 'else',
				'for', 'while', 'do', 'switch', 'case', 'break', 'continue',
				'new', 'typeof', 'instanceof', 'in', 'of', 'this', 'class',
				'extends', 'super', 'import', 'export', 'from', 'default',
				'try', 'catch', 'finally', 'throw', 'async', 'await', 'yield',
				'delete', 'void', 'static', 'get', 'set',
			]),
			types: wordSet([]),
			literals: wordSet(['true', 'false', 'null', 'undefined', 'NaN', 'Infinity']),
			rules: [
				{ re: RE.blockC, cls: 'comment' },
				{ re: RE.lineSlash, cls: 'comment' },
				{ re: RE.tplStr, cls: 'string' },
				{ re: RE.dStr, cls: 'string' },
				{ re: RE.sStr, cls: 'string' },
				{ re: RE.num, cls: 'number' },
				{ re: RE.ident, cls: 'ident' },
			],
		},
		rust: {
			upperType: true,
			keywords: wordSet([
				'fn', 'let', 'mut', 'const', 'static', 'struct', 'enum',
				'trait', 'impl', 'for', 'while', 'loop', 'if', 'else',
				'match', 'return', 'break', 'continue', 'use', 'mod', 'pub',
				'crate', 'self', 'Self', 'super', 'as', 'where', 'ref',
				'move', 'dyn', 'async', 'await', 'unsafe', 'extern', 'type',
				'in', 'box',
			]),
			types: wordSet([
				'i8', 'i16', 'i32', 'i64', 'i128', 'isize',
				'u8', 'u16', 'u32', 'u64', 'u128', 'usize',
				'f32', 'f64', 'bool', 'char', 'str', 'String',
				'Vec', 'Option', 'Result', 'Box', 'Rc', 'Arc', 'HashMap',
				'BTreeMap', 'Outcome',
			]),
			literals: wordSet(['true', 'false', 'Some', 'None', 'Ok', 'Err']),
			rules: [
				{ re: RE.blockC, cls: 'comment' },
				{ re: RE.lineSlash, cls: 'comment' },
				{ re: RE.dStr, cls: 'string' },
				{ re: RE.sStr, cls: 'string' },
				{ re: RE.num, cls: 'number' },
				{ re: RE.ident, cls: 'ident' },
			],
		},
		python: {
			upperType: true,
			keywords: wordSet([
				'def', 'return', 'if', 'elif', 'else', 'for', 'while',
				'break', 'continue', 'pass', 'import', 'from', 'as', 'class',
				'try', 'except', 'finally', 'raise', 'with', 'lambda',
				'global', 'nonlocal', 'yield', 'del', 'in', 'is', 'not',
				'and', 'or', 'assert', 'async', 'await',
			]),
			types: wordSet([
				'int', 'float', 'str', 'bool', 'list', 'dict', 'tuple',
				'set', 'bytes', 'object',
			]),
			literals: wordSet(['True', 'False', 'None', 'self', 'cls']),
			rules: [
				{ re: RE.lineHash, cls: 'comment' },
				{ re: RE.pyTripleD, cls: 'string' },
				{ re: RE.pyTripleS, cls: 'string' },
				{ re: RE.dStr, cls: 'string' },
				{ re: RE.sStr, cls: 'string' },
				{ re: RE.num, cls: 'number' },
				{ re: RE.ident, cls: 'ident' },
			],
		},
		bash: {
			upperType: false,
			keywords: wordSet([
				'if', 'then', 'else', 'elif', 'fi', 'for', 'while', 'until',
				'do', 'done', 'case', 'esac', 'function', 'in', 'select',
				'return', 'break', 'continue', 'local', 'export', 'declare',
				'readonly', 'source', 'alias', 'unset', 'set', 'echo', 'cd',
				'exit', 'trap',
			]),
			types: wordSet([]),
			literals: wordSet(['true', 'false']),
			rules: [
				{ re: RE.lineHash, cls: 'comment' },
				{ re: RE.bashVar, cls: 'type' },
				{ re: RE.dStr, cls: 'string' },
				{ re: RE.sStr, cls: 'string' },
				{ re: RE.num, cls: 'number' },
				{ re: RE.ident, cls: 'ident' },
			],
		},
		json: {
			upperType: false,
			keywords: wordSet([]),
			types: wordSet([]),
			literals: wordSet(['true', 'false', 'null']),
			rules: [
				{ re: RE.dStr, cls: 'string' },
				{ re: RE.num, cls: 'number' },
				{ re: RE.ident, cls: 'ident' },
			],
		},
	};

	// Map common aliases to canonical language keys.
	var ALIAS = {
		js: 'javascript', javascript: 'javascript', node: 'javascript',
		jsx: 'javascript', mjs: 'javascript', ts: 'javascript',
		typescript: 'javascript', tsx: 'javascript',
		rs: 'rust', rust: 'rust',
		py: 'python', python: 'python',
		sh: 'bash', shell: 'bash', bash: 'bash', zsh: 'bash', console: 'bash',
		json: 'json', jsonc: 'json', json5: 'json',
	};

	/// Resolve a fence language token to a canonical key, or '' if
	/// unsupported.
	function canonLang(tok) {
		if (!tok) return '';
		return ALIAS[tok.toLowerCase()] || '';
	}

	// ── Highlighter ────────────────────────────────────────────

	/// Classify a bare identifier against a language spec.
	function classifyIdent(word, spec) {
		if (spec.keywords[word]) {
			return '<span class="tok-keyword">' + escapeHtml(word) + '</span>';
		}
		if (spec.literals[word]) {
			return '<span class="tok-literal">' + escapeHtml(word) + '</span>';
		}
		if (spec.types[word]) {
			return '<span class="tok-type">' + escapeHtml(word) + '</span>';
		}
		if (spec.upperType && /^[A-Z]/.test(word)) {
			return '<span class="tok-type">' + escapeHtml(word) + '</span>';
		}
		return escapeHtml(word);
	}

	/// Highlight raw code for a canonical language.  Unknown or empty
	/// languages fall back to a plain HTML-escaped render.  Never
	/// throws.
	function highlight(code, lang) {
		var spec = LANGS[lang];
		if (!spec) return escapeHtml(code);
		var out = '';
		var i = 0;
		var n = code.length;
		var rules = spec.rules;
		var guard = 0;
		while (i < n) {
			// Safety valve against any pathological zero-width match.
			if (++guard > n + 16) { out += escapeHtml(code.slice(i)); break; }
			var matched = false;
			for (var r = 0; r < rules.length; r++) {
				var rule = rules[r];
				rule.re.lastIndex = i;
				var m = rule.re.exec(code);
				if (m && m.index === i && m[0].length > 0) {
					var txt = m[0];
					if (rule.cls === 'ident') {
						out += classifyIdent(txt, spec);
					} else {
						out += '<span class="tok-' + rule.cls + '">' +
							escapeHtml(txt) + '</span>';
					}
					i += txt.length;
					matched = true;
					break;
				}
			}
			if (!matched) {
				out += escapeHtml(code.charAt(i));
				i++;
			}
		}
		return out;
	}

	// ── Code-block enhancement ─────────────────────────────────

	// Matches a marked-emitted fenced code block.  The body is already
	// entity-escaped by marked, so `</code></pre>` is unambiguous.
	var CODE_RE = /<pre><code([^>]*)>([\s\S]*?)<\/code><\/pre>/g;

	/// Replace each `<pre><code>` block with a titled container that
	/// carries a language label, a copy button, and highlighted code.
	function enhanceCodeBlocks(html) {
		return html.replace(CODE_RE, function (whole, attrs, body) {
			try {
				var tok = '';
				var cm = /class="([^"]*)"/.exec(attrs);
				if (cm) {
					var lm = /language-([A-Za-z0-9_+#.-]+)/.exec(cm[1]);
					if (lm) tok = lm[1];
				}
				var lang = canonLang(tok);
				// Recover the raw source, dropping the trailing newline
				// that marked appends.
				var raw = unescapeHtml(body).replace(/\n$/, '');
				var hi = highlight(raw, lang);
				var label = tok ? tok.toLowerCase() : 'text';
				// Raw code parked in a data attribute for the copy button;
				// escapeHtml makes it attribute-safe and it decodes back to
				// the exact source via getAttribute().
				var enc = escapeHtml(raw);
				return '<div class="code-block" data-lang="' + escapeHtml(label) + '">' +
					'<div class="code-block-head">' +
					'<span class="code-block-lang">' + escapeHtml(label) + '</span>' +
					'<button class="code-copy-btn" type="button" data-code="' + enc + '">Copy</button>' +
					'</div>' +
					'<pre><code>' + hi + '</code></pre>' +
					'</div>';
			} catch (e) {
				// Never lose the content on an odd edge case.
				return whole;
			}
		});
	}

	// ── Sanitisation (H5: escape-by-default) ───────────────────
	// The rendered surface is now the whole app, so model output must
	// never introduce live markup.  `marked` passes raw HTML through
	// untouched, so its output is sanitised against a tag/attribute
	// whitelist before it ever reaches the DOM.  A `<template>` holds
	// the parse inertly (no scripts run, no resources load); any tag
	// outside the whitelist is reduced to its text, dangerous elements
	// are dropped whole, and only vetted attributes and URLs survive.

	// Inline formatting, lists, headings, tables, code — the shape of
	// ordinary markdown output, nothing that can execute.
	var TAG_OK = wordSet([
		'A', 'ABBR', 'B', 'BLOCKQUOTE', 'BR', 'CODE', 'DEL', 'DIV', 'EM',
		'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'HR', 'I', 'IMG', 'KBD', 'LI',
		'OL', 'P', 'PRE', 'S', 'SPAN', 'STRONG', 'SUB', 'SUP', 'TABLE',
		'TBODY', 'TD', 'TH', 'THEAD', 'TR', 'U', 'UL',
	]);
	// Elements dropped whole (content and all), never merely unwrapped.
	var TAG_DROP = wordSet([
		'SCRIPT', 'STYLE', 'IFRAME', 'OBJECT', 'EMBED', 'LINK', 'META',
		'TEMPLATE', 'NOSCRIPT', 'FORM', 'INPUT', 'BUTTON', 'TEXTAREA',
		'SELECT', 'SVG', 'MATH',
	]);
	// Attributes safe on any allowed element.
	var ATTR_OK = wordSet(['CLASS', 'TITLE', 'ALT', 'ALIGN']);

	/// True when a URL is safe to keep — http(s), mailto, in-page or
	/// root-relative, or an inline image data URI.  Everything else
	/// (notably `javascript:`) is rejected.
	function safeUrl(u) {
		var v = String(u == null ? '' : u).trim();
		if (/^(https?:|mailto:|#|\/)/i.test(v)) return true;
		if (/^data:image\/(png|jpe?g|gif|webp|svg\+xml);/i.test(v)) return true;
		return false;
	}

	/// Recursively scrub a node's children in place against the
	/// whitelist.  Elements are visited on a static snapshot so
	/// live-collection surprises cannot skip a node.
	function scrub(node) {
		var kids = Array.prototype.slice.call(node.childNodes);
		for (var i = 0; i < kids.length; i++) {
			var ch = kids[i];
			if (ch.nodeType === 8) { node.removeChild(ch); continue; } // comment
			if (ch.nodeType !== 1) continue;                            // keep text
			var tag = ch.tagName;
			if (TAG_DROP[tag]) { node.removeChild(ch); continue; }
			if (!TAG_OK[tag]) {
				// Unknown wrapper: keep the text, discard the markup.
				node.replaceChild(document.createTextNode(ch.textContent || ''), ch);
				continue;
			}
			var attrs = Array.prototype.slice.call(ch.attributes);
			for (var a = 0; a < attrs.length; a++) {
				var name = attrs[a].name;
				var up = name.toUpperCase();
				var keep = ATTR_OK[up];
				if (!keep && up === 'HREF' && tag === 'A') keep = safeUrl(attrs[a].value);
				if (!keep && up === 'SRC' && tag === 'IMG') keep = safeUrl(attrs[a].value);
				if (!keep) ch.removeAttribute(name);
			}
			if (tag === 'A') ch.setAttribute('rel', 'noopener noreferrer nofollow');
			scrub(ch);
		}
	}

	/// Sanitise an HTML string, returning safe HTML.  Falls back to a
	/// fully-escaped render if the DOM APIs are unavailable.
	function sanitize(html) {
		if (typeof document === 'undefined' || !document.createElement) {
			return escapeHtml(html);
		}
		var tpl = document.createElement('template');
		tpl.innerHTML = String(html == null ? '' : html);
		scrub(tpl.content);
		return tpl.innerHTML;
	}

	// ── Public render ──────────────────────────────────────────

	/// Render markdown `text` to a sanitised HTML string.
	function md(text) {
		var src = (text == null) ? '' : String(text);
		var html;
		try {
			html = marked.parse(src, { breaks: true });
		} catch (e) {
			return escapeHtml(src);
		}
		// Sanitise the model-authored markup first, then apply the
		// trusted code-block transform (which builds its own markup
		// from already-escaped source).
		try {
			html = sanitize(html);
		} catch (e) { return escapeHtml(src); }
		try {
			html = enhanceCodeBlocks(html);
		} catch (e) { /* keep unenhanced html */ }
		return html;
	}

	// ── Copy-to-clipboard (event delegation) ───────────────────

	/// Copy `text` to the clipboard, with a legacy fallback for
	/// browsers without the async clipboard API.
	function copyText(text) {
		if (navigator.clipboard && navigator.clipboard.writeText) {
			return navigator.clipboard.writeText(text);
		}
		return new Promise(function (resolve, reject) {
			try {
				var ta = document.createElement('textarea');
				ta.value = text;
				ta.setAttribute('readonly', '');
				ta.style.position = 'fixed';
				ta.style.left = '-9999px';
				ta.style.opacity = '0';
				document.body.appendChild(ta);
				ta.select();
				var ok = document.execCommand('copy');
				document.body.removeChild(ta);
				if (ok) { resolve(); } else { reject(new Error('copy failed')); }
			} catch (e) { reject(e); }
		});
	}

	// A single delegated listener services every copy button, present
	// or future, so app.js needs no per-button wiring.
	document.addEventListener('click', function (ev) {
		var btn = ev.target;
		if (!btn || !btn.classList || !btn.classList.contains('code-copy-btn')) return;
		var code = btn.getAttribute('data-code') || '';
		var restore = function (label) {
			btn.textContent = label;
			setTimeout(function () {
				btn.textContent = 'Copy';
				btn.classList.remove('copied');
			}, 1400);
		};
		copyText(code).then(function () {
			btn.classList.add('copied');
			restore('Copied');
		}, function () {
			restore('Failed');
		});
	});

	// ── Export ─────────────────────────────────────────────────
	window.RedRender = { md: md, escapeHtml: escapeHtml, sanitize: sanitize };
})();
