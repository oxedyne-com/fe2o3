// authoring.js -- in-browser annotation authoring for the Pearl reader.
//
// This closes the annotations loop the reader only read before: with "Annotate" on, a drag over a page
// creates a highlight or a note, it renders at once through the reader's own `Pearl.renderAnnotation`, and
// "Save .prl" writes the document back with the new annotations so a reload shows them.
//
// Persistence is an updated `.prl`, not a sidecar. The reader's jdat parser strips a scalar's type tag, so
// re-serialising the whole document from the parsed model could not know each number's tag and the Rust
// decoder would reject the wrong one. Annotations are the one section authored here, with every atom's tag
// known, so `jdat.js`'s serialiser re-emits just the `annotations` list and this layer splices it back
// into the original document text byte for byte -- the glyph, block and geometry bytes are never touched,
// and the Rust reader (`PearlDoc::annotations`) reads the result back unchanged.

"use strict";

const Authoring = (() => {
	const SP_PER_PT = 65536;

	const state = {
		on:        false,      // is annotate mode engaged?
		container: null,       // the #pages element
		doc:       null,       // the current parsed document model
		name:      "document.prl",
		author:    "me",
		onChange:  null,       // called with the live annotation count after an edit
	};

	// ---- geometry: a page's on-screen box back to scaled points on that page -------------------------

	// The points-per-pixel scale for a page's SVG: its viewBox width (points) over its rendered CSS width.
	// It is 1 when the page is shown at natural size, and corrects for any responsive shrink.
	function pageScale(svg) {
		const box = svg.getBoundingClientRect();
		const vb  = svg.viewBox && svg.viewBox.baseVal ? svg.viewBox.baseVal.width : 0;
		if (!vb || !box.width) return 1;
		return vb / box.width;
	}

	// A client-space rectangle over a page converted to `[x, y, w, h]` scaled points on that page: px ->
	// pt (x scale) -> x SP_PER_PT, the exact inverse of the reader's `sp()` render transform.
	function rectToSp(svg, left, top, width, height) {
		const box   = svg.getBoundingClientRect();
		const scale = pageScale(svg);
		const toSp  = px => Math.round(px * scale * SP_PER_PT);
		return [
			toSp(left - box.left),
			toSp(top  - box.top),
			toSp(width),
			toSp(height),
		];
	}

	// ---- drawing a selection over a page -------------------------------------------------------------

	let drag = null; // { page, svg, overlay, startX, startY, preview } while a drag is in flight

	function onMouseDown(ev) {
		if (!state.on || ev.button !== 0) return;
		if (!ev.target.closest) return;
		// A press inside the composer is the user filling the form, not the start of a new region.
		if (ev.target.closest(".pearl-composer")) return;
		const page = ev.target.closest(".pearl-page");
		if (!page) return;
		const svg     = page.querySelector("svg");
		const overlay = page.querySelector(".pearl-overlay");
		if (!svg || !overlay) return;

		ev.preventDefault();
		const preview = document.createElement("div");
		preview.className = "pearl-draw";
		overlay.appendChild(preview);
		drag = { page, svg, overlay, startX: ev.clientX, startY: ev.clientY, preview };
		updatePreview(ev.clientX, ev.clientY);

		window.addEventListener("mousemove", onMouseMove);
		window.addEventListener("mouseup", onMouseUp);
	}

	function boxFrom(d, x, y) {
		return {
			left:   Math.min(d.startX, x),
			top:    Math.min(d.startY, y),
			width:  Math.abs(x - d.startX),
			height: Math.abs(y - d.startY),
		};
	}

	function updatePreview(x, y) {
		const ov = drag.overlay.getBoundingClientRect();
		const b  = boxFrom(drag, x, y);
		drag.preview.style.left   = (b.left - ov.left) + "px";
		drag.preview.style.top    = (b.top  - ov.top)  + "px";
		drag.preview.style.width  = b.width  + "px";
		drag.preview.style.height = b.height + "px";
	}

	function onMouseMove(ev) {
		if (drag) updatePreview(ev.clientX, ev.clientY);
	}

	function onMouseUp(ev) {
		window.removeEventListener("mousemove", onMouseMove);
		window.removeEventListener("mouseup", onMouseUp);
		if (!drag) return;
		const d = drag;
		drag = null;
		d.preview.remove();

		const b = boxFrom(d, ev.clientX, ev.clientY);
		// A tiny gesture is a click, not a drag: offer a note placed at that point rather than a highlight.
		const tiny = b.width < 5 && b.height < 5;
		openComposer(d, b, tiny);
	}

	// ---- the composer popover ------------------------------------------------------------------------

	let composer = null;

	function closeComposer() {
		if (composer) { composer.remove(); composer = null; }
	}

	// Offers kind, payload and author for the drawn region, then commits the annotation on Save. `tiny`
	// steers the default kind to a note (a click) rather than a highlight (a drag).
	function openComposer(d, box, tiny) {
		closeComposer();
		const ov = d.overlay.getBoundingClientRect();

		const el = document.createElement("div");
		el.className = "pearl-composer";
		el.style.left = Math.max(4, box.left - ov.left) + "px";
		el.style.top  = (box.top - ov.top + box.height + 8) + "px";
		el.innerHTML = `
			<div class="pc-row pc-kind">
				<label><input type="radio" name="pc-kind" value="highlight" ${tiny ? "" : "checked"}> Highlight</label>
				<label><input type="radio" name="pc-kind" value="note" ${tiny ? "checked" : ""}> Note</label>
			</div>
			<textarea class="pc-payload" rows="2" placeholder="Text (a note's words, or a highlight's label)"></textarea>
			<div class="pc-row">
				<input class="pc-author" type="text" placeholder="author">
				<span class="pc-sp"></span>
				<button class="pc-cancel" type="button">Cancel</button>
				<button class="pc-save" type="button">Save</button>
			</div>`;
		d.overlay.appendChild(el);
		composer = el;

		const authorInput = el.querySelector(".pc-author");
		authorInput.value = state.author;
		const payload = el.querySelector(".pc-payload");
		payload.focus();

		el.querySelector(".pc-cancel").addEventListener("click", closeComposer);
		el.querySelector(".pc-save").addEventListener("click", () => {
			const kind   = el.querySelector('input[name="pc-kind"]:checked').value;
			const author = authorInput.value.trim() || "me";
			state.author = author;
			const ann = {
				anchor:  d.page.dataset.block,
				kind,
				payload: payload.value,
				author,
				created: new Date().toISOString(),
			};
			// A highlight carries its region; a note lives in the margin, so it stores no rect.
			if (kind === "highlight") {
				ann.rect = rectToSp(d.svg, box.left, box.top, box.width, box.height);
			}
			commit(ann, d.overlay);
			closeComposer();
		});
	}

	// Adds the annotation to the model and paints it at once through the reader's own render path.
	function commit(ann, overlay) {
		if (!state.doc.annotations) state.doc.annotations = [];
		state.doc.annotations.push(ann);
		if (window.Pearl && Pearl.renderAnnotation) Pearl.renderAnnotation(ann, overlay);
		if (state.onChange) state.onChange(state.doc.annotations.length);
	}

	// ---- persistence: re-emit the annotations section and splice it into the original text -----------

	// One annotation as a tagged jdat value, its atom tags matching the Rust `Annotation::to_dat`: strings
	// for the five text fields, and a rect of four `(i32|..)` scaled lengths when present.
	function annToDat(ann) {
		const m = {
			anchor:  ann.anchor,
			kind:    ann.kind,
			payload: ann.payload || "",
			author:  ann.author || "me",
			created: ann.created || "",
		};
		if (ann.rect) {
			m.rect = ann.rect.map(Jdat.i32);
		}
		return Jdat.omap(m);
	}

	// The scan skips over the contents of a jdat string so a `"` or bracket inside a payload cannot fool
	// the bracket matcher. Returns the index just past the `]` that closes the list opened at `start`.
	function matchBracket(src, start) {
		let i = start, depth = 0, inStr = false, esc = false;
		while (i < src.length) {
			const c = src[i];
			if (inStr) {
				if (esc) esc = false;
				else if (c === "\\") esc = true;
				else if (c === '"') inStr = false;
			} else {
				if (c === '"') inStr = true;
				else if (c === "[" || c === "{") depth++;
				else if (c === "]" || c === "}") { depth--; if (depth === 0) return i + 1; }
			}
			i++;
		}
		throw new Error("authoring: unterminated list while splicing annotations");
	}

	// Replaces the top-level `annotations` list in `src` with `listText`, leaving every other byte as it
	// was. The key is found at brace-depth 1 -- directly inside the top omap -- so an `"annotations"` that
	// happened to sit inside a payload string or a nested map is never mistaken for it.
	function spliceAnnotations(src, listText) {
		const KEY = '"annotations"';
		let i = 0, depth = 0, inStr = false, esc = false;
		while (i < src.length) {
			const c = src[i];
			if (inStr) {
				if (esc) esc = false;
				else if (c === "\\") esc = true;
				else if (c === '"') inStr = false;
				i++;
				continue;
			}
			if (c === '"') {
				if (depth === 1 && src.startsWith(KEY, i)) {
					let j = i + KEY.length;
					while (j < src.length && /\s/.test(src[j])) j++;
					if (src[j] === ":") {
						j++;
						while (j < src.length && /\s/.test(src[j])) j++;
						if (src[j] === "[") {
							const end = matchBracket(src, j);
							return src.slice(0, j) + listText + src.slice(end);
						}
					}
				}
				inStr = true; i++; continue;
			}
			if (c === "[" || c === "{") depth++;
			else if (c === "]" || c === "}") depth--;
			i++;
		}
		// No annotations section (a file older than the field): insert one before the top omap's close.
		const close = src.lastIndexOf("}");
		if (close < 0) throw new Error("authoring: no document map to splice into");
		return src.slice(0, close) + `, "annotations": ${listText}` + src.slice(close);
	}

	// The current document re-serialised with its live annotations, as `.prl` text. Exposed so a headless
	// check can read the bytes a download would write, without a real file dialog.
	function buildUpdatedSource() {
		if (!state.doc || !state.doc.__source) throw new Error("authoring: no document loaded");
		const anns = state.doc.annotations || [];
		const listText = Jdat.encode(anns.map(annToDat));
		return spliceAnnotations(state.doc.__source, listText);
	}

	function save() {
		const text = buildUpdatedSource();
		const blob = new Blob([text], { type: "application/octet-stream" });
		const url  = URL.createObjectURL(blob);
		const a    = document.createElement("a");
		a.href = url;
		a.download = state.name;
		document.body.appendChild(a);
		a.click();
		a.remove();
		setTimeout(() => URL.revokeObjectURL(url), 1000);
	}

	// ---- wiring --------------------------------------------------------------------------------------

	function setMode(on) {
		state.on = on;
		if (state.container) state.container.classList.toggle("annotating", on);
		if (!on) closeComposer();
	}

	function setDocument(doc, name) {
		state.doc = doc;
		if (name) state.name = name;
	}

	function init(opts) {
		state.container = opts.container;
		state.onChange  = opts.onChange || null;
		state.author    = (opts.authorInput && opts.authorInput.value) || "me";

		if (opts.authorInput) {
			opts.authorInput.value = state.author;
			opts.authorInput.addEventListener("input", () => {
				state.author = opts.authorInput.value.trim() || "me";
			});
		}
		if (opts.toggle) {
			opts.toggle.addEventListener("click", () => {
				setMode(!state.on);
				opts.toggle.classList.toggle("on", state.on);
				opts.toggle.textContent = state.on ? "Annotating…" : "Annotate";
			});
		}
		if (opts.save) opts.save.addEventListener("click", save);

		state.container.addEventListener("mousedown", onMouseDown);
	}

	return { init, setDocument, setMode, save, buildUpdatedSource,
		get annotations() { return (state.doc && state.doc.annotations) || []; } };
})();

window.Authoring = Authoring;
