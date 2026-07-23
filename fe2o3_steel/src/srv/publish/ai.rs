//! The AI settings a site keeps, and the calls it makes with them.
//!
//! One record holds everything the operator sets on the AI panel: which model to call and the key to
//! call it with, the instruction sent with a post being "fixed" and the one sent with a comment being
//! judged, and the addresses to email when a comment is held for a human. The key is a secret and is
//! stored exactly as the destination tokens are -- a `Dat` in the vhost's own database, encrypted at
//! rest under the database's scheme -- so it is not a key in a file in the clear, and it is never
//! logged.
//!
//! The client that makes the call lives upstream in [`oxedyne_fe2o3_net::llm`]; this module is the
//! settings around it, and the two prompts a site sends with its two kinds of request.

use crate::srv::publish::subscribe;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};
use oxedyne_fe2o3_net::llm::{
	self,
	LlmConfig,
	Provider,
};

use std::sync::{
	Arc,
	RwLock,
};

use tokio_rustls::rustls::ClientConfig;


/// The key a vhost's AI settings live under in its store.
pub const AI_KEY: &str = "publish/ai";

/// The instruction sent with a post the author asks to fix, where the operator has set none.
///
/// Deliberately narrow. These blogs are written by hand for their own sake, so the one job this prompt
/// gives the model is to correct what is plainly wrong -- spelling, grammar, punctuation -- and to
/// leave the voice, the word choice and the argument exactly as they were. The author reviews the
/// result before it replaces anything, so the prompt errs towards doing too little rather than too
/// much: a fix that changed the meaning is a worse failure than a typo it left alone.
pub const FIX_PROMPT_DEFAULT: &str =
	"You are a meticulous copy-editor for a personal blog. Correct only clear errors of spelling, \
	grammar and punctuation in the text below. Do not change the author's wording, voice, tone, \
	structure or meaning; do not add, remove or reorder ideas; do not rewrite for style. If a \
	passage is already correct, leave it untouched. Return only the corrected text, with no preamble, \
	no explanation and no markup you were not given.";

/// The instruction sent with a comment being judged, where the operator has set none.
///
/// One word out, so the reply maps cleanly to a verdict. The three words are the three a moderator can
/// reach: publish it, bin it, or hold it for a person. The prompt is told to prefer holding when
/// unsure, because the cost of holding a good comment is a short wait and the cost of publishing a bad
/// one is a bad comment on the page.
pub const COMMENT_PROMPT_DEFAULT: &str =
	"You are moderating reader comments on a personal blog. Judge only the comment text below. Reply \
	with exactly one word and nothing else: APPROVE if it is a genuine, civil, on-topic comment; SPAM \
	if it is advertising, a link farm, gibberish or off-topic promotion; HOLD if you are unsure or it \
	is borderline. Prefer HOLD over APPROVE when in doubt.";


/// Everything a site sets on its AI panel.
///
/// Empty strings are the unset state throughout, not a record of blanks: a blank provider is "no AI",
/// a blank prompt means "use the default", a blank key means "nothing stored". So a fresh site and a
/// site that has cleared its settings read the same, which is what they mean.
#[derive(Clone, Debug, Default)]
pub struct AiSettings {
	/// The provider word: `""` (unset), `openrouter`, `fireworks` or `mistral`.
	pub provider:	String,
	/// The model, in the provider's own naming.
	pub model:	String,
	/// The bearer key. Write-only from the console, never logged.
	pub api_key:	String,
	/// The fix instruction. Empty means [`FIX_PROMPT_DEFAULT`].
	pub fix_prompt:	String,
	/// The comment instruction. Empty means [`COMMENT_PROMPT_DEFAULT`].
	pub comment_prompt:	String,
	/// The addresses emailed when a comment is held for a human. Empty means nobody is told, and the
	/// comment simply waits in the console queue.
	pub alert_emails:	Vec<String>,
}

impl AiSettings {

	/// The record as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("provider"),	dat!(self.provider.clone()));
		m.insert(dat!("model"),		dat!(self.model.clone()));
		m.insert(dat!("api_key"),	dat!(self.api_key.clone()));
		m.insert(dat!("fix_prompt"),	dat!(self.fix_prompt.clone()));
		m.insert(dat!("comment_prompt"),dat!(self.comment_prompt.clone()));
		m.insert(dat!("alert_emails"),
			Dat::List(self.alert_emails.iter().map(|e| dat!(e.clone())).collect()));
		Dat::Map(m)
	}

	/// The record from a daticle, tolerant of a missing field so an older record still reads.
	pub fn from_dat(d: &Dat) -> Self {
		let s = |m: &DaticleMap, k: &str| match m.get(&dat!(k)) {
			Some(Dat::Str(v))	=> v.clone(),
			_			=> String::new(),
		};
		match d {
			Dat::Map(m)	=> Self {
				provider:	s(m, "provider"),
				model:		s(m, "model"),
				api_key:	s(m, "api_key"),
				fix_prompt:	s(m, "fix_prompt"),
				comment_prompt:	s(m, "comment_prompt"),
				alert_emails:	match m.get(&dat!("alert_emails")) {
					Some(Dat::List(l))	=> l.iter().filter_map(|e| match e {
						Dat::Str(v)	=> Some(v.clone()),
						_		=> None,
					}).collect(),
					_			=> Vec::new(),
				},
			},
			_		=> Self::default(),
		}
	}

	/// Whether a call can be made: a provider, a model and a key are all set.
	pub fn ready(&self) -> bool {
		!self.provider.trim().is_empty()
			&& !self.model.trim().is_empty()
			&& !self.api_key.trim().is_empty()
	}

	/// The fix instruction to send: the operator's where they set one, the default otherwise.
	pub fn fix_prompt(&self) -> &str {
		if self.fix_prompt.trim().is_empty() { FIX_PROMPT_DEFAULT } else { &self.fix_prompt }
	}

	/// The comment instruction to send.
	pub fn comment_prompt(&self) -> &str {
		if self.comment_prompt.trim().is_empty() { COMMENT_PROMPT_DEFAULT } else { &self.comment_prompt }
	}

	/// The connection config for a call, or the reason there is none.
	pub fn llm(&self) -> Outcome<LlmConfig> {
		if !self.ready() {
			return Err(err!(
				"The site's AI is not configured: it needs a provider, a model and a key.";
				Invalid, Input, Missing));
		}
		Ok(LlmConfig {
			provider:	res!(Provider::of(self.provider.trim())),
			model:		self.model.trim().to_string(),
			api_key:	self.api_key.clone(),
		})
	}
}

/// What the model said to do with a comment.
///
/// The three a moderator can reach, and no more. Kept here rather than in the comment module so the
/// dependency runs one way -- the comment module maps this to its own verdict -- and so the parsing
/// of a model's reply lives beside the prompt that asked for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommentVerdict {
	/// Publish it.
	Approve,
	/// Bin it.
	Spam,
	/// A person should look.
	Hold,
}

/// Reads a one-word reply into a verdict.
///
/// The prompt asks for one word, but a model is not a promise: it may wrap the word in a sentence, add
/// a full stop, or answer in a different case. So this looks for the first of the three words anywhere
/// in the reply, case-folded, and **falls back to holding** -- the safe direction -- when it finds
/// none. A reply that says nothing recognisable is exactly when a person should look, not when a guess
/// should publish.
pub fn parse_comment_verdict(reply: &str) -> CommentVerdict {
	let up = reply.to_uppercase();
	// SPAM is checked before APPROVE so a reply that mentions both ("not spam, approve") is read the
	// strict way; a model torn between the two is a comment worth holding, but binning is the safer
	// of the two decisive readings and the words rarely co-occur innocently.
	if up.contains("SPAM") {
		CommentVerdict::Spam
	} else if up.contains("APPROVE") {
		CommentVerdict::Approve
	} else {
		CommentVerdict::Hold
	}
}

/// Judges a comment's text with the site's model.
///
/// `None` where the call could not be made -- AI is not configured, or the model would not answer --
/// so the caller keeps the comment held rather than treating a network failure as a decision. A
/// judgement is only ever `Some` when the model actually spoke. The comment's text is all that is sent;
/// no address, no name, nothing about who wrote it, since none of that is the model's business here.
pub async fn judge_comment(
	settings:	&AiSettings,
	tls:		Arc<ClientConfig>,
	body:		&str,
)
	-> Option<CommentVerdict>
{
	let cfg = settings.llm().ok()?;
	match llm::complete(&cfg, settings.comment_prompt(), body, tls).await {
		Ok(reply)	=> Some(parse_comment_verdict(&reply)),
		Err(_)		=> None,
	}
}

/// Parses an address list a person typed -- one per line, or separated by commas -- keeping the valid
/// ones in order and dropping blanks and duplicates.
///
/// Lenient on the separators because a person pasting addresses should not have to care which this
/// wanted, and strict on the addresses because an alert sent to a malformed one is an alert lost.
pub fn parse_emails(raw: &str) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	for part in raw.split(|c| c == '\n' || c == '\r' || c == ',' || c == ';' || c == ' ') {
		let e = part.trim().to_lowercase();
		if e.is_empty() || !subscribe::valid_email(&e) {
			continue;
		}
		if !out.iter().any(|x| x == &e) {
			out.push(e);
		}
	}
	out
}

/// The AI settings a site has stored. The default (all empty) where none are stored, which is not an
/// error: a site that wants no AI stores nothing.
pub fn get_settings<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
)
	-> Outcome<AiSettings>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	match res!(guard.get(&dat!(AI_KEY), None)) {
		Some((val, _))	=> Ok(AiSettings::from_dat(&val)),
		None		=> Ok(AiSettings::default()),
	}
}

/// Writes a site's AI settings to its store, where the key is encrypted at rest under the database's
/// own scheme -- the same treatment its posts, its sessions and its destination tokens get.
pub fn put_settings<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	settings: &AiSettings,
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	let guard = lock_read!(db_arc);
	res!(guard.insert(dat!(AI_KEY), settings.to_dat(), *user, None));
	Ok(())
}


#[cfg(test)]
mod tests {
	use super::*;

	/// The settings round-trip through a daticle, the key and all.
	#[test]
	fn test_settings_round_trip_00() -> Outcome<()> {
		let s = AiSettings {
			provider:	fmt!("mistral"),
			model:		fmt!("mistral-large-latest"),
			api_key:	fmt!("sk-secret"),
			fix_prompt:	fmt!("Fix it."),
			comment_prompt:	String::new(),
			alert_emails:	vec![fmt!("me@example.com"), fmt!("also@example.com")],
		};
		let back = AiSettings::from_dat(&s.to_dat());
		assert_eq!(back.provider, "mistral");
		assert_eq!(back.model, "mistral-large-latest");
		assert_eq!(back.api_key, "sk-secret");
		assert_eq!(back.fix_prompt, "Fix it.");
		assert_eq!(back.alert_emails, vec![fmt!("me@example.com"), fmt!("also@example.com")]);
		Ok(())
	}

	/// An empty prompt reads as its default; a set one reads as itself. So the panel can prefill the
	/// default without freezing it, and clearing the box restores the default rather than sending none.
	#[test]
	fn test_a_blank_prompt_is_the_default_01() -> Outcome<()> {
		let mut s = AiSettings::default();
		assert_eq!(s.fix_prompt(), FIX_PROMPT_DEFAULT);
		assert_eq!(s.comment_prompt(), COMMENT_PROMPT_DEFAULT);
		s.fix_prompt = fmt!("Only fix spelling.");
		assert_eq!(s.fix_prompt(), "Only fix spelling.");
		Ok(())
	}

	/// A call is possible only with a provider, a model and a key; anything short of that is a clear
	/// error, not a call that fails at the socket.
	#[test]
	fn test_ready_needs_all_three_02() -> Outcome<()> {
		let mut s = AiSettings::default();
		assert!(!s.ready());
		assert!(s.llm().is_err());
		s.provider = fmt!("openrouter");
		s.model = fmt!("some/model");
		assert!(!s.ready(), "no key is not ready");
		s.api_key = fmt!("k");
		assert!(s.ready());
		let cfg = res!(s.llm());
		assert_eq!(cfg.provider, Provider::OpenRouter);
		assert_eq!(cfg.model, "some/model");
		// An unknown provider word is refused at the point of the call.
		s.provider = fmt!("nope");
		assert!(s.llm().is_err());
		Ok(())
	}

	/// A one-word reply reads to a verdict; a reply in a sentence, in the wrong case, or saying
	/// nothing recognisable still reads safely -- towards holding, never towards publishing on a guess.
	#[test]
	fn test_comment_verdict_parsing_04() -> Outcome<()> {
		assert_eq!(parse_comment_verdict("APPROVE"), CommentVerdict::Approve);
		assert_eq!(parse_comment_verdict("spam"), CommentVerdict::Spam);
		assert_eq!(parse_comment_verdict("HOLD"), CommentVerdict::Hold);
		// Wrapped in a sentence, still read.
		assert_eq!(parse_comment_verdict("This looks fine, so APPROVE."), CommentVerdict::Approve);
		// Torn between the two decisive words is read the strict way.
		assert_eq!(parse_comment_verdict("not spam, I would approve"), CommentVerdict::Spam);
		// Nothing recognisable holds, rather than guessing publish.
		assert_eq!(parse_comment_verdict("I am not sure about this one."), CommentVerdict::Hold);
		assert_eq!(parse_comment_verdict(""), CommentVerdict::Hold);
		Ok(())
	}

	/// The address list takes newlines or commas, keeps the valid in order, and drops blanks,
	/// duplicates and anything malformed -- since an alert to a bad address is an alert lost.
	#[test]
	fn test_email_parsing_03() -> Outcome<()> {
		let got = parse_emails("  A@Example.com ,\n b@x.org\n\n a@example.com , not-an-email ,c@y.net ");
		assert_eq!(got, vec![fmt!("a@example.com"), fmt!("b@x.org"), fmt!("c@y.net")]);
		assert!(parse_emails("   \n , ; ").is_empty());
		Ok(())
	}
}
