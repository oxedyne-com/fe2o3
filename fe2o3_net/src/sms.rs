//! A bring-your-own-key client for the SMS gateways, in the half that is not a socket.
//!
//! # What it is for
//!
//! One message, one receipt. It exists because a text message is the only alert channel that
//! reaches a person with no data connection: push needs the internet, mail needs the internet,
//! and a host that has just gone dark is exactly when neither may be to hand. So this is the
//! last leg of an alerting path rather than a messaging feature, and it is deliberately small.
//!
//! # Why it does not send anything
//!
//! [`Provider::request`] returns the *parts* of a call -- host, port, path, method, headers,
//! body -- and stops. The caller dials. That is the same division [`crate::search`] draws and
//! for the same reason: the caller resolves the host, refuses a private address and repeats the
//! refusal on every redirect hop, and a module that opened its own socket would walk around all
//! of it. There is deliberately no convenience here that sends.
//!
//! # Three vendors, one authentication scheme
//!
//! Every provider here reads its credential from an HTTP `Authorization: Basic` header. That is
//! not a coincidence, it is the entry requirement: a vendor whose scheme puts the secret in the
//! request *body* is excluded, because bodies are logged, echoed in error messages and captured
//! by proxies in ways headers are not. Vonage is the notable absence on exactly that ground --
//! it takes `api_key` and `api_secret` as body parameters. Adding it would mean the module could
//! no longer promise what [`Provider::request`]'s tests assert, which is that **the secret
//! appears in the headers and nowhere else**.
//!
//! The three differ in everything else: two take JSON and one takes a form body, two put the
//! account identifier in the path and one does not, and each names its fields differently. So
//! the enum carries real per-arm code, and [`Receipt`] is the narrow common shape they are
//! flattened into.
//!
//! # A credential is a pair
//!
//! All three authenticate as a user and a secret, though each calls the pair something else: a
//! username and an API key, an account identifier and an auth token. [`Credential`] carries the
//! two without adopting any one vendor's names for them.
//!
//! # What is not here
//!
//! No delivery receipts, no inbound messages, no scheduling, no templates, no contact lists. An
//! alert is sent and forgotten; whether it arrived is answered by the person's phone buzzing,
//! and a delivery-receipt webhook is a second service to run on the host that may be the one in
//! trouble.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
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
use oxedyne_fe2o3_text::base64;

use std::collections::BTreeMap;

use crate::http::{
	header::HttpMethod,
	pct,
};


// The longest body a single call will carry. Not a protocol limit -- a gateway will happily
// accept more and bill it as many segments -- but an alert that runs past this has stopped
// being an alert. Ten segments of plain GSM text is far more than a sentence naming a host and
// a fault, and the refusal is louder than a silent truncation would be.
pub const MAX_BODY_LEN: usize = 1530;

/// The account and secret a gateway authenticates with.
///
/// Two fields, because all three vendors here want a pair and each calls it something
/// different: a username and an API key, an account identifier and an auth token. Naming them
/// after any one vendor would make the other two read as exceptions.
///
/// **Neither field is ever written to a log by this module**, and [`Provider::request`] puts
/// both only in the `Authorization` header. See the module documentation for why that is a
/// requirement rather than a habit.
pub struct Credential<'a> {
	pub user:	&'a str,	// a username, an account identifier, an authentication identifier
	pub secret:	&'a str,	// an API key, an auth token
}

/// One message to one number.
pub struct Message<'a> {
	pub to:		&'a str,	// E.164, with the leading `+`
	// As the vendor wants it: a number the account owns, or an alphanumeric identifier where
	// the destination permits one. Empty asks the vendor for its default, which is what an
	// account with a single number should do rather than repeat itself.
	pub from:	&'a str,
	pub body:	&'a str,
}

/// Everything needed to place one call, and nothing else.
///
/// The caller owns the socket. See the module documentation.
pub struct SmsCall {
	pub host:	String,			// dialled, and the name the certificate is validated against
	pub port:	u16,			// always 443 for these vendors, carried rather than assumed
	pub path:	String,			// with any account identifier already escaped into it
	pub method:	HttpMethod,
	pub headers:	Vec<(String, String)>,	// including the credential under `Authorization`
	pub body:	Vec<u8>,
}

/// What a gateway said when it took the message.
///
/// Every field is a string except the count, because the three vendors disagree about the type
/// of every one of them -- a price arrives as a number from one and as a quoted decimal from
/// another -- and a receipt that reformatted them would be asserting a precision none of them
/// promises. The price in particular is passed through exactly as written, in whatever currency
/// the account is billed in, and is never parsed into a figure this module would then be
/// claiming to understand.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Receipt {
	pub id:		String,		// the vendor's own, for a support ticket
	// What the vendor called the outcome, not normalised: `SUCCESS`, `queued` and
	// `message(s) queued` all mean accepted, and flattening them into one word would throw away
	// the only text a person can quote back to the vendor.
	pub status:	String,
	pub parts:	u32,		// segments billed, or zero where the vendor did not say
	pub price:	String,		// verbatim and unparsed, or empty where the vendor did not say
}

/// An SMS gateway.
///
/// Three, all authenticating by `Authorization: Basic`. See the module documentation for why
/// that is the entry requirement and which vendor it excludes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provider {
	ClickSend,	// Australian, billing in Australian dollars, JSON body, no account in the path
	Twilio,		// form-encoded body, account identifier in the path
	Plivo,		// JSON body, account identifier in the path
}

impl Provider {
	// A list rather than a `match`, so a variant added and not listed here is unreachable
	// through `Self::from_id` -- the safe direction for a set whose members each carry a
	// credential.
	pub const ALL: [Self; 3] = [Self::ClickSend, Self::Twilio, Self::Plivo];

	/// The id, one spelling everywhere: this enum, a configuration value, a log line.
	pub fn id(&self) -> &'static str {
		match self {
			Self::ClickSend	=> "clicksend",
			Self::Twilio	=> "twilio",
			Self::Plivo	=> "plivo",
		}
	}

	pub fn from_id(s: &str) -> Option<Self> {
		let s = s.trim().to_lowercase();
		Self::ALL.into_iter().find(|p| p.id() == s)
	}

	/// Dialled, and the name the certificate is validated against.
	pub fn host(&self) -> &'static str {
		match self {
			Self::ClickSend	=> "rest.clicksend.com",
			Self::Twilio	=> "api.twilio.com",
			Self::Plivo	=> "api.plivo.com",
		}
	}

	/// The `Authorization` value for a credential.
	///
	/// One place, because three vendors spelling out the same base64 would be three places for
	/// it to be spelled wrong.
	fn authorization(&self, cred: &Credential) -> String {
		fmt!("Basic {}", base64::encode(fmt!("{}:{}", cred.user, cred.secret).as_bytes()))
	}

	/// Host, path, method, headers and body for one message. **Not a request that is sent**:
	/// the caller owns the transport, the address check and the TLS.
	pub fn request(&self, cred: &Credential, m: &Message) -> Outcome<SmsCall> {
		// Checked here rather than left to the vendor, because a malformed number comes back as
		// a 400 with a vendor-specific code at the far end of a socket, and this is an alerting
		// path: the failure that matters is the one discovered while the operator is asleep.
		if !is_e164(m.to) {
			return Err(err!(
				"An SMS recipient must be in E.164 with a leading '+', got {:?}.", m.to;
				Invalid, Input));
		}
		if m.body.is_empty() {
			return Err(err!("An SMS needs something to say."; Invalid, Input, Missing));
		}
		if m.body.len() > MAX_BODY_LEN {
			return Err(err!(
				"An SMS body of {} bytes is past the {} this will carry. An alert longer than \
				that has stopped being an alert.", m.body.len(), MAX_BODY_LEN;
				Invalid, Input, TooBig));
		}

		let headers = |ct: &str| vec![
			(fmt!("Authorization"), self.authorization(cred)),
			(fmt!("Content-Type"), fmt!("{}", ct)),
			(fmt!("Accept"), fmt!("application/json")),
		];

		match self {
			Self::ClickSend => {
				// The account is identified by the credential alone, so the path is fixed and
				// carries nothing. One message per call: this is an alerter, not a campaign.
				let mut one = DaticleMap::new();
				one.insert(dat!("to"), dat!(m.to.to_string()));
				one.insert(dat!("body"), dat!(m.body.to_string()));
				if !m.from.is_empty() {
					one.insert(dat!("from"), dat!(m.from.to_string()));
				}
				let mut b = DaticleMap::new();
				b.insert(dat!("messages"), Dat::List(vec![Dat::Map(one)]));
				Ok(SmsCall {
					host:		self.host().to_string(),
					port:		443,
					path:		fmt!("/v3/sms/send"),
					method:		HttpMethod::POST,
					headers:	headers("application/json"),
					body:		res!(Dat::Map(b).json()).into_bytes(),
				})
			},
			Self::Twilio => {
				// The account identifier is in the path as well as in the credential. It is
				// escaped even though it is an opaque identifier of known shape, because a path
				// built by concatenation is a path that will one day be built from something
				// else.
				let path = fmt!("/2010-04-01/Accounts/{}/Messages.json",
					pct::encode_component(cred.user));
				let mut form = fmt!("To={}&Body={}",
					pct::encode_component(m.to), pct::encode_component(m.body));
				if !m.from.is_empty() {
					form.push_str(&fmt!("&From={}", pct::encode_component(m.from)));
				}
				Ok(SmsCall {
					host:		self.host().to_string(),
					port:		443,
					path,
					method:		HttpMethod::POST,
					headers:	headers("application/x-www-form-urlencoded"),
					body:		form.into_bytes(),
				})
			},
			Self::Plivo => {
				let path = fmt!("/v1/Account/{}/Message/", pct::encode_component(cred.user));
				let mut b = DaticleMap::new();
				b.insert(dat!("dst"), dat!(m.to.to_string()));
				b.insert(dat!("text"), dat!(m.body.to_string()));
				if !m.from.is_empty() {
					b.insert(dat!("src"), dat!(m.from.to_string()));
				}
				Ok(SmsCall {
					host:		self.host().to_string(),
					port:		443,
					path,
					method:		HttpMethod::POST,
					headers:	headers("application/json"),
					body:		res!(Dat::Map(b).json()).into_bytes(),
				})
			},
		}
	}

	/// An error document is surfaced with the provider's own words rather than a summary, since
	/// that text is where a vendor explains a rejected credential or an unfunded account.
	pub fn parse(&self, body: &[u8]) -> Outcome<Receipt> {
		let txt = match std::str::from_utf8(body) {
			Ok(s) => s,
			Err(e) => return Err(err!(e,
				"{} answered with something that is not text.", self.id(); Invalid, Input)),
		};
		let dat = match Dat::decode_string_with_config(txt.to_string(), &json_decoder()) {
			Ok(d) => d,
			Err(e) => return Err(err!(e,
				"{} answered with something that is not JSON: {}", self.id(), clip(txt);
				Network, Data, Decode)),
		};
		let map = match &dat {
			Dat::Map(m)	=> m,
			other		=> return Err(err!(
				"{} answered with a JSON {:?} rather than an object: {}",
				self.id(), other.kind(), clip(txt); Network, Data, Mismatch)),
		};

		// The error document first, and before the happy path, because two of these vendors
		// answer a rejected credential with HTTP 200 and an error object. A parser that read the
		// success fields first would find them absent and report a shape problem, hiding the
		// sentence that says the account is out of credit.
		if let Some(msg) = error_text(map) {
			return Err(err!("{} refused the message: {}", self.id(), msg; Invalid, Input));
		}

		match self {
			Self::ClickSend => {
				// data.messages[0]
				let one = res!(first_message(map, "data", "messages").ok_or_else(|| err!(
					"{} answered with no message record: {}", self.id(), clip(txt);
					Invalid, Input, Missing)));
				Ok(Receipt {
					id:	text(&one, "message_id"),
					status:	text(&one, "status"),
					parts:	number(&one, "message_parts"),
					price:	text(&one, "message_price"),
				})
			},
			Self::Twilio => Ok(Receipt {
				id:	text(map, "sid"),
				status:	text(map, "status"),
				parts:	number(map, "num_segments"),
				price:	text(map, "price"),
			}),
			Self::Plivo => {
				// The identifier arrives as a list of one, since the endpoint can take several
				// destinations. This module sends to one.
				let id = match map.get(&dat!("message_uuid")) {
					Some(Dat::List(l)) => l.first().map(scalar_text).unwrap_or_default(),
					Some(d) => scalar_text(d),
					None => String::new(),
				};
				Ok(Receipt {
					id,
					status:	text(map, "message"),
					parts:	0,
					price:	String::new(),
				})
			},
		}
	}
}

/// Is this an E.164 number?
///
/// A leading `+`, then between eight and fifteen digits and nothing else. Deliberately strict:
/// spaces, hyphens and brackets are how a human writes a number and every vendor here refuses
/// them, so accepting them would only move the refusal to the far end of a socket.
pub fn is_e164(s: &str) -> bool {
	let mut it = s.chars();
	if it.next() != Some('+') {
		return false;
	}
	let digits = s.len() - 1;
	digits >= 8 && digits <= 15 && it.all(|c| c.is_ascii_digit())
}

fn json_decoder() -> DecoderConfig<
	BTreeMap<UsrKindCode, UsrKind>,
	BTreeMap<String, UsrKindId>,
>
{
	DecoderConfig::json(None)
}

/// As much of a reply as belongs in an error message.
fn clip(s: &str) -> String {
	let s = s.trim();
	if s.len() <= 200 {
		return s.to_string();
	}
	fmt!("{}...", &s[..200])
}

/// A scalar as text, whatever the vendor made it.
///
/// A price arrives quoted from one vendor and bare from another; a segment count arrives as a
/// string from one and an integer from another. Reading either shape is not laxity, it is the
/// only way one receipt can describe three vendors without lying about one of them.
///
/// The integer widths are spelled out rather than left to `Display`, because a daticle knows its
/// own width and says so, and a receipt carrying `(u64|1)` where a person expected `1` is a
/// receipt that has quietly leaked the serialisation format into a support ticket.
fn scalar_text(d: &Dat) -> String {
	match d {
		Dat::Str(s)	=> s.clone(),
		Dat::Empty	=> String::new(),
		Dat::U8(n)	=> fmt!("{}", n),
		Dat::U16(n)	=> fmt!("{}", n),
		Dat::U32(n)	=> fmt!("{}", n),
		Dat::U64(n)	=> fmt!("{}", n),
		Dat::I32(n)	=> fmt!("{}", n),
		Dat::I64(n)	=> fmt!("{}", n),
		Dat::F32(n)	=> fmt!("{}", n),
		Dat::F64(n)	=> fmt!("{}", n),
		other		=> fmt!("{:?}", other),
	}
}

/// Empty when absent.
fn text(map: &DaticleMap, key: &str) -> String {
	map.get(&dat!(key)).map(scalar_text).unwrap_or_default()
}

/// Zero when absent or unreadable. Both shapes are read, because one vendor quotes its segment
/// count and another sends it as a number.
///
/// The widths are listed rather than parsed back out of a rendered daticle: a daticle prints its
/// own width, so `parse::<u32>()` over `Display` silently returned zero for every integer reply.
fn number(map: &DaticleMap, key: &str) -> u32 {
	match map.get(&dat!(key)) {
		Some(Dat::Str(s))	=> s.trim().parse::<u32>().unwrap_or(0),
		Some(Dat::U8(n))	=> *n as u32,
		Some(Dat::U16(n))	=> *n as u32,
		Some(Dat::U32(n))	=> *n,
		// Saturating rather than wrapping. A segment count cannot reach this, which is the
		// point: if one ever does, the reply is not a segment count and a large number is a
		// better clue than a small one produced by truncation.
		Some(Dat::U64(n))	=> (*n).min(u32::MAX as u64) as u32,
		Some(Dat::I32(n))	=> if *n > 0 { *n as u32 } else { 0 },
		Some(Dat::I64(n))	=> if *n > 0 { (*n as u64).min(u32::MAX as u64) as u32 } else { 0 },
		_			=> 0,
	}
}

/// `outer.inner[0]` as a map, for a vendor that nests its receipt in a list.
fn first_message(map: &DaticleMap, outer: &str, inner: &str) -> Option<DaticleMap> {
	let d = match map.get(&dat!(outer)) {
		Some(Dat::Map(m)) => m,
		_ => return None,
	};
	match d.get(&dat!(inner)) {
		Some(Dat::List(l)) => match l.first() {
			Some(Dat::Map(m)) => Some(m.clone()),
			_ => None,
		},
		_ => None,
	}
}

/// The vendor's own words for a refusal, where the document is one.
///
/// Checked before the success fields: two of these vendors answer a rejected credential with
/// HTTP 200 and an error object, so a parser that looked for the receipt first would report a
/// missing field where the vendor had written a sentence explaining itself.
fn error_text(map: &DaticleMap) -> Option<String> {
	// A response code that is present and is not a success is the plainest signal.
	if let Some(Dat::Str(code)) = map.get(&dat!("response_code")) {
		if !code.eq_ignore_ascii_case("SUCCESS") {
			let msg = text(map, "response_msg");
			return Some(if msg.is_empty() { code.clone() } else { fmt!("{} ({})", msg, code) });
		}
	}
	// Otherwise a message-shaped error field, under whichever name the vendor uses. `message`
	// is not among them: one vendor uses it for the SUCCESS text.
	for k in ["error", "error_message", "error-message", "detail"] {
		match map.get(&dat!(k)) {
			Some(Dat::Str(s)) if !s.is_empty() => return Some(s.clone()),
			Some(Dat::Map(m)) => {
				let msg = text(m, "message");
				if !msg.is_empty() {
					return Some(msg);
				}
			},
			_ => {},
		}
	}
	None
}


#[cfg(test)]
mod tests {
	use super::*;

	/// A header by name, compared as HTTP compares them.
	fn hdr<'a>(c: &'a SmsCall, name: &str) -> Option<&'a str> {
		c.headers.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case(name))
			.map(|(_, v)| v.as_str())
	}

	fn cred() -> Credential<'static> {
		Credential { user: "acct-identifier", secret: "s3cr3t-token-value" }
	}

	fn msg() -> Message<'static> {
		Message { to: "+61400000000", from: "", body: "jarrah gateway down" }
	}

	/// THE PROPERTY THIS MODULE PROMISES: the secret is in the headers and nowhere else.
	///
	/// Asserted per provider rather than once over a list, so a fourth arm added without
	/// thinking about it fails here rather than quietly widening the promise. See the module
	/// documentation for why a body is a worse place for a secret than a header.
	#[test]
	fn secret_only_ever_in_the_authorization_header() {
		let c = cred();
		let m = msg();
		for p in Provider::ALL {
			let call = match p.request(&c, &m) {
				Ok(call) => call,
				Err(e) => panic!("{} would not build a request: {}", p.id(), e),
			};
			let body = String::from_utf8_lossy(&call.body).to_string();
			assert!(!call.path.contains(c.secret),
				"{} put the secret in the path: {}", p.id(), call.path);
			assert!(!body.contains(c.secret),
				"{} put the secret in the body: {}", p.id(), body);
			// And it IS present, in the one place it belongs -- so this test cannot pass by
			// the credential having been dropped altogether.
			let auth = match hdr(&call, "authorization") {
				Some(a) => a,
				None => panic!("{} sent no Authorization header", p.id()),
			};
			let expect = base64::encode(fmt!("{}:{}", c.user, c.secret).as_bytes());
			assert_eq!(auth, fmt!("Basic {}", expect),
				"{} did not send the credential as Basic", p.id());
			assert!(!call.host.is_empty(), "{} did not name its host", p.id());
			assert_eq!(call.port, 443, "{} did not use TLS", p.id());
		}
	}

	/// The recipient and the text survive into the request, whatever the vendor's field names.
	#[test]
	fn the_message_reaches_the_body() {
		let c = cred();
		let m = msg();
		for p in Provider::ALL {
			let call = res_unwrap(p.request(&c, &m), p);
			let body = String::from_utf8_lossy(&call.body).to_string();
			// The number is percent-escaped in a form body and plain in a JSON one, so the
			// digits are what is looked for rather than the whole string.
			assert!(body.contains("61400000000"),
				"{} lost the recipient: {}", p.id(), body);
			assert!(body.contains("gateway") || body.contains("gateway%20"),
				"{} lost the text: {}", p.id(), body);
		}
	}

	#[test]
	fn a_number_that_is_not_e164_is_refused_before_a_socket_opens() {
		let c = cred();
		for bad in ["0400 000 000", "61400000000", "+61-400-000-000", "+123", ""] {
			let m = Message { to: bad, from: "", body: "x" };
			for p in Provider::ALL {
				assert!(p.request(&c, &m).is_err(),
					"{} accepted {:?} as a number", p.id(), bad);
			}
		}
		assert!(is_e164("+61400000000"));
		assert!(is_e164("+14155550123"));
	}

	#[test]
	fn an_empty_or_enormous_body_is_refused() {
		let c = cred();
		let long = "x".repeat(MAX_BODY_LEN + 1);
		for p in Provider::ALL {
			assert!(p.request(&c, &Message { to: "+61400000000", from: "", body: "" }).is_err(),
				"{} accepted an empty body", p.id());
			assert!(p.request(&c, &Message { to: "+61400000000", from: "", body: &long }).is_err(),
				"{} accepted a body past the cap", p.id());
		}
	}

	/// An id round-trips, and an unknown one is refused rather than defaulted.
	#[test]
	fn ids_are_one_spelling() {
		for p in Provider::ALL {
			assert_eq!(Provider::from_id(p.id()), Some(p));
			assert_eq!(Provider::from_id(&p.id().to_uppercase()), Some(p));
		}
		assert_eq!(Provider::from_id("vonage"), None);
		assert_eq!(Provider::from_id(""), None);
	}

	#[test]
	fn a_receipt_is_read_from_each_vendors_own_shape() {
		let cs = br#"{"http_code":200,"response_code":"SUCCESS","data":{"messages":[
			{"message_id":"ABC-123","status":"SUCCESS","message_parts":1,"message_price":"0.0790"}]}}"#;
		let r = res_unwrap(Provider::ClickSend.parse(cs), Provider::ClickSend);
		assert_eq!(r.id, "ABC-123");
		assert_eq!(r.parts, 1);
		assert_eq!(r.price, "0.0790", "the price is passed through verbatim");

		let tw = br#"{"sid":"SM9","status":"queued","num_segments":"2","price":null}"#;
		let r = res_unwrap(Provider::Twilio.parse(tw), Provider::Twilio);
		assert_eq!(r.id, "SM9");
		assert_eq!(r.status, "queued");
		assert_eq!(r.parts, 2, "a count quoted as a string is still a count");

		let pl = br#"{"message_uuid":["uu-1"],"message":"message(s) queued"}"#;
		let r = res_unwrap(Provider::Plivo.parse(pl), Provider::Plivo);
		assert_eq!(r.id, "uu-1", "the identifier is lifted out of its list of one");
	}

	/// A refusal that arrives with HTTP 200 is still a refusal, and it says why.
	#[test]
	fn an_error_document_is_an_error_and_repeats_the_vendors_words() {
		let out_of_credit = br#"{"http_code":400,"response_code":"NO_CREDIT",
			"response_msg":"Insufficient credit"}"#;
		match Provider::ClickSend.parse(out_of_credit) {
			Ok(r) => panic!("an unfunded account read as a receipt: {:?}", r),
			Err(e) => {
				let s = e.to_string();
				assert!(s.contains("Insufficient credit"),
					"the vendor's own sentence was thrown away: {}", s);
			},
		}
		let bad_key = br#"{"status":401,"message":"Authenticate","error":"authentication failed"}"#;
		assert!(Provider::Twilio.parse(bad_key).is_err(),
			"a rejected credential read as a receipt");
	}

	/// Unwrap in a test, naming which provider failed.
	fn res_unwrap<T>(r: Outcome<T>, p: Provider) -> T {
		match r {
			Ok(v) => v,
			Err(e) => panic!("{}: {}", p.id(), e),
		}
	}
}
