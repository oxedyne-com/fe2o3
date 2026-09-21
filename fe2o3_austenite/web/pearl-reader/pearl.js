// pearl.js -- a browser reader for the Pearl (.prl) document format.
//
// It renders a Pearl document to inline SVG from the format's own data model -- glyph outlines stored
// once, placed per leaf, plus fills, strokes, rules and rasters -- reproducing the transform the
// Austenite SVG arm applies (see fe2o3_austenite/src/emit/pearl.rs render_page and
// src/emit/svg.rs draw_text/run_text_layer). The goal is pixel parity with that arm's SVG, which already
// matches the PDF.
//
// A `text` leaf's fields past its rigid geometry and outline glyphs (size, selectable spans, the
// optional colour) ride in leaf[7], a v1 self-describing keyed object -- see pearl.rs's own comment on
// why a positional tail was dropped. `spans` there is the same cluster-to-source-text mapping the Rust
// SVG and PDF writers derive from `ShapedText::glyph_text`, so this reader's selectable `.tsel` layer
// agrees with both of them about what each glyph says.
//
// Transport: the reader fetches the .prl and parses its text jdat directly in the browser (jdat.js) --
// there is no JSON projection any more. Nothing here is pre-rendered: the page is built from the
// outlines and placements the .prl carries. A typed jdat atom decodes to the same plain value its old
// JSON projection did -- an omap to an object, a list to an array, u32/i32/u8/f32 to a Number, a base64
// PNG to a string -- so the renderer below is unchanged from the JSON-fed first cut.

"use strict";

// One point is 65536 scaled points (Sp), as in TeX; every stored length is an Sp integer.
const SP_PER_PT = 65536;
const sp = v => v / SP_PER_PT;

// A colour list [r, g, b, a] -> "#rrggbb", matching the Rust `rgb()` helper.
function rgb(c) {
	const h = n => n.toString(16).padStart(2, "0");
	return "#" + h(c[0]) + h(c[1]) + h(c[2]);
}

// The fill/stroke opacity string, only written when the colour is not opaque, as `opacity()` does.
function opacity(c) { return (c[3] / 255).toFixed(3); }

const SVGNS = "http://www.w3.org/2000/svg";

function el(name, attrs) {
	const e = document.createElementNS(SVGNS, name);
	for (const k in attrs) {
		if (attrs[k] !== null && attrs[k] !== undefined) e.setAttribute(k, attrs[k]);
	}
	return e;
}

// A `<tspan>` at (x, y) in `size`, carrying `text` -- the selectable text layer's one building block,
// used for both a run's own glyph spans and the synthetic interword space between two runs.
function tspanEl(x, y, size, text) {
	const t = el("tspan", { x, y, "font-size": size });
	t.textContent = text;
	return t;
}

// Renders one page block to an <svg> element, reproducing the SVG arm leaf by leaf.
function renderPage(doc, blockKey) {
	const block  = doc.blocks[blockKey];
	const glyphs = doc.glyphs;
	const images = doc.images;

	// The viewport is the media box: geometry width/height rounded to whole points.
	const geom = block.geom;
	const w = Math.round(sp(geom[0]));
	const h = Math.round(sp(geom[1]));

	const svg = el("svg", {
		xmlns:   SVGNS,
		width:   w,
		height:  h,
		viewBox: `0 0 ${w} ${h}`,
	});
	svg.appendChild(el("rect", { x: 0, y: 0, width: w, height: h, fill: "#ffffff" }));

	// The selectable text layer's style, matching the SVG arm's own <style> declaration verbatim.
	const tselStyle = el("style", {});
	tselStyle.textContent = ".tsel { fill: transparent; }";
	svg.appendChild(tselStyle);

	// Gathered across every "text" leaf below into ONE page-wide <text>, appended once at the end --
	// see svg.rs's run_text_layer for why one element per run breaks a browser's cross-element search. A
	// leading space precedes every run but the page's first, standing in for the interword gap Austenite's
	// line breaker leaves as pure position rather than a glyph.
	const tsel = el("text", { class: "tsel" });
	let tselHasText = false;

	for (const leaf of block.leaves) {
		const tag = leaf[0];
		switch (tag) {
			case "text": {
				// base_x = x, base_y = y + height (the baseline); each glyph is the stored outline
				// flipped in y and moved to (base_x + gx, base_y - gy).
				const baseX = sp(leaf[1]);
				const baseY = sp(leaf[2]) + sp(leaf[4]);
				// A leaf without a `colour` key is black, the form every pre-colour text leaf took --
				// matching the Rust reader's own default at pearl.rs's `colour` lookup.
				const meta = leaf[7];
				const c = meta && meta.colour;
				for (const g of leaf[6]) {
					const entry = glyphs[g[0]];
					const d = entry && entry.d;
					if (!d) continue; // A space carries an advance but no ink.
					const tx = baseX + g[1];
					const ty = baseY - g[2];
					svg.appendChild(el("path", {
						d,
						transform: `matrix(1,0,0,-1,${tx},${ty})`,
						fill: c ? rgb(c) : "#000000",
						"fill-opacity": c && c[3] < 255 ? opacity(c) : null,
					}));
				}
				// The run's selectable twin: leaf[7].spans maps each text-bearing glyph (spaces included)
				// to its source text, positioned exactly as its outline was above.
				const runSpans = (meta && meta.spans) || [];
				if (runSpans.length > 0) {
					if (tselHasText) tsel.appendChild(tspanEl(baseX, baseY, meta.size, " "));
					for (const s of runSpans) {
						tsel.appendChild(tspanEl(baseX + s[0], baseY - s[1], meta.size, s[2]));
					}
					tselHasText = true;
				}
				break;
			}
			case "rule":
			case "reserved": {
				const x0 = sp(leaf[1]);
				const y0 = sp(leaf[2]);
				const x1 = sp(leaf[1] + leaf[3]);
				const y1 = sp(leaf[2] + leaf[4] + leaf[5]); // y + height + depth
				if (x1 <= x0 || y1 <= y0) continue; // A zero-area box draws nothing.
				const rectAttrs = { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
				if (tag === "rule") {
					svg.appendChild(el("rect", { ...rectAttrs, fill: "#000000" }));
				} else {
					// A reservation: a half-point grey stroke, pen grey = (176,176,176).
					svg.appendChild(el("rect", {
						...rectAttrs,
						fill: "none",
						stroke: "#b0b0b0",
						"stroke-width": 0.5,
						"stroke-linecap": "butt",
						"stroke-linejoin": "miter",
						"stroke-miterlimit": 4,
					}));
				}
				break;
			}
			case "fill": {
				// A pre-translated path: draw the d string at (bx, by), filled with its colour.
				const c = leaf[4];
				svg.appendChild(el("path", {
					d: leaf[3],
					transform: `translate(${sp(leaf[1])},${sp(leaf[2])})`,
					fill: rgb(c),
					"fill-opacity": c[3] < 255 ? opacity(c) : null,
				}));
				break;
			}
			case "stroke": {
				const c = leaf[4];
				svg.appendChild(el("path", {
					d: leaf[3],
					transform: `translate(${sp(leaf[1])},${sp(leaf[2])})`,
					fill: "none",
					stroke: rgb(c),
					"stroke-opacity": c[3] < 255 ? opacity(c) : null,
					"stroke-width": leaf[5],
					"stroke-linecap": "butt",
					"stroke-linejoin": "miter",
					"stroke-miterlimit": 4,
				}));
				break;
			}
			case "image": {
				// The raster's frame is the page's own, top-left, y down, so the box is placed directly.
				const ox = sp(leaf[1]);
				const oy = sp(leaf[2]);
				const gx = leaf[3], gy = leaf[4], iw = leaf[5], ih = leaf[6];
				const entry = images[leaf[7]];
				svg.appendChild(el("image", {
					x: ox + gx,
					y: oy + gy,
					width: iw,
					height: ih,
					preserveAspectRatio: "none",
					href: "data:image/png;base64," + entry.png,
				}));
				break;
			}
			case "link":
				// A link leaf places no ink -- it is a hotspot, drawn by the overlay layer, not the SVG.
				break;
			default:
				console.warn("Unknown Pearl leaf kind:", tag);
		}
	}
	if (tselHasText) svg.appendChild(tsel);
	return svg;
}

// ---------------------------------------------------------------------------------------------------
// Links: reading the `link` leaves off a page, and resolving a target the way `PearlDoc::resolve_link`
// does -- an external uri stands as its address; an internal anchor goes through the shipped ledger to a
// page, then through the index to that page's content-addressed block.
// ---------------------------------------------------------------------------------------------------

// The `link` leaves on the page at `idx`: each carries a rectangle in scaled points and a target, in the
// order they were emitted, mirroring `PearlDoc::links_on_page`.
function linksOnPage(doc, idx) {
	const entry = doc.index[idx];
	const block = doc.blocks[entry.block];
	const out   = [];
	for (const leaf of block.leaves) {
		if (leaf[0] !== "link") continue;
		out.push({ x: leaf[1], y: leaf[2], w: leaf[3], h: leaf[4], target: leaf[5] });
	}
	return out;
}

// A stored link target -- `["uri", addr]` or `["anchor", { kind, key }]` -- resolved to where it points.
// Returns { kind: "uri", uri } for an external target; { kind: "block", block, page } for an internal one
// the ledger has fixed; or null for a dangling cross-reference, exactly as `resolve_link` returns `None`.
function resolveLink(doc, target) {
	const tag = target[0];
	if (tag === "uri") {
		return { kind: "uri", uri: target[1] };
	}
	if (tag === "anchor") {
		const id     = target[1];              // { kind: <u8 tag>, key: <string> }
		const ledger = doc.ledger;
		const anchor = (ledger.anchors || []).find(a => a.id.kind === id.kind && a.id.key === id.key);
		if (!anchor) return null;              // the ledger has not fixed this anchor
		const page = anchor.page;
		const hit  = doc.index.find(e => e.page === page);
		if (!hit) return null;                 // the anchor's page is not one the index holds
		return { kind: "block", block: hit.block, page };
	}
	console.warn("Unknown Pearl link-target kind:", tag);
	return null;
}

// The annotations anchored to a given block hash, in the order they were added. A file written before the
// annotations section existed simply carries none.
function annotationsForBlock(doc, blockHash) {
	return (doc.annotations || []).filter(a => a.anchor === blockHash);
}

// ---------------------------------------------------------------------------------------------------
// The outline: the heading tree carried in the `.prl` header, and the ledger lookup that fixes each
// heading to a page and a y -- the same anchor resolution `resolveLink` performs for a cross-reference,
// so a table-of-contents entry and a link to the same heading land in the same place.
// ---------------------------------------------------------------------------------------------------

// Resolves a stored anchor `{ kind, key }` through the shipped ledger to `{ page, y }` -- the 1-based
// page and the y within it, both as the ledger recorded them -- or null when the ledger never fixed it,
// exactly the dangling case `resolveLink` returns null for. Unlike `resolveLink` this keeps the y, so a
// jump lands on the heading's own line rather than the page top.
function resolveAnchor(doc, anchor) {
	const a = (doc.ledger.anchors || []).find(x => x.id.kind === anchor.kind && x.id.key === anchor.key);
	if (!a) return null;
	return { page: a.page, y: a.y };
}

// The document's heading outline as an array of `{ level, number, title, page, y }`, each entry resolved
// through the ledger, mirroring `PearlDoc::outline` on the Rust side. `page`/`y` are null for a heading
// the ledger never fixed. A `.prl` without an outline section (a headless manuscript, or a file that
// predates the field) yields an empty array.
function outlineEntries(doc) {
	return (doc.outline || []).map(e => {
		const loc = resolveAnchor(doc, e.anchor);
		return {
			level:  e.level,
			number: e.number || "",
			title:  e.title || "",
			page:   loc ? loc.page : null,
			y:      loc ? loc.y : null,
		};
	});
}

// ---------------------------------------------------------------------------------------------------
// Rendering the document, plus an overlay layer per page carrying link hotspots and annotations. The
// SVG is authored in points and drawn at 1 user unit = 1 px (its width/height attributes are the point
// dimensions), so a scaled-point length converts to a CSS pixel through `sp()` alone -- no page scale to
// track.
// ---------------------------------------------------------------------------------------------------

function renderDocument(doc, container) {
	container.innerHTML = "";
	// A version mismatch is refused outright, matching the Rust reader's own `PearlDoc::from_string`
	// check -- a v0 file must fail loudly rather than render silently with no `.tsel` layer.
	if (doc.pearl !== "1") {
		throw new Error(`This reader speaks Pearl v1, but the file is v${doc.pearl}.`);
	}

	// Build every page first, keeping the DOM node beside its index entry so an internal link can scroll
	// its target block into view.
	const pageEls = [];
	doc.index.forEach((entry, idx) => {
		const page = document.createElement("div");
		page.className = "pearl-page";
		page.dataset.block = entry.block;
		page.appendChild(renderPage(doc, entry.block));

		const overlay = document.createElement("div");
		overlay.className = "pearl-overlay";
		page.appendChild(overlay);

		container.appendChild(page);
		pageEls.push(page);

		addLinks(doc, idx, overlay, container);
		addAnnotations(doc, entry.block, overlay);
	});
	return pageEls;
}

// Lays a clickable hotspot over each link leaf: an external uri opens in a new tab; an internal anchor
// resolves and scrolls the target page's block into view. Each hotspot shows a subtle box-and-underline
// so a reader can see it is a link, the affordance the SVG arm draws no ink for.
function addLinks(doc, idx, overlay, container) {
	for (const link of linksOnPage(doc, idx)) {
		const res  = resolveLink(doc, link.target);
		const spot = document.createElement("a");
		spot.className = "pearl-link" + (res && res.kind === "uri" ? " ext" : " int");
		spot.style.left   = sp(link.x) + "px";
		spot.style.top    = sp(link.y) + "px";
		spot.style.width  = sp(link.w) + "px";
		spot.style.height = sp(link.h) + "px";

		if (res && res.kind === "uri") {
			spot.href   = res.uri;
			spot.target = "_blank";
			spot.rel    = "noopener";
			spot.title  = res.uri;
			console.log(`link (page ${idx + 1}): external -> ${res.uri}`);
		} else if (res && res.kind === "block") {
			spot.href  = "#";
			spot.title = `page ${res.page}`;
			spot.addEventListener("click", (ev) => {
				ev.preventDefault();
				const tgt = container.querySelector(`.pearl-page[data-block="${res.block}"]`);
				if (tgt) tgt.scrollIntoView({ behavior: "smooth", block: "start" });
			});
			console.log(`link (page ${idx + 1}): internal -> block ${res.block.slice(0, 8)}… on page ${res.page}`);
		} else {
			// A dangling cross-reference: mark it, but do not pretend it goes anywhere.
			spot.className += " dead";
			spot.title = "unresolved link";
			console.warn(`link (page ${idx + 1}): unresolved target`, link.target);
		}
		overlay.appendChild(spot);
	}
}

// Draws the annotations anchored to this page's block, in order. Each is placed by `renderAnnotation`,
// which the authoring layer also calls to show a freshly created annotation without a full repaint.
function addAnnotations(doc, blockHash, overlay) {
	for (const ann of annotationsForBlock(doc, blockHash)) {
		renderAnnotation(ann, overlay);
	}
}

// Places a single annotation into a page's overlay: a `highlight` is a translucent rectangle over its
// `rect` (or a left-edge band when it has none); a `note` is a margin marker that reveals its payload and
// author on click. Notes stack down the margin, the running row kept on the overlay so a later addition
// lands below the ones already there.
function renderAnnotation(ann, overlay) {
	if (ann.kind === "highlight") {
		const r = ann.rect;
		const box = document.createElement("div");
		box.className = "pearl-highlight";
		if (r) {
			box.style.left   = sp(r[0]) + "px";
			box.style.top    = sp(r[1]) + "px";
			box.style.width  = sp(r[2]) + "px";
			box.style.height = sp(r[3]) + "px";
		} else {
			// A whole-block highlight: a thin band down the page's left edge, so it is visible but does
			// not blanket the text.
			box.style.left = "0"; box.style.top = "0"; box.style.width = "6px"; box.style.height = "100%";
		}
		if (ann.payload) box.title = ann.payload;
		overlay.appendChild(box);
	} else if (ann.kind === "note") {
		const noteRow = overlay._noteRow || 0;
		overlay._noteRow = noteRow + 1;

		const marker = document.createElement("button");
		marker.className = "pearl-note";
		marker.textContent = "✎"; // a pencil, the note affordance
		marker.style.top = (18 + noteRow * 30) + "px";

		const bubble = document.createElement("div");
		bubble.className = "pearl-note-bubble";
		bubble.innerHTML =
			`<div class="pearl-note-text"></div><div class="pearl-note-meta"></div>`;
		bubble.querySelector(".pearl-note-text").textContent = ann.payload;
		bubble.querySelector(".pearl-note-meta").textContent =
			`${ann.author || "unknown"} · ${ann.created || ""}`;
		marker.addEventListener("click", () => {
			bubble.classList.toggle("open");
		});
		marker.appendChild(bubble);
		overlay.appendChild(marker);
	}
}

// Parses a .prl's text jdat into the document model and renders it into `container`. The source text is
// kept on the returned model as `__source`, so the authoring layer can splice an updated annotations
// section back into the original document byte for byte (see authoring.js).
function renderText(text, container) {
	const doc = Jdat.parse(text);
	doc.__source = text;
	renderDocument(doc, container);
	return doc;
}

// Fetches a .prl by URL, then parses and renders it.
async function loadAndRender(url, container) {
	const res = await fetch(url, { cache: "no-store" });
	if (!res.ok) throw new Error(`Failed to load ${url}: ${res.status}`);
	return renderText(await res.text(), container);
}

window.Pearl = {
	renderDocument, renderPage, renderText, loadAndRender,
	linksOnPage, resolveLink, annotationsForBlock, renderAnnotation,
	resolveAnchor, outlineEntries,
	sp, SP_PER_PT,
};
