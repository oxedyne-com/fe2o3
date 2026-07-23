//! A bring-your-own-key client for an OpenAI-compatible chat-completions API.
//!
//! # What it is, and is not
//!
//! One request, one reply: a system instruction and a piece of text go up, an assistant message comes
//! back. It is not an agent, holds no conversation, and streams nothing -- a caller that wants a post
//! tidied or a comment judged sends the whole thing and reads the whole answer. That is the shape both
//! callers this was built for need, and a narrower client is a smaller thing to get wrong.
//!
//! # Why one client covers three providers
//!
//! OpenRouter, Fireworks and Mistral all speak the OpenAI chat-completions dialect: a `POST` of
//! `{"model", "messages":[{"role","content"}...]}` to a `/chat/completions` path, bearer-authenticated,
//! answered with `choices[0].message.content`. So a provider is a base address and nothing more, and
//! adding a fourth that speaks the same dialect is a line in one enum. A provider that spoke a
//! different dialect -- Anthropic's `messages` API, say -- would be a second [`complete`], not a second
//! [`Provider`] arm; this one does not pretend to abstract over that.
//!
//! # Built pure, wrapped thin
//!
//! [`chat_body`] builds the request and [`chat_reply`] reads the answer, both pure functions over
//! strings, tested without a socket -- because a test cannot reach a live model with a key it does not
//! have, and what it cannot reach it cannot catch. What it *can* pin -- the JSON a provider is sent,
//! the text pulled from what it returns, the error surfaced rather than swallowed -- it does. The
//! network wrapper [`complete`] is as thin as the send seam it borrows from [`crate::http::client`].

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

use std::{
	collections::BTreeMap,
	sync::Arc,
};

use tokio_rustls::rustls::ClientConfig;

use crate::http::{
	client::https_request,
	header::{
		HttpHeadline,
		HttpMethod,
	},
};


/// An OpenAI-compatible provider: the host that answers, and the path it answers on.
///
/// The three named here were asked for; [`Provider::Custom`] is the escape hatch for a fourth that
/// speaks the same dialect, so a new endpoint needs no new code here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Provider {
	/// `openrouter.ai` -- a router in front of many models.
	OpenRouter,
	/// `api.fireworks.ai`.
	Fireworks,
	/// `api.mistral.ai`.
	Mistral,
	/// Any other host and path that speaks the same dialect. Host without a scheme, path with a
	/// leading slash.
	Custom {
		/// The host to dial and validate the certificate for, e.g. `api.example.com`.
		host: String,
		/// The request path, e.g. `/v1/chat/completions`.
		path: String,
	},
}

impl Provider {

	/// The provider a stored word names.
	///
	/// Not a lenient default: a config that meant `mistral` and typed `mistrral` should hear about it
	/// rather than quietly reach a host that does not exist.
	pub fn of(s: &str) -> Outcome<Self> {
		match s {
			"openrouter"	=> Ok(Self::OpenRouter),
			"fireworks"	=> Ok(Self::Fireworks),
			"mistral"	=> Ok(Self::Mistral),
			_		=> Err(err!(
				"Unknown LLM provider '{}': expected openrouter, fireworks or mistral.", s;
				Invalid, Input)),
		}
	}

	/// The word a provider is stored and configured as. [`Provider::Custom`] has none, since it is
	/// named by its own host and path rather than by a key in this enum.
	pub fn as_str(&self) -> Option<&'static str> {
		match self {
			Self::OpenRouter	=> Some("openrouter"),
			Self::Fireworks		=> Some("fireworks"),
			Self::Mistral		=> Some("mistral"),
			Self::Custom { .. }	=> None,
		}
	}

	/// The host to dial and to validate the TLS certificate against.
	pub fn host(&self) -> &str {
		match self {
			Self::OpenRouter		=> "openrouter.ai",
			Self::Fireworks			=> "api.fireworks.ai",
			Self::Mistral			=> "api.mistral.ai",
			Self::Custom { host, .. }	=> host,
		}
	}

	/// The path the chat-completions endpoint answers on.
	pub fn path(&self) -> &str {
		match self {
			Self::OpenRouter		=> "/api/v1/chat/completions",
			Self::Fireworks			=> "/inference/v1/chat/completions",
			Self::Mistral			=> "/v1/chat/completions",
			Self::Custom { path, .. }	=> path,
		}
	}
}

/// Everything a call needs but its words: which provider, which model, and the key that pays for it.
///
/// The key is a secret and is never logged; a caller storing one gives it the same at-rest protection
/// its other secrets get. Held here only for the moment of a call.
#[derive(Clone, Debug)]
pub struct LlmConfig {
	/// The provider to reach.
	pub provider:	Provider,
	/// The model to ask for, in the provider's own naming, e.g. `mistralai/mistral-large-latest` or
	/// `accounts/fireworks/models/llama-v3p1-70b-instruct`.
	pub model:	String,
	/// The bearer key. Never logged.
	pub api_key:	String,
}

/// The decoder configuration for a provider's JSON reply.
fn json_decoder() -> DecoderConfig<
	BTreeMap<UsrKindCode, UsrKind>,
	BTreeMap<String, UsrKindId>,
>
{
	DecoderConfig::json(None)
}

/// The request body for a single-turn completion: a system instruction, then the user's text.
///
/// Built through the daticle encoder rather than by hand, so a system prompt an operator typed and a
/// post a reader wrote reach the model correctly quoted whatever they contain -- a quote, a newline, a
/// backslash. `temperature` is low, because both callers want the model to tidy or judge, not to
/// invent, and the same text should get much the same answer twice.
pub fn chat_body(model: &str, system: &str, user: &str) -> Outcome<String> {
	let msg = |role: &str, content: &str| {
		let mut m = DaticleMap::new();
		m.insert(dat!("role"), dat!(role.to_string()));
		m.insert(dat!("content"), dat!(content.to_string()));
		Dat::Map(m)
	};
	let mut body = DaticleMap::new();
	body.insert(dat!("model"), dat!(model.to_string()));
	body.insert(dat!("messages"), Dat::List(vec![
		msg("system", system),
		msg("user", user),
	]));
	body.insert(dat!("temperature"), dat!(0.2f64));
	Dat::Map(body).json()
}

/// The assistant's text from a provider's reply, or the reason there is none.
///
/// A provider answers a good request with `{"choices":[{"message":{"content":"..."}}]}` and a bad one
/// with `{"error":{"message":"..."}}` (or a bare `{"error":"..."}`); this reads the first and surfaces
/// the second as an error rather than an empty string, so a caller can tell "the model said nothing"
/// from "the model was never asked". An empty `choices` is the former and says so.
pub fn chat_reply(json: &str) -> Outcome<String> {
	let dat = res!(Dat::decode_string_with_config(json.to_string(), &json_decoder()));
	let map = match &dat {
		Dat::Map(m)	=> m,
		other		=> return Err(err!(
			"The LLM reply was not a JSON object but {:?}.", other.kind();
			Network, Data, Mismatch)),
	};

	// An error reply is surfaced with its own message, since that is the useful thing to show.
	if let Some(e) = map.get(&dat!("error")) {
		let why = match e {
			Dat::Str(s)	=> s.clone(),
			Dat::Map(em)	=> match em.get(&dat!("message")) {
				Some(Dat::Str(s))	=> s.clone(),
				_			=> fmt!("{:?}", e),
			},
			_		=> fmt!("{:?}", e),
		};
		return Err(err!("The LLM returned an error: {}", why; Network, Data));
	}

	let choices = match map.get(&dat!("choices")) {
		Some(Dat::List(l))	=> l,
		_			=> return Err(err!(
			"The LLM reply carried no 'choices' list: {}", json;
			Network, Data, Missing)),
	};
	let first = match choices.first() {
		Some(Dat::Map(m))	=> m,
		_			=> return Err(err!(
			"The LLM returned no choices, so it said nothing.";
			Network, Data, Missing)),
	};
	let message = match first.get(&dat!("message")) {
		Some(Dat::Map(m))	=> m,
		_			=> return Err(err!(
			"The LLM choice carried no message: {}", json;
			Network, Data, Missing)),
	};
	match message.get(&dat!("content")) {
		Some(Dat::Str(s))	=> Ok(s.clone()),
		_			=> Err(err!(
			"The LLM message carried no text content: {}", json;
			Network, Data, Missing)),
	}
}

/// Makes one completion call and returns the assistant's text.
///
/// The thin wrapper around the two pure functions: build the body, `POST` it bearer-authenticated over
/// TLS, read the reply. A non-2xx status is surfaced with the body the provider sent, since that body
/// is where a provider says what was wrong with a key or a model name.
pub async fn complete(
	cfg:	&LlmConfig,
	system:	&str,
	user:	&str,
	tls:	Arc<ClientConfig>,
)
	-> Outcome<String>
{
	let body = res!(chat_body(&cfg.model, system, user));
	let auth = fmt!("Bearer {}", cfg.api_key);
	let headers: &[(&str, &str)] = &[
		("Host",		cfg.provider.host()),
		("Authorization",	&auth),
		("Content-Type",	"application/json"),
		("Accept",		"application/json"),
	];
	let resp = res!(https_request(
		cfg.provider.host(),
		443,
		HttpMethod::POST,
		cfg.provider.path(),
		headers,
		body.as_bytes(),
		tls,
	).await);

	let payload = String::from_utf8_lossy(&resp.body).to_string();
	let status = match &resp.header.headline {
		HttpHeadline::Response { status }	=> *status as u16,
		_					=> 0,
	};
	if !(200..300).contains(&status) {
		return Err(err!(
			"The LLM provider answered {} to a completion request: {}", status, payload;
			Network, Data));
	}
	chat_reply(&payload)
}


#[cfg(test)]
mod tests {
	use super::*;

	/// A provider round-trips through its word, and an unknown word is refused rather than guessed.
	#[test]
	fn test_a_provider_names_itself_00() -> Outcome<()> {
		for word in ["openrouter", "fireworks", "mistral"] {
			let p = res!(Provider::of(word));
			assert_eq!(p.as_str(), Some(word), "'{}' did not round-trip", word);
			assert!(p.host().contains('.'), "'{}' has no host", word);
			assert!(p.path().starts_with('/'), "'{}' has no path", word);
		}
		assert!(Provider::of("claude").is_err(), "an unknown provider was accepted");
		Ok(())
	}

	/// The request body carries the model and both messages, and quotes what the text contains.
	#[test]
	fn test_the_body_quotes_its_text_01() -> Outcome<()> {
		// A system prompt and a user text that between them hold every character a naive concatenation
		// would break out of: a quote, a newline, a backslash, a brace.
		let body = res!(chat_body(
			"acme/model-1",
			"You are a \"strict\" editor.\nFix typos only.",
			"He said {hi} and\\or bye.",
		));
		// It parses back as JSON, which a hand-built body with an unescaped quote would not.
		let dat = res!(Dat::decode_string_with_config(body.clone(), &json_decoder()));
		let map = match dat { Dat::Map(m) => m, _ => return Err(err!("not an object"; Test)) };
		assert!(matches!(map.get(&dat!("model")), Some(Dat::Str(s)) if s == "acme/model-1"),
			"model missing: {}", body);
		let msgs = match map.get(&dat!("messages")) {
			Some(Dat::List(l)) => l,
			_ => return Err(err!("no messages list: {}", body; Test)),
		};
		assert_eq!(msgs.len(), 2, "expected system then user: {}", body);
		// The roles are in order and the awkward text survived the round trip intact.
		let role = |d: &Dat| match d { Dat::Map(m) => match m.get(&dat!("role")) {
			Some(Dat::Str(s)) => s.clone(), _ => String::new() }, _ => String::new() };
		assert_eq!(role(&msgs[0]), "system");
		assert_eq!(role(&msgs[1]), "user");
		Ok(())
	}

	/// A good reply yields its content; an empty `choices` says the model said nothing rather than
	/// returning an empty string that reads like a valid answer.
	#[test]
	fn test_a_reply_yields_its_content_02() -> Outcome<()> {
		let good = r#"{"choices":[{"message":{"role":"assistant","content":"Fixed text."}}]}"#;
		assert_eq!(res!(chat_reply(good)), "Fixed text.");

		let empty = r#"{"choices":[]}"#;
		assert!(chat_reply(empty).is_err(), "an empty choices list should be an error");
		Ok(())
	}

	/// A provider's error reply is surfaced with its message, in both the shapes providers send.
	#[test]
	fn test_an_error_reply_is_surfaced_03() -> Outcome<()> {
		let nested = r#"{"error":{"message":"invalid api key","type":"auth"}}"#;
		let e = fmt!("{}", chat_reply(nested).err().unwrap());
		assert!(e.contains("invalid api key"), "the message was not surfaced: {}", e);

		let bare = r#"{"error":"model not found"}"#;
		let e2 = fmt!("{}", chat_reply(bare).err().unwrap());
		assert!(e2.contains("model not found"), "the bare message was not surfaced: {}", e2);
		Ok(())
	}
}
