// pearl.js -- a browser reader for the Pearl (.prl) document format.
//
// It renders a Pearl document to inline SVG from the format's own data model -- glyph outlines stored
// once, placed per leaf, plus fills, strokes, rules and rasters -- reproducing the transform the
// Austenite SVG arm applies (see fe2o3_austenite/src/emit/pearl.rs render_page and
// src/emit/svg.rs draw_text). The goal is pixel parity with that arm's SVG, which already matches the
// PDF.
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

	for (const leaf of block.leaves) {
		const tag = leaf[0];
		switch (tag) {
			case "text": {
				// base_x = x, base_y = y + height (the baseline); each glyph is the stored outline
				// flipped in y and moved to (base_x + gx, base_y - gy).
				const baseX = sp(leaf[1]);
				const baseY = sp(leaf[2]) + sp(leaf[4]);
				for (const g of leaf[6]) {
					const entry = glyphs[g[0]];
					const d = entry && entry.d;
					if (!d) continue; // A space carries an advance but no ink.
					const tx = baseX + g[1];
					const ty = baseY - g[2];
					svg.appendChild(el("path", {
						d,
						transform: `matrix(1,0,0,-1,${tx},${ty})`,
						fill: "#000000",
					}));
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
// Rendering the document, plus an overlay layer per page carrying link hotspots and annotations. The
// SVG is authored in points and drawn at 1 user unit = 1 px (its width/height attributes are the point
// dimensions), so a scaled-point length converts to a CSS pixel through `sp()` alone -- no page scale to
// track.
// ---------------------------------------------------------------------------------------------------

function renderDocument(doc, container) {
	container.innerHTML = "";
	if (doc.pearl !== "0") {
		console.warn("This reader speaks Pearl v0; file is v" + doc.pearl);
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

// Draws the annotations anchored to this page's block: a `highlight` is a translucent rectangle over its
// `rect` (or the whole page when it has none); a `note` is a margin marker that reveals its payload and
// author on click.
function addAnnotations(doc, blockHash, overlay) {
	let noteRow = 0;
	for (const ann of annotationsForBlock(doc, blockHash)) {
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
			const marker = document.createElement("button");
			marker.className = "pearl-note";
			marker.textContent = "✎"; // a pencil, the note affordance
			marker.style.top = (18 + noteRow * 30) + "px";
			noteRow++;

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
}

// Parses a .prl's text jdat into the document model and renders it into `container`.
function renderText(text, container) {
	const doc = Jdat.parse(text);
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
	linksOnPage, resolveLink, annotationsForBlock,
};
