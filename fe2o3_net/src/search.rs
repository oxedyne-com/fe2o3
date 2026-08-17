//! A bring-your-own-key client for the web search APIs, in the half that is not a socket.
//!
//! # What it is, and is not
//!
//! One query, one list: a phrase goes up, titles and links come back. It is not an answer engine,
//! holds no session, and asks for no page contents -- a caller wanting the text of a result fetches
//! that result itself, and pays the tokens knowingly rather than by surprise.
//!
//! # Why it does not send anything
//!
//! [`Engine::request`] returns the *parts* of a call -- host, port, path, method, headers, body --
//! and stops there. The caller dials. That is the whole point rather than an oversight: a caller
//! that resolves a host, refuses a private address and repeats the refusal on every redirect hop
//! has built a gate, and a module that opens its own socket walks around it. There is deliberately
//! no convenience here that sends.
//!
//! # Four vendors, four dialects
//!
//! Unlike [`crate::llm`], where three providers speak one dialect and differ only in address, these
//! four agree on nothing: one is a `GET` with the query in the path, three are a `POST` with it in a
//! JSON body, and each names the key header differently. So the enum carries real per-arm code, and
//! [`SearchResult`] is the narrow common shape all four are flattened into -- title, url, snippet,
//! age, every field a string.
//!
//! # Forgiving by the row, strict by the document
//!
//! [`Engine::parse`] drops a result that has no title or no url and returns the rest, because one
//! malformed row out of twenty is not a reason to answer a user with nothing. It returns an error
//! only when the *document* is an error document, and then it names the engine and repeats what the
//! engine said, since that text is where a vendor explains a rejected key or an unknown parameter.
//!
//! `age` is passed through exactly as the engine wrote it and is never parsed into a timestamp. The
//! engines disagree about what it measures -- when a page was published, when it was last crawled,
//! how long ago either was -- and a confidently wrong date is worse than an honest blank.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	string::dec::DecoderConfig,
	usr::{
		UsrKind,
		UsrKindCode,
		UsrKindId,
	},
};

use std::collections::BTreeMap;

use crate::http::{
	header::HttpMethod,
	pct,
};


/// What a query is looking for, and therefore which of an engine's endpoints answers it.
///
/// Three kinds rather than a free string, because an engine either has a corner of its index for a
/// kind or it does not, and [`Engine::supports`] can only answer a closed question.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Kind {
	#[default]
	Web,		// the open web, and the default when a caller says nothing
	News,		// recent journalism, from whichever corner of the index holds it
	Academic,	// papers, preprints, journal articles; not every engine has any
}

impl Kind {

	/// The word a kind is named by on the wire, in a request and in a stored setting.
	pub fn id(&self) -> &'static str {
		match self {
			Self::Web	=> "web",
			Self::News	=> "news",
			Self::Academic	=> "academic",
		}
	}

	/// The kind a word names, or `None` if it names no kind.
	///
	/// `None` rather than a default, so a caller can tell a kind it was not given from a kind it was
	/// given wrongly, and answer the second with an error instead of silently searching the web.
	pub fn from_id(s: &str) -> Option<Self> {
		match s {
			"web"		=> Some(Self::Web),
			"news"		=> Some(Self::News),
			"academic"	=> Some(Self::Academic),
			_		=> None,
		}
	}
}

/// A search engine, which is to say a request shape and a reply shape that go together.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Engine {
	Brave,	// its own crawler, keyed by a subscription-token header
	Exa,	// neural retrieval over an embedding index
	Tavily,	// a retrieval API built for agents, bearer-authenticated
	Serper,	// another engine's results, resold as JSON
}

impl Engine {

	// In a fixed order, so a caller listing them need not keep its own copy in step.
	pub const ALL: [Self; 4] = [Self::Brave, Self::Exa, Self::Tavily, Self::Serper];

	/// The word an engine is named by: in a request, in a stored setting, and in a ledger line.
	///
	/// One string in every one of those places. They are not independent spellings that happen to
	/// agree, and a typo in any of them is a mismatch nothing reports.
	pub fn id(&self) -> &'static str {
		match self {
			Self::Brave	=> "brave",
			Self::Exa	=> "exa",
			Self::Tavily	=> "tavily",
			Self::Serper	=> "serper",
		}
	}

	/// The engine a word names, or `None` if it names no engine.
	pub fn from_id(s: &str) -> Option<Self> {
		match s {
			"brave"		=> Some(Self::Brave),
			"exa"		=> Some(Self::Exa),
			"tavily"	=> Some(Self::Tavily),
			"serper"	=> Some(Self::Serper),
			_		=> None,
		}
	}

	/// The host to dial and to validate the TLS certificate against.
	pub fn host(&self) -> &'static str {
		match self {
			Self::Brave	=> "api.search.brave.com",
			Self::Exa	=> "api.exa.ai",
			Self::Tavily	=> "api.tavily.com",
			Self::Serper	=> "google.serper.dev",
		}
	}

	/// Can this engine answer this kind at all?
	///
	/// Two of the four have no scholarly index and say so here, so a caller can offer a kind the
	/// configured engine can actually serve rather than discovering it in a rejected request.
	pub fn supports(&self, kind: Kind) -> bool {
		match (self, kind) {
			// No scholarly corner of the index, and no parameter that would ask for one.
			(Self::Brave, Kind::Academic)	=> false,
			(Self::Tavily, Kind::Academic)	=> false,
			_				=> true,
		}
	}

	/// Host, path, method, headers and body for one query.
	///
	/// **Not a request that is sent.** The caller owns the transport, the address check and the TLS;
	/// see the module documentation for why that division is deliberate.
	pub fn request(&self, key: &str, q: &SearchQuery) -> Outcome<SearchCall> {
		if q.query.trim().is_empty() {
			return Err(err!(
				"An empty query was passed to the {} engine.", self.id();
				Invalid, Input, Missing));
		}
		// A missing key is refused here rather than sent, because the vendor answers a keyless
		// request with a bare 401 and no caller can read the reason out of that.
		if key.is_empty() {
			return Err(err!(
				"No API key was given for the {} engine.", self.id();
				Invalid, Input, Missing));
		}
		if !self.supports(q.kind) {
			return Err(err!(
				"The {} engine cannot answer a '{}' search.", self.id(), q.kind.id();
				Invalid, Input, Unknown));
		}
		let n = self.clamped(q); // Results asked for, within what the engine allows.

		match self {
			Self::Brave => {
				// The query rides in the path, so it is escaped as a browser would escape it.
				let seg = match q.kind {
					Kind::News	=> "news",
					_		=> "web",
				};
				let path = fmt!("/res/v1/{}/search?q={}&count={}",
					seg, pct::encode_component(q.query), n);
				Ok(SearchCall {
					host:		self.host().to_string(),
					port:		443,
					path,
					method:		HttpMethod::GET,
					headers:	self.headers(key),
					body:		Vec::new(),
				})
			},
			Self::Exa => {
				let mut b = DaticleMap::new();
				b.insert(dat!("query"), dat!(q.query.to_string()));
				b.insert(dat!("numResults"), dat!(n as u64));
				// The category names the slice of the index; the web is the whole of it, so it
				// asks for no category at all.
				match q.kind {
					Kind::News	=> { b.insert(dat!("category"), dat!("news")); },
					// Scholarly work is filed under publications, which is what this vendor
					// calls papers, preprints and journal articles.
					Kind::Academic	=> { b.insert(dat!("category"), dat!("publication")); },
					Kind::Web	=> {},
				}
				self.post(key, "/search", res!(Dat::Map(b).json()))
			},
			Self::Tavily => {
				let mut b = DaticleMap::new();
				b.insert(dat!("query"), dat!(q.query.to_string()));
				b.insert(dat!("max_results"), dat!(n as u64));
				b.insert(dat!("topic"), match q.kind {
					Kind::News	=> dat!("news"),
					_		=> dat!("general"),
				});
				self.post(key, "/search", res!(Dat::Map(b).json()))
			},
			Self::Serper => {
				// Here the kind is the path rather than a parameter.
				let path = match q.kind {
					Kind::Web	=> "/search",
					Kind::News	=> "/news",
					Kind::Academic	=> "/scholar",
				};
				let mut b = DaticleMap::new();
				b.insert(dat!("q"), dat!(q.query.to_string()));
				b.insert(dat!("num"), dat!(n as u64));
				self.post(key, path, res!(Dat::Map(b).json()))
			},
		}
	}

	/// The engine's answer, whatever its shape, as the common list.
	///
	/// Forgiving by the row and strict by the document: see the module documentation.
	pub fn parse(&self, body: &[u8]) -> Outcome<Vec<SearchResult>> {
		let txt = match std::str::from_utf8(body) {
			Ok(t)	=> t,
			Err(e)	=> return Err(err!(e,
				"The {} engine answered with bytes that are not text.", self.id();
				Network, Data, Decode)),
		};
		let dat = res!(Dat::decode_string_with_config(txt.to_string(), &json_decoder()));
		let map = match &dat {
			Dat::Map(m)	=> m,
			other		=> return Err(err!(
				"The {} engine answered with a JSON {:?} rather than an object.",
				self.id(), other.kind();
				Network, Data, Mismatch)),
		};

		// An error document is surfaced with the engine's own words, since that is the only place a
		// rejected key or an unknown parameter is ever explained.
		if let Some(why) = error_message(map) {
			return Err(err!(
				"The {} engine returned an error: {}", self.id(), why;
				Network, Data));
		}

		let rows = match self.rows(map) {
			Some(l)	=> l,
			None	=> {
				// Some engines report a refusal as a bare message beside a status code, with no
				// wrapper this reads as an error document, so it is named here instead.
				let tail = match map.get(&dat!("message")) {
					Some(Dat::Str(s))	=> s.clone(),
					_			=> clip(txt),
				};
				return Err(err!(
					"The {} engine answered with no results: {}", self.id(), tail;
					Network, Data, Missing));
			},
		};

		let mut out = Vec::with_capacity(rows.len());
		for row in rows {
			let m = match row {
				Dat::Map(m)	=> m,
				_		=> continue, // Not an object, so not a result.
			};
			if let Some(r) = self.row(m) {
				out.push(r);
			}
		}
		Ok(out)
	}

	/// The greatest number of results this engine will return for this kind.
	///
	/// Each is the vendor's own published ceiling; asking for more is a rejected request rather than
	/// a longer list.
	fn max_results(&self, kind: Kind) -> usize {
		match (self, kind) {
			(Self::Brave, Kind::News)	=> 50,
			(Self::Brave, _)		=> 20,
			(Self::Exa, _)			=> 100,
			(Self::Tavily, _)		=> 20,
			(Self::Serper, _)		=> 100,
		}
	}

	/// The number of results to ask for: what the caller wanted, held between one and the ceiling.
	fn clamped(&self, q: &SearchQuery) -> usize {
		let max = self.max_results(q.kind);
		if q.limit == 0 { 1 } else if q.limit > max { max } else { q.limit }
	}

	/// The headers for one call, including the key under whichever name this vendor reads it.
	///
	/// `Accept-Encoding: identity` because the caller owns the socket and need not own a
	/// decompressor as well; a body it cannot inflate is a body it cannot parse.
	fn headers(&self, key: &str) -> Vec<(String, String)> {
		let mut h = vec![
			("Host".to_string(),			self.host().to_string()),
			("Accept".to_string(),			"application/json".to_string()),
			("Accept-Encoding".to_string(),		"identity".to_string()),
		];
		// Four vendors, four spellings. None of them takes the key as a query parameter, so it never
		// reaches a path, a log line or a referrer.
		let (name, value) = match self {
			Self::Brave	=> ("X-Subscription-Token", key.to_string()),
			Self::Exa	=> ("x-api-key", key.to_string()),
			Self::Tavily	=> ("Authorization", fmt!("Bearer {}", key)),
			Self::Serper	=> ("X-API-KEY", key.to_string()),
		};
		h.push((name.to_string(), value));
		h
	}

	/// A JSON `POST` to this engine, which is the shape of three of the four.
	fn post(&self, key: &str, path: &str, body: String) -> Outcome<SearchCall> {
		let mut headers = self.headers(key);
		headers.push(("Content-Type".to_string(), "application/json".to_string()));
		Ok(SearchCall {
			host:	self.host().to_string(),
			port:	443,
			path:	path.to_string(),
			method:	HttpMethod::POST,
			headers,
			body:	body.into_bytes(),
		})
	}

	/// The list of rows in this engine's answer, wherever it keeps them.
	///
	/// [`Engine::parse`] is not told the kind, and two engines file a news answer somewhere other
	/// than a web one, so each candidate place is tried in turn.
	fn rows<'a>(&self, map: &'a DaticleMap) -> Option<&'a Vec<Dat>> {
		match self {
			Self::Brave => {
				// A web answer nests its list under `web`; a news answer puts it at the top.
				if let Some(Dat::Map(w)) = map.get(&dat!("web")) {
					if let Some(Dat::List(l)) = w.get(&dat!("results")) {
						return Some(l);
					}
				}
				list(map, "results")
			},
			Self::Exa	=> list(map, "results"),
			Self::Tavily	=> list(map, "results"),
			// Web and scholarly answers are both `organic`; a news answer is `news`.
			Self::Serper	=> list(map, "organic").or_else(|| list(map, "news")),
		}
	}

	/// One row of this engine's answer as a common result, or `None` if it is not usable.
	///
	/// A row with no title or no url is dropped rather than passed on empty: a link with nothing to
	/// click and a heading with nothing under it are both worse than one fewer result.
	fn row(&self, m: &DaticleMap) -> Option<SearchResult> {
		let (title, url, snippet, age) = match self {
			Self::Brave => (
				text(m, "title"),
				text(m, "url"),
				text(m, "description"),
				// The relative phrase if the engine gave one, the crawl stamp otherwise.
				first_of(m, &["age", "page_age"]),
			),
			Self::Exa => (
				text(m, "title"),
				text(m, "url"),
				// Page contents are not asked for, so this is normally empty. The full text, if
				// an engine sends it unbidden, is deliberately not folded in: a snippet is a
				// line and a page is a token bill.
				match m.get(&dat!("highlights")) {
					Some(Dat::List(l)) => match l.first() {
						Some(Dat::Str(s))	=> s.clone(),
						_			=> text(m, "summary"),
					},
					_ => text(m, "summary"),
				},
				text(m, "publishedDate"),
			),
			Self::Tavily => (
				text(m, "title"),
				text(m, "url"),
				text(m, "content"),
				text(m, "published_date"),
			),
			Self::Serper => (
				text(m, "title"),
				text(m, "link"),
				// A scholarly row carries its authors and venue where a web row carries a
				// snippet, and that line is the useful one to show.
				first_of(m, &["snippet", "publicationInfo"]),
				// A scholarly row dates itself with a bare year, which is rendered rather than
				// interpreted.
				first_of(m, &["date", "year"]),
			),
		};
		if title.is_empty() || url.is_empty() {
			return None;
		}
		Some(SearchResult { title, url, snippet, age })
	}
}

/// One result, flattened out of whatever the engine called its fields.
///
/// Every field is a string, `snippet` and `age` may be empty, and `title` and `url` may not -- a row
/// missing either never becomes one of these.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchResult {
	pub title:	String,
	pub url:	String,
	pub snippet:	String,		// the engine's excerpt, which may be empty
	// Whatever freshness the engine reported, verbatim and unparsed. Never a timestamp this
	// module worked out.
	pub age:	String,
}

/// What is being asked for, before any engine has been chosen.
#[derive(Clone, Copy, Debug)]
pub struct SearchQuery<'a> {
	pub query:	&'a str,
	pub kind:	Kind,		// which corner of the index to search
	pub limit:	usize,		// clamped by the engine's own maximum; nought is read as one
}

/// The parts of one call, for a caller that will make it.
///
/// Everything needed to dial and to write the request, and nothing that dials. See the module
/// documentation for why this type exists instead of a function that sends.
#[derive(Clone, Debug)]
pub struct SearchCall {
	pub host:	String,			// dialled, and the name the certificate is validated against
	pub port:	u16,			// always 443 for these vendors, carried rather than assumed
	pub path:	String,			// with any query string already escaped into it
	pub method:	HttpMethod,
	pub headers:	Vec<(String, String)>,	// the key among them, named as this vendor names it
	pub body:	Vec<u8>,		// empty for a `GET`
}

fn json_decoder() -> DecoderConfig<
	BTreeMap<UsrKindCode, UsrKind>,
	BTreeMap<String, UsrKindId>,
>
{
	DecoderConfig::json(None)
}

/// A string field, or the empty string if it is absent or is neither a string nor a whole number.
fn text(m: &DaticleMap, key: &str) -> String {
	match m.get(&dat!(key)) {
		Some(Dat::Str(s))	=> s.clone(),
		Some(other)		=> whole(other).unwrap_or_default(),
		None			=> String::new(),
	}
}

/// A whole number written out, or `None` if the value is not one.
///
/// One engine dates a paper with a bare integer, and the decoder narrows an unannotated number to
/// the smallest kind that holds it -- so every width has to be answered here, not just the widest.
/// Rendering is not parsing: the digits go through as they arrived.
fn whole(d: &Dat) -> Option<String> {
	match d {
		Dat::U8(n)	=> Some(fmt!("{}", n)),
		Dat::U16(n)	=> Some(fmt!("{}", n)),
		Dat::U32(n)	=> Some(fmt!("{}", n)),
		Dat::U64(n)	=> Some(fmt!("{}", n)),
		Dat::U128(n)	=> Some(fmt!("{}", n)),
		Dat::C64(n)	=> Some(fmt!("{}", n)),
		Dat::I8(n)	=> Some(fmt!("{}", n)),
		Dat::I16(n)	=> Some(fmt!("{}", n)),
		Dat::I32(n)	=> Some(fmt!("{}", n)),
		Dat::I64(n)	=> Some(fmt!("{}", n)),
		Dat::I128(n)	=> Some(fmt!("{}", n)),
		Dat::Aint(n)	=> Some(fmt!("{}", n)),
		_		=> None,
	}
}

/// The first of several fields that is present and not empty.
fn first_of(m: &DaticleMap, keys: &[&str]) -> String {
	for k in keys {
		let v = text(m, k);
		if !v.is_empty() {
			return v;
		}
	}
	String::new()
}

/// A list field, or `None` if it is absent or is not a list.
fn list<'a>(m: &'a DaticleMap, key: &str) -> Option<&'a Vec<Dat>> {
	match m.get(&dat!(key)) {
		Some(Dat::List(l))	=> Some(l),
		_			=> None,
	}
}

/// The message out of an error document, or `None` if the document is not one.
///
/// The four vendors wrap it four ways -- a bare string, an object with a `detail`, an object with a
/// `message`, an object holding an object -- so each shape is unwrapped rather than any one being
/// assumed.
fn error_message(map: &DaticleMap) -> Option<String> {
	for outer in ["error", "detail"] {
		match map.get(&dat!(outer)) {
			Some(Dat::Str(s)) => {
				// A short word beside a longer explanation: report both, in that order.
				let extra = text(map, "message");
				return Some(if extra.is_empty() || &extra == s {
					s.clone()
				} else {
					fmt!("{}: {}", s, extra)
				});
			},
			Some(Dat::Map(inner)) => {
				let v = first_of(inner, &["detail", "message", "error"]);
				return Some(if v.is_empty() { fmt!("{:?}", inner) } else { v });
			},
			_ => {},
		}
	}
	None
}

/// A body cut short enough to put in an error message, on a character boundary.
fn clip(s: &str) -> String {
	const MAX: usize = 200; // Enough to recognise the document, short enough to read.
	if s.len() <= MAX {
		return s.to_string();
	}
	let mut end = MAX;
	while end > 0 && !s.is_char_boundary(end) {
		end -= 1;
	}
	fmt!("{}...", &s[..end])
}


#[cfg(test)]
mod tests {
	use super::*;

	// Sample bodies, written to the shapes the four vendors publish. Values are invented; the field
	// names, their nesting and their spelling are the point.

	// A web answer, nesting its list under `web`, with one row dated relatively, one only by its
	// crawl stamp, and one not at all.
	const BRAVE_WEB: &str = r#"{
		"query": {"original": "iron oxide", "more_results_available": true},
		"web": {"results": [
			{"title": "Iron oxide", "url": "https://enc.example.org/iron-oxide",
			 "description": "Iron oxides are chemical compounds.",
			 "age": "3 days ago", "page_age": "2026-08-07T11:00:00"},
			{"title": "How rust forms", "url": "https://chem.example.com/rust",
			 "description": "Corrosion in the presence of water.",
			 "page_age": "2026-07-01T09:30:00"},
			{"title": "Red pigments", "url": "https://pigment.example.net/red",
			 "description": ""}
		]}
	}"#;

	// A news answer, which puts its list at the top level instead.
	const BRAVE_NEWS: &str = r#"{
		"type": "news",
		"results": [
			{"title": "Foundry reopens", "url": "https://news.example.org/foundry",
			 "description": "The plant restarts.", "age": "5 hours ago"},
			{"title": "Ore prices ease", "url": "https://news.example.com/ore",
			 "description": "Down on the week.", "age": "1 day ago"}
		]
	}"#;

	// Two results, one carrying a highlight and a null author, one carrying neither.
	const EXA: &str = r#"{
		"requestId": "req-1",
		"results": [
			{"title": "A study of iron oxide", "url": "https://arxiv.example.org/abs/2401.00001",
			 "id": "https://arxiv.example.org/abs/2401.00001",
			 "publishedDate": "2024-01-02T00:00:00.000Z", "author": null},
			{"title": "Corrosion review", "url": "https://journal.example.com/c/12",
			 "id": "c12", "publishedDate": "2023-05-05T00:00:00.000Z",
			 "author": "A. Smith",
			 "highlights": ["Corrosion of steel in seawater."]}
		],
		"costDollars": {"total": 0.005}
	}"#;

	// Two results, the second dated in the way a news answer dates them.
	const TAVILY: &str = r#"{
		"query": "iron oxide",
		"results": [
			{"title": "Iron oxide", "url": "https://example.org/a",
			 "content": "Iron oxides are compounds.", "score": 0.98},
			{"title": "Ochre", "url": "https://example.org/b",
			 "content": "A natural pigment.", "score": 0.81,
			 "published_date": "Mon, 04 Aug 2026 09:00:00 GMT"}
		],
		"response_time": 1.2
	}"#;

	// A web answer, whose list is `organic` and whose address field is `link`.
	const SERPER_WEB: &str = r#"{
		"searchParameters": {"q": "iron oxide", "type": "search"},
		"organic": [
			{"title": "Iron oxide - Encyclopaedia", "link": "https://enc.example.org/iron-oxide",
			 "snippet": "Iron oxides are chemical compounds.", "position": 1},
			{"title": "Rust never sleeps", "link": "https://blog.example.com/rust",
			 "snippet": "On corrosion.", "date": "12 Jul 2026", "position": 2}
		],
		"credits": 1
	}"#;

	// A news answer, whose list is `news` instead.
	const SERPER_NEWS: &str = r#"{
		"news": [
			{"title": "Foundry reopens", "link": "https://news.example.org/foundry",
			 "snippet": "The plant restarts.", "date": "2 hours ago", "source": "Example Times"}
		]
	}"#;

	// A scholarly answer: no snippet, a venue line instead, and a bare year for a date.
	const SERPER_SCHOLAR: &str = r#"{
		"organic": [
			{"title": "Attention is all you need", "link": "https://p.example.org/7181",
			 "publicationInfo": "A Vaswani, N Shazeer - Advances in neural information processing",
			 "year": 2017, "citedBy": 119097}
		]
	}"#;

	// Error documents, one per vendor, each wrapping its message differently.

	const BRAVE_ERR: &str = r#"{"type":"ErrorResponse","error":{"id":"e1","status":422,
		"code":"VALIDATION","detail":"Unable to validate request parameter(s)","meta":{}},
		"time":1754800000}"#;
	const EXA_ERR: &str = r#"{"error":"Unauthorized","message":"Invalid API key","statusCode":401}"#;
	const TAVILY_ERR: &str = r#"{"detail":{"error":"Unauthorized: missing or invalid API key."}}"#;
	const SERPER_ERR: &str = r#"{"message":"Unauthorized.","statusCode":403}"#;

	/// The urls of a parsed body, which is what a test compares: the whole list, in order, rather
	/// than a count that would still pass with the wrong rows in it.
	fn urls(rs: &[SearchResult]) -> Vec<&str> {
		rs.iter().map(|r| r.url.as_str()).collect()
	}

	/// A query of a given kind, so the tests below read as what they are varying.
	fn q<'a>(query: &'a str, kind: Kind, limit: usize) -> SearchQuery<'a> {
		SearchQuery { query, kind, limit }
	}

	/// The header of a call, by name, case-insensitively as HTTP reads them.
	fn hdr(c: &SearchCall, name: &str) -> Option<String> {
		c.headers.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case(name))
			.map(|(_, v)| v.clone())
	}

	/// Every engine's word round-trips, and a word that names no engine is refused rather than
	/// guessed at. The same string is the wire format in four places, so one typo is a silent
	/// mismatch nothing else would catch.
	#[test]
	fn test_an_engine_names_itself_00() -> Outcome<()> {
		for e in Engine::ALL {
			let id = e.id();
			assert!(!id.is_empty(), "{:?} has no id", e);
			assert_eq!(Engine::from_id(id), Some(e), "'{}' did not round-trip", id);
			assert!(e.host().contains('.'), "'{}' has no host", id);
		}
		// Every id is distinct, which a match arm copied and half-edited would break.
		for a in Engine::ALL {
			for b in Engine::ALL {
				if a != b {
					assert_ne!(a.id(), b.id(), "{:?} and {:?} share an id", a, b);
				}
			}
		}
		assert_eq!(Engine::from_id("bing"), None, "an unknown engine was accepted");
		assert_eq!(Engine::from_id("Brave"), None, "the id is not case-insensitive");
		assert_eq!(Engine::from_id(""), None, "an empty id was accepted");
		Ok(())
	}

	/// A kind's word round-trips too, since it crosses the same wire.
	#[test]
	fn test_a_kind_names_itself_01() -> Outcome<()> {
		for k in [Kind::Web, Kind::News, Kind::Academic] {
			assert_eq!(Kind::from_id(k.id()), Some(k), "'{}' did not round-trip", k.id());
		}
		assert_eq!(Kind::from_id("scholar"), None, "an unknown kind was accepted");
		assert_eq!(Kind::default(), Kind::Web, "the default kind is the web");
		Ok(())
	}

	/// Each vendor reads the key from its own header, and none of them reads it from the path -- so
	/// a key never reaches a request line, and a header named wrongly is a 401 with no explanation.
	#[test]
	fn test_the_key_goes_where_the_vendor_wants_it_02() -> Outcome<()> {
		let key = "test-key-value";
		let expect: &[(Engine, &str, &str)] = &[
			(Engine::Brave,		"X-Subscription-Token",	"test-key-value"),
			(Engine::Exa,		"x-api-key",		"test-key-value"),
			(Engine::Tavily,	"Authorization",	"Bearer test-key-value"),
			(Engine::Serper,	"X-API-KEY",		"test-key-value"),
		];
		for (e, name, value) in expect {
			let c = res!(e.request(key, &q("iron oxide", Kind::Web, 5)));
			assert_eq!(hdr(&c, name).as_deref(), Some(*value),
				"{} did not carry its key in {}", e.id(), name);
			assert_eq!(hdr(&c, "Host").as_deref(), Some(e.host()),
				"{} did not name its host", e.id());
			assert!(!c.path.contains(key),
				"{} put the key in the path: {}", e.id(), c.path);
			let body = String::from_utf8_lossy(&c.body).to_string();
			assert!(!body.contains(key),
				"{} put the key in the body: {}", e.id(), body);
			// No other vendor's header name is present, which a copied arm would leave behind.
			// Compared as HTTP compares them, since two of these vendors ask for the same name
			// in different case and that is one header, not two.
			for (_, other, _) in expect {
				if !other.eq_ignore_ascii_case(name) {
					assert!(hdr(&c, other).is_none(),
						"{} also carried {}", e.id(), other);
				}
			}
		}
		// A call with no key at all is refused here rather than sent to be refused there.
		for e in Engine::ALL {
			assert!(e.request("", &q("iron oxide", Kind::Web, 5)).is_err(),
				"{} accepted an empty key", e.id());
			assert!(e.request("k", &q("   ", Kind::Web, 5)).is_err(),
				"{} accepted an empty query", e.id());
		}
		Ok(())
	}

	/// The query reaches the wire intact: escaped into the path where it rides in the path, and
	/// quoted into JSON where it rides in a body.
	#[test]
	fn test_a_query_reaches_the_wire_intact_03() -> Outcome<()> {
		// Every character a naive concatenation would break out of, in both directions.
		let awkward = "rust & \"iron\" +oxide/water";

		let c = res!(Engine::Brave.request("k", &q(awkward, Kind::Web, 5)));
		assert_eq!(c.method, HttpMethod::GET, "brave is a GET");
		assert!(c.body.is_empty(), "a GET carries no body");
		assert!(c.path.starts_with("/res/v1/web/search?q="), "wrong path: {}", c.path);
		// The ampersand and the quote are escaped, so neither starts a parameter nor ends a word.
		assert!(!c.path.contains(" ") && !c.path.contains("\""),
			"the query was not escaped: {}", c.path);
		assert!(c.path.contains("%26"), "the ampersand was not escaped: {}", c.path);
		let raw = match c.path.split("q=").nth(1) {
			Some(t)	=> match t.split('&').next() {
				Some(v)	=> v,
				None	=> return Err(err!("no q value: {}", c.path; Test)),
			},
			None	=> return Err(err!("no q parameter: {}", c.path; Test)),
		};
		assert_eq!(res!(pct::decode_str(raw)), awkward, "the query did not survive escaping");

		for e in [Engine::Exa, Engine::Tavily, Engine::Serper] {
			let c = res!(e.request("k", &q(awkward, Kind::Web, 5)));
			assert_eq!(c.method, HttpMethod::POST, "{} is a POST", e.id());
			assert_eq!(hdr(&c, "Content-Type").as_deref(), Some("application/json"),
				"{} did not declare JSON", e.id());
			let txt = String::from_utf8_lossy(&c.body).to_string();
			// It parses back as JSON, which a hand-built body with a bare quote would not.
			let dat = res!(Dat::decode_string_with_config(txt.clone(), &json_decoder()));
			let m = match dat {
				Dat::Map(m)	=> m,
				_		=> return Err(err!("{} sent no object", e.id(); Test)),
			};
			let field = match e {
				Engine::Serper	=> "q",
				_		=> "query",
			};
			assert_eq!(text(&m, field), awkward,
				"{} did not carry the query intact: {}", e.id(), txt);
		}
		Ok(())
	}

	/// A caller asking for more than the vendor allows is held at the ceiling, and one asking for
	/// nothing still asks for something.
	#[test]
	fn test_the_limit_is_held_within_the_ceiling_04() -> Outcome<()> {
		// The number of results a call asks for, wherever that engine writes it.
		fn asked(e: Engine, c: &SearchCall) -> Outcome<usize> {
			match e {
				Engine::Brave => {
					let s = match c.path.split("count=").nth(1) {
						Some(s)	=> s,
						None	=> return Err(err!(
							"no count parameter: {}", c.path; Test)),
					};
					Ok(res!(s.parse::<usize>()))
				},
				_ => {
					let txt = String::from_utf8_lossy(&c.body).to_string();
					let dat = res!(Dat::decode_string_with_config(txt, &json_decoder()));
					let m = match dat {
						Dat::Map(m)	=> m,
						_		=> return Err(err!("no object"; Test)),
					};
					let field = match e {
						Engine::Exa	=> "numResults",
						Engine::Tavily	=> "max_results",
						_		=> "num",
					};
					Ok(res!(text(&m, field).parse::<usize>()))
				},
			}
		}
		for e in Engine::ALL {
			for kind in [Kind::Web, Kind::News, Kind::Academic] {
				if !e.supports(kind) {
					continue;
				}
				let ceiling = e.max_results(kind);
				let big = res!(e.request("k", &q("iron", kind, 5_000)));
				let n = res!(asked(e, &big));
				assert_eq!(n, ceiling,
					"{} '{}' asked for {} against a ceiling of {}",
					e.id(), kind.id(), n, ceiling);

				let none = res!(e.request("k", &q("iron", kind, 0)));
				assert!(res!(asked(e, &none)) >= 1,
					"{} '{}' asked for no results at all", e.id(), kind.id());

				let some = res!(e.request("k", &q("iron", kind, 3)));
				assert_eq!(res!(asked(e, &some)), 3,
					"{} '{}' did not pass a modest limit through", e.id(), kind.id());
			}
		}
		Ok(())
	}

	/// An engine with no scholarly index says so, and refuses the request rather than sending one it
	/// knows will come back empty or rejected.
	#[test]
	fn test_an_engine_is_honest_about_the_kinds_it_answers_05() -> Outcome<()> {
		for e in Engine::ALL {
			assert!(e.supports(Kind::Web), "{} cannot search the web", e.id());
			assert!(e.supports(Kind::News), "{} cannot search news", e.id());
		}
		assert!(!Engine::Brave.supports(Kind::Academic), "brave claimed a scholarly index");
		assert!(!Engine::Tavily.supports(Kind::Academic), "tavily claimed a scholarly index");
		assert!(Engine::Exa.supports(Kind::Academic), "exa has a publications category");
		assert!(Engine::Serper.supports(Kind::Academic), "serper has a scholar endpoint");

		// What is not supported is refused, and the refusal names both the engine and the kind.
		for e in Engine::ALL {
			let r = e.request("k", &q("iron oxide", Kind::Academic, 5));
			assert_eq!(r.is_err(), !e.supports(Kind::Academic),
				"{} disagreed with its own supports()", e.id());
			if let Err(err) = r {
				let msg = fmt!("{}", err);
				assert!(msg.contains(e.id()), "the refusal did not name {}: {}", e.id(), msg);
				assert!(msg.contains("academic"), "the refusal did not name the kind: {}", msg);
			}
		}
		// The kind that is supported reaches the wire as that vendor spells it.
		let exa = res!(Engine::Exa.request("k", &q("iron", Kind::Academic, 5)));
		let txt = String::from_utf8_lossy(&exa.body).to_string();
		assert!(txt.contains("publication"), "exa did not ask for publications: {}", txt);
		let ser = res!(Engine::Serper.request("k", &q("iron", Kind::Academic, 5)));
		assert_eq!(ser.path, "/scholar", "serper did not use its scholar endpoint");
		let ser_news = res!(Engine::Serper.request("k", &q("iron", Kind::News, 5)));
		assert_eq!(ser_news.path, "/news", "serper did not use its news endpoint");
		Ok(())
	}

	/// Each engine's answer flattens to the same list, in the engine's own order.
	#[test]
	fn test_a_sample_answer_becomes_the_common_list_06() -> Outcome<()> {
		let cases: &[(Engine, &str, &[&str])] = &[
			(Engine::Brave, BRAVE_WEB, &[
				"https://enc.example.org/iron-oxide",
				"https://chem.example.com/rust",
				"https://pigment.example.net/red",
			]),
			(Engine::Brave, BRAVE_NEWS, &[
				"https://news.example.org/foundry",
				"https://news.example.com/ore",
			]),
			(Engine::Exa, EXA, &[
				"https://arxiv.example.org/abs/2401.00001",
				"https://journal.example.com/c/12",
			]),
			(Engine::Tavily, TAVILY, &[
				"https://example.org/a",
				"https://example.org/b",
			]),
			(Engine::Serper, SERPER_WEB, &[
				"https://enc.example.org/iron-oxide",
				"https://blog.example.com/rust",
			]),
			(Engine::Serper, SERPER_NEWS, &["https://news.example.org/foundry"]),
			(Engine::Serper, SERPER_SCHOLAR, &["https://p.example.org/7181"]),
		];
		for (e, body, want) in cases {
			let got = res!(e.parse(body.as_bytes()));
			assert_eq!(urls(&got), want.to_vec(), "{} parsed the wrong rows", e.id());
			for r in &got {
				assert!(!r.title.is_empty(), "{} emitted a titleless row", e.id());
			}
		}

		// The fields each vendor calls something else arrive under the common names.
		let brave = res!(Engine::Brave.parse(BRAVE_WEB.as_bytes()));
		assert_eq!(brave[0].snippet, "Iron oxides are chemical compounds.");
		let serper = res!(Engine::Serper.parse(SERPER_WEB.as_bytes()));
		assert_eq!(serper[1].title, "Rust never sleeps");
		assert_eq!(serper[1].snippet, "On corrosion.");
		let tavily = res!(Engine::Tavily.parse(TAVILY.as_bytes()));
		assert_eq!(tavily[0].snippet, "Iron oxides are compounds.");
		let exa = res!(Engine::Exa.parse(EXA.as_bytes()));
		assert_eq!(exa[1].snippet, "Corrosion of steel in seawater.");
		assert_eq!(exa[0].snippet, "", "contents were not asked for, so there is no snippet");
		let scholar = res!(Engine::Serper.parse(SERPER_SCHOLAR.as_bytes()));
		assert!(scholar[0].snippet.starts_with("A Vaswani"),
			"a scholarly row lost its venue line: {}", scholar[0].snippet);
		Ok(())
	}

	/// Whatever the engine said about freshness is what comes out, character for character.
	#[test]
	fn test_age_is_passed_through_unparsed_07() -> Outcome<()> {
		let brave = res!(Engine::Brave.parse(BRAVE_WEB.as_bytes()));
		assert_eq!(brave[0].age, "3 days ago", "a relative phrase was not left alone");
		assert_eq!(brave[1].age, "2026-07-01T09:30:00", "the crawl stamp was not used as a fallback");
		assert_eq!(brave[2].age, "", "an undated row was given a date");

		let exa = res!(Engine::Exa.parse(EXA.as_bytes()));
		assert_eq!(exa[0].age, "2024-01-02T00:00:00.000Z", "an ISO stamp was reshaped");

		let tavily = res!(Engine::Tavily.parse(TAVILY.as_bytes()));
		assert_eq!(tavily[0].age, "", "an undated row was given a date");
		assert_eq!(tavily[1].age, "Mon, 04 Aug 2026 09:00:00 GMT", "an RFC date was reshaped");

		let serper = res!(Engine::Serper.parse(SERPER_WEB.as_bytes()));
		assert_eq!(serper[1].age, "12 Jul 2026", "a written date was reshaped");
		let scholar = res!(Engine::Serper.parse(SERPER_SCHOLAR.as_bytes()));
		assert_eq!(scholar[0].age, "2017", "a bare year was not rendered as it stands");
		Ok(())
	}

	/// A row with nothing to click, or nothing to read, is dropped and the rest are kept: one bad
	/// row out of a page is not a reason to answer with nothing.
	#[test]
	fn test_a_row_with_no_title_or_no_url_is_dropped_08() -> Outcome<()> {
		// Per engine: a good row, then one missing a title, one missing a url, one with an empty
		// url, one with an empty title, and one that is not an object at all.
		let brave = r#"{"web":{"results":[
			{"title":"Kept one","url":"https://example.org/1","description":"d"},
			{"url":"https://example.org/no-title","description":"d"},
			{"title":"No url","description":"d"},
			{"title":"Empty url","url":"","description":"d"},
			{"title":"","url":"https://example.org/empty-title","description":"d"},
			"not an object",
			{"title":"Kept two","url":"https://example.org/2","description":"d"}
		]}}"#;
		let exa = r#"{"results":[
			{"title":"Kept one","url":"https://example.org/1"},
			{"url":"https://example.org/no-title"},
			{"title":"No url"},
			{"title":"Empty url","url":""},
			{"title":"","url":"https://example.org/empty-title"},
			42,
			{"title":"Kept two","url":"https://example.org/2"}
		]}"#;
		let tavily = r#"{"results":[
			{"title":"Kept one","url":"https://example.org/1","content":"c"},
			{"url":"https://example.org/no-title","content":"c"},
			{"title":"No url","content":"c"},
			{"title":"Empty url","url":"","content":"c"},
			{"title":"","url":"https://example.org/empty-title","content":"c"},
			{"title":"Kept two","url":"https://example.org/2","content":"c"}
		]}"#;
		let serper = r#"{"organic":[
			{"title":"Kept one","link":"https://example.org/1","snippet":"s"},
			{"link":"https://example.org/no-title","snippet":"s"},
			{"title":"No url","snippet":"s"},
			{"title":"Empty url","link":"","snippet":"s"},
			{"title":"","link":"https://example.org/empty-title","snippet":"s"},
			{"title":"Kept two","link":"https://example.org/2","snippet":"s"}
		]}"#;
		let cases: &[(Engine, &str)] = &[
			(Engine::Brave, brave),
			(Engine::Exa, exa),
			(Engine::Tavily, tavily),
			(Engine::Serper, serper),
		];
		let want = vec!["https://example.org/1", "https://example.org/2"];
		for (e, body) in cases {
			let got = res!(e.parse(body.as_bytes()));
			assert_eq!(urls(&got), want, "{} kept or dropped the wrong rows", e.id());
			for r in &got {
				assert!(!r.title.is_empty() && !r.url.is_empty(),
					"{} emitted an empty row", e.id());
			}
		}
		// A list with nothing usable in it is an empty answer, not an error: the engine replied.
		let none = res!(Engine::Tavily.parse(br#"{"results":[]}"#));
		assert!(none.is_empty(), "an empty list should parse to an empty list");
		Ok(())
	}

	/// An error document is an error, and the error says which engine refused and what it said.
	///
	/// Each case also names something that appears only in the raw document -- a wrapper key, a
	/// status code -- and insists it is absent. Without that, a parser that recognised no error at
	/// all would still pass by dumping the whole body, since the body contains the message.
	#[test]
	fn test_an_error_document_names_the_engine_and_its_message_09() -> Outcome<()> {
		let cases: &[(Engine, &str, &[&str], &[&str])] = &[
			(Engine::Brave,	 BRAVE_ERR,
				&["Unable to validate request parameter(s)"],
				&["VALIDATION", "ErrorResponse"]),
			(Engine::Exa,	 EXA_ERR,
				&["Unauthorized", "Invalid API key"],
				&["statusCode"]),
			(Engine::Tavily, TAVILY_ERR,
				&["Unauthorized: missing or invalid API key."],
				&["detail"]),
			(Engine::Serper, SERPER_ERR,
				&["Unauthorized."],
				&["statusCode", "403"]),
		];
		for (e, body, said, unsaid) in cases {
			let r = e.parse(body.as_bytes());
			assert!(r.is_err(), "{} read an error document as results", e.id());
			if let Err(err) = r {
				let msg = fmt!("{}", err);
				assert!(msg.contains(e.id()), "the error did not name {}: {}", e.id(), msg);
				for want in *said {
					assert!(msg.contains(want),
						"the error did not repeat what {} said ('{}'): {}",
						e.id(), want, msg);
				}
				for raw in *unsaid {
					assert!(!msg.contains(raw),
						"the error dumped the document rather than reading it ('{}'): {}",
						raw, msg);
				}
			}
		}
		// A body that is not an object, and one that is not text, are errors that name the engine.
		for bad in [&b"[]"[..], &b"not json at all"[..], &[0xffu8, 0xfe][..]] {
			let r = Engine::Brave.parse(bad);
			assert!(r.is_err(), "a body of {:?} was read as results", bad);
		}
		Ok(())
	}

	/// A call carries everything the caller needs to dial, and nothing that dials.
	#[test]
	fn test_a_call_is_only_its_parts_10() -> Outcome<()> {
		for e in Engine::ALL {
			let c = res!(e.request("k", &q("iron oxide", Kind::Web, 5)));
			assert_eq!(c.host, e.host(), "{} named the wrong host", e.id());
			assert_eq!(c.port, 443, "{} is not on 443", e.id());
			assert!(c.path.starts_with('/'), "{} has no leading slash: {}", e.id(), c.path);
			assert_eq!(hdr(&c, "Accept").as_deref(), Some("application/json"),
				"{} did not ask for JSON", e.id());
			// The caller owns the socket and not a decompressor, so nothing arrives compressed.
			assert_eq!(hdr(&c, "Accept-Encoding").as_deref(), Some("identity"),
				"{} may be answered with a body the caller cannot inflate", e.id());
			assert_eq!(c.method.body_required(), !c.body.is_empty(),
				"{} disagrees with its own method about a body", e.id());
		}
		Ok(())
	}
}
