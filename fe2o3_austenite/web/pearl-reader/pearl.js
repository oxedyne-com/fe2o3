// pearl.js -- a first-cut browser reader for the Pearl (.prl) document format.
//
// It renders a Pearl document to inline SVG from the format's own data model -- glyph outlines stored
// once, placed per leaf, plus fills, strokes, rules and rasters -- reproducing the transform the
// Austenite SVG arm applies (see fe2o3_austenite/src/emit/pearl.rs render_page and
// src/emit/svg.rs draw_text). The goal is pixel parity with that arm's SVG, which already matches the
// PDF.
//
// Transport: the .prl is text jdat; the `pearl_json` companion binary re-encodes the identical Dat as
// JSON so the browser can JSON.parse it. Nothing here is pre-rendered -- the page is built from
// outlines and placements.

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
			default:
				console.warn("Unknown Pearl leaf kind:", tag);
		}
	}
	return svg;
}

// Renders every page in the document's index into `container`.
function renderDocument(doc, container) {
	container.innerHTML = "";
	if (doc.pearl !== "0") {
		console.warn("This reader speaks Pearl v0; file is v" + doc.pearl);
	}
	for (const entry of doc.index) {
		const page = document.createElement("div");
		page.className = "pearl-page";
		page.appendChild(renderPage(doc, entry.block));
		container.appendChild(page);
	}
}

async function loadAndRender(url, container) {
	const res = await fetch(url, { cache: "no-store" });
	if (!res.ok) throw new Error(`Failed to load ${url}: ${res.status}`);
	const doc = await res.json();
	renderDocument(doc, container);
	return doc;
}

window.Pearl = { renderDocument, renderPage, loadAndRender };
