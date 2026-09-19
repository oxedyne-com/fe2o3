// jdat.js -- a minimal browser parser for the text-jdat subset a Pearl (.prl) document uses.
//
// A .prl is text jdat: an ordered map of ordered maps, lists, strings and typed scalar atoms. This
// parses exactly the subset the Pearl v1 writer emits (see fe2o3_austenite/src/emit/pearl.rs) and
// returns plain JavaScript values -- objects, arrays, strings and numbers -- so the renderer consumes
// the real document with no JSON projection in between.
//
// The subset, confirmed against a real .prl:
//   (omap|{ "k": v, ... })   an ordered map          -> a plain object, insertion order preserved
//   [ v, v, ... ]            a list                   -> an array
//   "..."                    a string (RFC 8259 escapes) -> a string
//   (u32|N) (i32|N) (u8|N)   integer atoms            -> a Number
//   (f32|X.YeZ)              a float atom (scientific) -> a Number
//
// There are no byte-strings, bools or nulls in a v1 .prl: a raster's PNG rides as a base64 *string*
// (the writer stores `base64::encode(png)`), so the whole file is these five shapes. Unknown type tags
// and unknown map keys are tolerated -- a parallel lane may add fields -- by stripping the tag and
// keeping the value, and by never assuming a fixed key set.

"use strict";

// A recursive-descent parser over the source string, tracking one cursor.
function parseJdat(src) {
	let i = 0;
	const n = src.length;

	function isWs(c) { return c === " " || c === "\t" || c === "\n" || c === "\r"; }

	function skipWs() {
		while (i < n && isWs(src[i])) i++;
	}

	function fail(msg) {
		const near = src.slice(Math.max(0, i - 20), i + 20);
		throw new Error(`jdat parse error at ${i}: ${msg} -- near "${near}"`);
	}

	// A value is a typed atom, a map, a list, a string, or a bare number.
	function value() {
		skipWs();
		if (i >= n) fail("unexpected end of input");
		const c = src[i];
		if (c === "(") return typed();
		if (c === "{") return map();
		if (c === "[") return list();
		if (c === '"') return str();
		return number();
	}

	// A typed atom: '(' TAG '|' inner ')'. The tag is stripped -- an omap's inner is a map, a scalar's
	// inner is a bare number -- so the caller sees the plain value either way.
	function typed() {
		i++; // (
		const tagStart = i;
		while (i < n && src[i] !== "|" && src[i] !== ")") i++;
		if (i >= n) fail("unterminated typed atom");
		// A tag with no '|' (no payload separator) is not a shape this subset uses.
		if (src[i] === ")") fail("typed atom without a '|' payload");
		i++; // |
		const inner = value();
		skipWs();
		if (src[i] !== ")") fail("typed atom not closed with ')'");
		i++; // )
		return inner;
	}

	// A map body: '{' ( "key" : value (',' ...)* )? '}'. Keys are always quoted strings here.
	function map() {
		i++; // {
		const obj = {};
		skipWs();
		if (src[i] === "}") { i++; return obj; }
		for (;;) {
			skipWs();
			if (src[i] !== '"') fail("map key is not a string");
			const key = str();
			skipWs();
			if (src[i] !== ":") fail("map key not followed by ':'");
			i++; // :
			obj[key] = value();
			skipWs();
			if (src[i] === ",") { i++; continue; }
			if (src[i] === "}") { i++; break; }
			fail("expected ',' or '}' in map");
		}
		return obj;
	}

	// A list: '[' ( value (',' ...)* )? ']'.
	function list() {
		i++; // [
		const arr = [];
		skipWs();
		if (src[i] === "]") { i++; return arr; }
		for (;;) {
			arr.push(value());
			skipWs();
			if (src[i] === ",") { i++; continue; }
			if (src[i] === "]") { i++; break; }
			fail("expected ',' or ']' in list");
		}
		return arr;
	}

	// A quoted string with RFC 8259 escapes. The .prl's own strings (hex keys, slug keys, SVG path data
	// and base64) carry no escapes, but the full set is handled so any legal jdat string round-trips.
	function str() {
		i++; // opening quote
		let out = "";
		while (i < n) {
			const c = src[i++];
			if (c === '"') return out;
			if (c === "\\") {
				const e = src[i++];
				switch (e) {
					case '"':	out += '"';	break;
					case "\\":	out += "\\";	break;
					case "/":	out += "/";	break;
					case "b":	out += "\b";	break;
					case "f":	out += "\f";	break;
					case "n":	out += "\n";	break;
					case "r":	out += "\r";	break;
					case "t":	out += "\t";	break;
					case "u": {
						const hex = src.slice(i, i + 4);
						i += 4;
						out += String.fromCharCode(parseInt(hex, 16));
						break;
					}
					default:	out += e; // Tolerate an unknown escape by keeping the char.
				}
			} else {
				out += c;
			}
		}
		fail("unterminated string");
	}

	// A bare number: integer or float, with an optional sign and scientific exponent (jdat writes f32 as
	// e.g. "5.665e0"). parseFloat covers every form this subset emits.
	function number() {
		const start = i;
		while (i < n && "+-0123456789.eE".includes(src[i])) i++;
		if (i === start) fail("expected a value");
		const num = parseFloat(src.slice(start, i));
		if (Number.isNaN(num)) fail("not a number");
		return num;
	}

	const result = value();
	skipWs();
	// Trailing content after the top value means the file was not the single document map expected.
	if (i < n) fail("trailing content after document");
	return result;
}

// ---------------------------------------------------------------------------------------------------
// Serialising back to text jdat -- the inverse of the parser above, enough to re-emit an annotation and
// the list that holds it. The parser strips a scalar's type tag on the way in, so a re-serialisation of
// the *whole* document from the parsed model could not know a number's original tag (u32 vs i32 vs u8 vs
// f32), and the Rust decoder rejects the wrong one (`try_extract_dat!(_, U8)` and its kin). Annotations
// are the one section the authoring layer builds itself, with every atom's tag known, so this serialiser
// carries the tag explicitly through the small wrappers below and emits a section the Rust reader accepts.
// ---------------------------------------------------------------------------------------------------

// A scalar with its jdat tag preserved, so `encodeJdat` writes `(<tag>|<n>)` rather than a bare number.
function atom(tag, n) { return { __atom: tag, value: n }; }

// An ordered map with its tag, so `encodeJdat` writes `(omap|{ ... })` rather than a bare `{ ... }`.
function omap(obj) { return { __omap: obj }; }

// A jdat string with RFC 8259 escapes, matching the escapes the parser's `str()` accepts.
function encodeString(s) {
	let out = '"';
	for (const ch of s) {
		switch (ch) {
			case '"':	out += '\\"';	break;
			case "\\":	out += "\\\\";	break;
			case "\b":	out += "\\b";	break;
			case "\f":	out += "\\f";	break;
			case "\n":	out += "\\n";	break;
			case "\r":	out += "\\r";	break;
			case "\t":	out += "\\t";	break;
			default:
				const code = ch.codePointAt(0);
				if (code < 0x20) out += "\\u" + code.toString(16).padStart(4, "0");
				else out += ch;
		}
	}
	return out + '"';
}

// Emits a value as text jdat, reproducing the spacing the Rust encoder uses: a space after `{`, `[`, `:`
// and `,`, and none before the closing bracket. An `omap`/`atom` wrapper carries its tag; a plain string,
// array or number falls through to the bare forms.
function encodeJdat(v) {
	if (v && v.__atom !== undefined)	return `(${v.__atom}|${v.value})`;
	if (v && v.__omap !== undefined) {
		const obj = v.__omap;
		const keys = Object.keys(obj);
		if (keys.length === 0) return "(omap|{})";
		const body = keys.map(k => `${encodeString(k)}: ${encodeJdat(obj[k])}`).join(", ");
		return `(omap|{ ${body}})`;
	}
	if (Array.isArray(v)) {
		if (v.length === 0) return "[]";
		return `[ ${v.map(encodeJdat).join(", ")}]`;
	}
	if (typeof v === "string")	return encodeString(v);
	if (typeof v === "number")	return String(v);
	throw new Error("encodeJdat: cannot serialise " + typeof v);
}

window.Jdat = {
	parse:	parseJdat,
	encode:	encodeJdat,
	atom,
	omap,
	i32:	n => atom("i32", Math.round(n)),
	u32:	n => atom("u32", Math.round(n)),
	u8:		n => atom("u8", Math.round(n)),
	f32:	n => atom("f32", n),
};
