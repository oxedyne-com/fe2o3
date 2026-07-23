//! Comments on a post: what one is, where it is kept, and what decides whether it appears.
//!
//! # The shape of the thing
//!
//! A comment is prose by somebody who is not the author, attached to a post, possibly in reply to
//! another comment. It is written in the same Markdown the posts are, parsed to the same tree, and
//! rendered through [`fe2o3_text::doc::policy`](oxedyne_fe2o3_text::doc::policy) first -- which is
//! what makes a stranger's link safe to publish. Nothing here renders anything; that belongs to the
//! page, and this owns what is stored and what is decided.
//!
//! # Three seams, deliberately
//!
//! Two of them are not used yet and exist so that what comes later drops in rather than rewrites:
//!
//! - [`Identity`] is who a commenter is. Today that is a name and an optional address; the variant
//!   for a network identity is present and unfilled.
//! - [`Moderator`] is what decides. Today that is [`Rules`](Moderator::Rules), which is arithmetic.
//!   A moderator that asks a model is the same seam with a different arm.
//! - [`Ranker`] is what order comments come back in. Today chronological, oldest first, which is how
//!   a conversation reads. A ranker that weighs something is the same seam with a different arm.
//!
//! # What a comment costs a reader
//!
//! Nothing. There is no third-party script, no avatar fetched from elsewhere, no image a commenter
//! can place (see the policy's reasoning), and no identifier stored about who *read* a thread. A
//! commenter's address, where they give one, is stored and **never rendered and never returned by
//! any endpoint** -- it exists to notify them of a reply and for nothing else.

use crate::srv::publish::{
	Markup,
	ai,
	parse_markup,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_core::rand::Rand;
use oxedyne_fe2o3_hash::hash::HashScheme;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::{
	Database,
	ScanOpts,
};
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
	prelude::*,
	id::NumIdDat,
};

use std::sync::{
	Arc,
	RwLock,
};

use tokio_rustls::rustls::ClientConfig;


/// The key every comment's key begins with.
///
/// A comment's key carries its post's slug, so every comment on a post is one prefix scan and a read
/// per comment -- the shape the posts themselves take, and for the same reason: nothing walks the
/// whole database to draw one page.
pub const KEY_PREFIX: &str = "publish/comment/";

/// The key every commenter record begins with.
///
/// A commenter is remembered only so that somebody already approved is not made to wait again. See
/// [`Commenter`].
pub const AUTHOR_PREFIX: &str = "publish/commenter/";

/// The longest a comment may be, in bytes of source.
///
/// Long enough for a considered reply and short enough that a page of them is a page. A limit that
/// exists at all is the point; the number is a judgement.
pub const BODY_MAX: usize = 8_000;

/// The longest a display name may be.
pub const NAME_MAX: usize = 64;

/// How deep a reply may nest.
///
/// Three is the depth at which a thread is still a conversation and not a staircase. A reply deeper
/// than this attaches to its grandparent instead of being refused: the person meant to reply to
/// something, and losing their words to a structural rule would be the wrong answer.
pub const DEPTH_MAX: usize = 3;

/// How many comments may be waiting on one post before it stops taking more.
///
/// The bound on what an unauthenticated write can cost. A comment that is held is storage somebody
/// else chose to spend, and without a ceiling a machine that ignores the proof-of-work can spend it
/// without limit. Once a post's queue is this full it takes nothing further until a person clears
/// some -- which is a visible, recoverable state, unlike a disk that filled overnight.
///
/// Approved comments are deliberately **not** counted: those are storage the site's own admin chose.
pub const PENDING_MAX: usize = 50;

/// How many comments one post will hold, in any state.
///
/// Every comment on a post is read back whenever the post is viewed, so the store is not merely disk:
/// it is work done on behalf of every reader, for ever. A thousand is far past any conversation worth
/// having and well short of a page that will not serve.
pub const POST_MAX: usize = 1_000;

/// The alphabet and length a comment's id is minted from.
const ID_LEN: usize = 16;
const ID_ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz0123456789";


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MODEL                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Where a comment stands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CommentState {
	/// Waiting on a human. What every comment by an unknown author is.
	#[default]
	Pending,
	/// Published, and visible to a reader.
	Approved,
	/// Judged spam. Kept rather than deleted, so a wrong judgement is recoverable and so a record of
	/// what arrives exists.
	Spam,
	/// Taken down after having been published, by the author of the site or the commenter.
	Removed,
}

impl CommentState {

	/// The word a record stores.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Pending	=> "pending",
			Self::Approved	=> "approved",
			Self::Spam	=> "spam",
			Self::Removed	=> "removed",
		}
	}

	/// The state a word names.
	///
	/// **An unknown word is pending**, which is the safe reading for the same reason a subscriber's
	/// is: a state this version cannot place must not thereby be published. A record written by a
	/// later version showing up as awaiting review is a visible, harmless failure; showing up as
	/// approved would be an invisible, harmful one.
	pub fn of(s: &str) -> Self {
		match s {
			"approved"	=> Self::Approved,
			"spam"		=> Self::Spam,
			"removed"	=> Self::Removed,
			_		=> Self::Pending,
		}
	}

	/// Whether a comment in this state is shown to a reader.
	pub fn is_public(&self) -> bool {
		matches!(self, Self::Approved)
	}
}

/// Who wrote a comment.
///
/// The seam of row 42. Two arms are live and the third is the shape of what is coming: an identity
/// vouched for by a network rather than by an address the person typed. Nothing outside this module
/// matches on the variant to decide whether to *publish*; that is the moderator's business, and
/// keeping it there is what lets a new arm arrive without touching the pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Identity {
	/// A name, and an address they chose to give. The address is never shown.
	Local {
		/// What the reader sees.
		name:	String,
		/// Where a reply notification would go, if they asked for one. Never rendered, never
		/// returned by an endpoint, never given to a third party.
		email:	Option<String>,
	},
	/// No name given. Shown as the site's word for a stranger.
	Anon,
	/// An identity a network vouches for. **Not yet issued by anything** -- the arm exists so that
	/// storage, moderation and rendering already handle it when it is.
	Vouched {
		/// The identifier the network knows them by.
		id:	String,
		/// What the reader sees.
		name:	String,
	},
}

impl Default for Identity {
	fn default() -> Self { Self::Anon }
}

impl Identity {

	/// The name a reader sees.
	pub fn display_name(&self) -> &str {
		match self {
			Self::Local { name, .. }	=> name,
			Self::Vouched { name, .. }	=> name,
			Self::Anon			=> "Anonymous",
		}
	}

	/// The address to reach them at, where there is one.
	pub fn email(&self) -> Option<&str> {
		match self {
			Self::Local { email, .. }	=> email.as_deref(),
			_				=> None,
		}
	}

	/// The stable handle by which this commenter is remembered between comments, if any.
	///
	/// **An identity with no handle can never become trusted**, and that is the honest consequence of
	/// letting people comment without identifying themselves: there is nothing to attach the trust
	/// to. Such comments wait for a human every time. An address is the handle where one is given; a
	/// vouched identity is its own. A bare name is deliberately *not* a handle -- anyone can type
	/// somebody else's name, and treating that as identity would let a stranger inherit another
	/// person's approval.
	pub fn handle(&self) -> Option<String> {
		match self {
			Self::Local { email: Some(e), .. } if !e.trim().is_empty()
				=> Some(fmt!("email:{}", e.trim().to_lowercase())),
			Self::Vouched { id, .. }
				=> Some(fmt!("vouched:{}", id)),
			_	=> None,
		}
	}

	/// The identity as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		match self {
			Self::Local { name, email } => {
				m.insert(dat!("kind"), dat!("local".to_string()));
				m.insert(dat!("name"), dat!(name.clone()));
				if let Some(e) = email {
					m.insert(dat!("email"), dat!(e.clone()));
				}
			}
			Self::Anon => {
				m.insert(dat!("kind"), dat!("anon".to_string()));
			}
			Self::Vouched { id, name } => {
				m.insert(dat!("kind"), dat!("vouched".to_string()));
				m.insert(dat!("id"),   dat!(id.clone()));
				m.insert(dat!("name"), dat!(name.clone()));
			}
		}
		Dat::Map(m)
	}

	/// The identity from a daticle. An unreadable one is anonymous rather than an error: a comment
	/// whose author cannot be read is still a comment, and losing the words would be worse.
	pub fn from_dat(d: &Dat) -> Self {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Self::Anon,
		};
		let s = |k: &str| match m.get(&dat!(k)) {
			Some(Dat::Str(v))	=> Some(v.clone()),
			_			=> None,
		};
		match s("kind").as_deref() {
			Some("local")	=> Self::Local {
				name:	s("name").unwrap_or_default(),
				email:	s("email"),
			},
			Some("vouched")	=> Self::Vouched {
				id:	s("id").unwrap_or_default(),
				name:	s("name").unwrap_or_default(),
			},
			_		=> Self::Anon,
		}
	}
}

/// One comment, as the store keeps it.
#[derive(Clone, Debug, Default)]
pub struct Comment {
	/// The comment's own name, unguessable, minted once.
	pub id:		String,
	/// The post it is attached to.
	pub slug:	String,
	/// The comment it replies to, where it replies to one.
	pub parent:	Option<String>,
	/// Who wrote it.
	pub author:	Identity,
	/// What they wrote, as written. The source is kept and never the rendering, for the same reason
	/// a post's is: the renderer improves, and a stored rendering is a photograph of an older one.
	pub body:	String,
	/// When it arrived, as an ISO timestamp.
	pub created:	String,
	/// Where it stands.
	pub state:	CommentState,
	/// Why it stands there, where something decided: the moderator's own words. Shown to the site's
	/// admin in the queue and never to a reader.
	pub reason:	Option<String>,
	/// Whether the site's own admin wrote this, rather than a visitor claiming to be them.
	///
	/// A display name is whatever somebody typed, so it can never distinguish the site's author from
	/// a stranger who typed their name. This can: it is set only by the console, never by anything a
	/// form carries, and it is what the page marks.
	pub by_site_author:	bool,
	/// A salted hash of the address it came from. **Not the address**: enough to recognise a
	/// returning nuisance, not enough to reconstruct who they are, and never shown.
	pub from:	Option<String>,
}

impl Comment {

	/// The comment as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("id"),		dat!(self.id.clone()));
		m.insert(dat!("slug"),		dat!(self.slug.clone()));
		m.insert(dat!("author"),	self.author.to_dat());
		m.insert(dat!("body"),		dat!(self.body.clone()));
		m.insert(dat!("created"),	dat!(self.created.clone()));
		m.insert(dat!("state"),		dat!(self.state.as_str().to_string()));
		if self.by_site_author {
			m.insert(dat!("by_site_author"), Dat::Bool(true));
		}
		// Absent keys rather than empty ones, as everywhere else in this grammar: one way to say
		// nothing is enough.
		if let Some(p) = &self.parent {
			m.insert(dat!("parent"), dat!(p.clone()));
		}
		if let Some(r) = &self.reason {
			m.insert(dat!("reason"), dat!(r.clone()));
		}
		if let Some(f) = &self.from {
			m.insert(dat!("from"), dat!(f.clone()));
		}
		Dat::Map(m)
	}

	/// The comment from a daticle.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Err(err!(
				"publish: a comment record must be a map, not {:?}.", d.kind();
				Invalid, Input, Mismatch)),
		};
		let s = |k: &str| match m.get(&dat!(k)) {
			Some(Dat::Str(v))	=> Some(v.clone()),
			_			=> None,
		};
		let id = match s("id") {
			Some(v) if !v.is_empty()	=> v,
			_				=> return Err(err!(
				"publish: a comment record names no id.";
				Invalid, Input, Missing)),
		};
		Ok(Self {
			id,
			slug:		s("slug").unwrap_or_default(),
			parent:		s("parent"),
			author:		m.get(&dat!("author")).map(Identity::from_dat).unwrap_or(Identity::Anon),
			body:		s("body").unwrap_or_default(),
			created:	s("created").unwrap_or_default(),
			state:		CommentState::of(&s("state").unwrap_or_default()),
			by_site_author:	matches!(m.get(&dat!("by_site_author")), Some(Dat::Bool(true))),
			reason:		s("reason"),
			from:		s("from"),
		})
	}

	/// The comment's prose as HTML, brought within what a site will publish from a stranger.
	///
	/// **The only way a comment should ever reach a page.** The policy is applied to the tree before
	/// rendering, so a `javascript:` destination, a remote image and a borrowed class name are gone
	/// before any HTML exists -- see [`policy`](oxedyne_fe2o3_text::doc::policy) for why that is a
	/// different and better thing than sanitising the output.
	pub fn render(&self) -> Outcome<String> {
		use oxedyne_fe2o3_text::doc::{html, policy};
		let doc = res!(parse_markup(&self.body, Markup::Markdown));
		// `nofollow ugc noopener`: a published comment otherwise lends the site's own standing to
		// whatever it points at, which is the whole economic motive for comment spam.
		let opts = html::Opts { link_rel: Some(fmt!("nofollow ugc noopener")) };
		Ok(html::render_with(&policy::apply(&doc, &policy::Policy::default()), &opts))
	}
}

/// What is remembered about somebody who has commented before.
///
/// The whole of "held once, then trusted": a commenter the site has approved is not made to wait
/// again. Nothing else is kept -- no history of what they said, no count of how often, no address
/// beyond the handle that is already derived from one.
#[derive(Clone, Debug, Default)]
pub struct Commenter {
	/// The handle, from [`Identity::handle`].
	pub handle:	String,
	/// The salted address hash trust was granted to, where trust has been granted.
	///
	/// An address in a form is not proof of anything: anyone may type an approved commenter's
	/// address and inherit their approval. Recording where the approved comment came from, and
	/// requiring a later comment to match it, makes that forgery cost the attacker the same
	/// vantage point as well as the address. Not proof either -- it is one more thing to have.
	pub from:	Option<String>,
	/// Whether an admin has approved something of theirs.
	pub trusted:	bool,
	/// Whether an admin has decided the opposite. A blocked commenter's comments go straight to spam
	/// without troubling anybody.
	pub blocked:	bool,
	/// When they were first seen.
	pub first_seen:	String,
}

impl Commenter {

	/// The record as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("handle"),	dat!(self.handle.clone()));
		if let Some(f) = &self.from {
			m.insert(dat!("from"), dat!(f.clone()));
		}
		m.insert(dat!("trusted"),	Dat::Bool(self.trusted));
		m.insert(dat!("blocked"),	Dat::Bool(self.blocked));
		m.insert(dat!("first_seen"),	dat!(self.first_seen.clone()));
		Dat::Map(m)
	}

	/// The record from a daticle.
	pub fn from_dat(d: &Dat) -> Outcome<Self> {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Err(err!(
				"publish: a commenter record must be a map, not {:?}.", d.kind();
				Invalid, Input, Mismatch)),
		};
		let b = |k: &str| matches!(m.get(&dat!(k)), Some(Dat::Bool(true)));
		let s = |k: &str| match m.get(&dat!(k)) {
			Some(Dat::Str(v))	=> Some(v.clone()),
			_			=> None,
		};
		Ok(Self {
			handle:		s("handle").unwrap_or_default(),
			from:		s("from"),
			trusted:	b("trusted"),
			blocked:	b("blocked"),
			first_seen:	s("first_seen").unwrap_or_default(),
		})
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ VALIDITY                                                                  │
// └───────────────────────────────────────────────────────────────────────────┘

/// Whether a display name is one a person may wear.
///
/// Length, and no control characters -- a name carrying a newline or a zero-width run is a name
/// chosen to do something other than name somebody. The name is escaped wherever it lands, so this is
/// not a safety check; it is a civility one.
pub fn valid_name(s: &str) -> bool {
	let t = s.trim();
	!t.is_empty()
		&& t.len() <= NAME_MAX
		&& !t.chars().any(|c| c.is_control())
}

/// Whether a body is one the store will take.
pub fn valid_body(s: &str) -> bool {
	let t = s.trim();
	!t.is_empty() && t.len() <= BODY_MAX
}

/// Whether a string is a name this module could have minted.
pub fn valid_id(s: &str) -> bool {
	let t = s.trim();
	t.len() == ID_LEN && t.chars().all(|c| ID_ALPHABET.contains(c))
}

/// Mints a comment's name.
pub fn mint_id() -> String {
	Rand::generate_random_string(ID_LEN, ID_ALPHABET)
}

/// The key a comment is stored under: its post, then its own name.
fn key_of(slug: &str, id: &str) -> Dat {
	dat!(fmt!("{}{}/{}", KEY_PREFIX, slug, id))
}

/// The prefix every comment on one post shares.
fn post_prefix(slug: &str) -> String {
	fmt!("{}{}/", KEY_PREFIX, slug)
}

/// The key a commenter is remembered under.
fn author_key(handle: &str) -> Dat {
	dat!(fmt!("{}{}", AUTHOR_PREFIX, handle))
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ PROOF OF WORK                                                             │
// └───────────────────────────────────────────────────────────────────────────┘

/// How many leading zero bits a comment's proof must show.
///
/// The cost is paid by the sender's browser, once, in about a second at this width, and by a spammer
/// once per attempt. It is not a wall -- anyone determined pays it -- it is a tax that makes posting
/// ten thousand comments cost ten thousand seconds instead of nothing. Raise it if that stops being
/// enough; every extra bit doubles the price.
pub const POW_BITS: u32 = 18;

/// What a proof is computed over: the challenge the form was given, and the nonce the browser found.
///
/// The challenge is a one-way function of the post, the site's secret and **the hour it was issued
/// in**, so a proof cannot be computed before the form is fetched, a proof for one post is not a
/// proof for another, and a proof does not last forever. Without the window a single solve served
/// every future comment on that post: the cost was paid once, not once per comment, which is not
/// what a tax is.
pub fn pow_challenge(slug: &str, secret: &[u8]) -> String {
	pow_challenge_at(slug, secret, &pow_window(0))
}

/// How long a challenge stands. An hour: long enough that a reader may write at length and still
/// post, short enough that a solved nonce is not a permanent licence.
pub const POW_WINDOW_SECS: u64 = 3600;

/// The window identifier, `back` windows ago.
///
/// A verifier accepts the current window and the one before it, so a reader who opened the form at
/// 10:59 and posted at 11:01 is not refused for it.
pub fn pow_window(back: u64) -> String {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	fmt!("{}", now.saturating_sub(back * POW_WINDOW_SECS) / POW_WINDOW_SECS)
}

/// The challenge for a named window.
pub fn pow_challenge_at(slug: &str, secret: &[u8], window: &str) -> String {
	let h = HashScheme::new_sha256().hash(&[slug.as_bytes(), b"comment-pow", secret, window.as_bytes()], []);
	hex(&h.as_hashform().as_vec())
}

/// Whether a challenge is one this site issued, in a window still standing.
pub fn pow_challenge_current(challenge: &str, slug: &str, secret: &[u8]) -> bool {
	challenge == pow_challenge_at(slug, secret, &pow_window(0))
		|| challenge == pow_challenge_at(slug, secret, &pow_window(1))
}

/// Whether a nonce solves a challenge to the required width.
pub fn pow_verify(challenge: &str, nonce: &str, bits: u32) -> bool {
	// SHA-256 and not SHA3, because the other side of this is `crypto.subtle.digest` in a browser
	// and WebCrypto offers no SHA3. Getting this wrong does not fail loudly: the proof simply never
	// verifies, the comment is refused before it is stored, and the reader is thanked for it. It was
	// wrong exactly that way once -- see the test, which checks against a digest computed outside
	// this program rather than against another call to the same function.
	let h = HashScheme::new_sha256().hash(&[challenge.as_bytes(), nonce.as_bytes()], []);
	leading_zero_bits(&h.as_hashform().as_vec()) >= bits
}

/// The leading zero bits of a digest.
fn leading_zero_bits(bytes: &[u8]) -> u32 {
	let mut n = 0;
	for b in bytes {
		if *b == 0 {
			n += 8;
			continue;
		}
		n += b.leading_zeros();
		break;
	}
	n
}

/// Hex, as the console renders it.
fn hex(bytes: &[u8]) -> String {
	let mut s = String::with_capacity(bytes.len() * 2);
	for b in bytes {
		s.push_str(&fmt!("{:02x}", b));
	}
	s
}

/// A salted, one-way rendering of a caller's address.
///
/// Stored instead of the address itself. It recognises a returning nuisance and reconstructs nobody:
/// the salt is per-site and never leaves the host, so the value is meaningless anywhere else, and the
/// address space being small enough to enumerate is exactly why the salt has to be there.
pub fn from_hash(addr: &str, salt: &[u8]) -> String {
	let h = HashScheme::new_sha3_256().hash(&[addr.as_bytes(), b"comment-from", salt], []);
	hex(&h.as_hashform().as_vec())[..32].to_string()
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ MODERATION                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// What a moderator decided, and why.
///
/// The seam an AI moderator arrives behind. Three outcomes and no more: a moderator may publish,
/// defer to a person, or bin. It may not delete, and it may not edit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Verdict {
	/// Publish it.
	Allow,
	/// A person should look. The reason is shown to that person, never to the commenter.
	Hold(String),
	/// Bin it, recoverably.
	Spam(String),
}

impl Verdict {

	/// The state this verdict puts a comment in.
	pub fn state(&self) -> CommentState {
		match self {
			Self::Allow	=> CommentState::Approved,
			Self::Hold(_)	=> CommentState::Pending,
			Self::Spam(_)	=> CommentState::Spam,
		}
	}

	/// The reason, where there is one.
	pub fn reason(&self) -> Option<String> {
		match self {
			Self::Allow		=> None,
			Self::Hold(r)		=> Some(r.clone()),
			Self::Spam(r)		=> Some(r.clone()),
		}
	}

	/// The stricter of two verdicts.
	///
	/// How a chain of moderators combines: **the strictest wins, and no later moderator can loosen an
	/// earlier one's refusal.** This is the rule that keeps a model from overturning arithmetic --
	/// asking one about a comment the rules already refused is both a waste of money and a way for a
	/// persuasive comment to talk its way out of a proof it never did.
	pub fn and_then(self, other: Verdict) -> Verdict {
		match (&self, &other) {
			(Self::Spam(_), _)	=> self,
			(_, Self::Spam(_))	=> other,
			(Self::Hold(_), _)	=> self,
			(_, Self::Hold(_))	=> other,
			_			=> Verdict::Allow,
		}
	}
}

/// What decides whether a comment appears.
///
/// An enum rather than a trait object, per the house rules, and the reason it is worth having at all
/// with only one arm filled: the pipeline that calls this is written once, and every later kind of
/// moderator is an arm here rather than a change to the pipeline.
#[derive(Clone, Debug)]
pub enum Moderator {
	/// Arithmetic: what the sender proved, what they wrote, and whether the site already knows them.
	Rules(Rules),
}

impl Default for Moderator {
	fn default() -> Self { Self::Rules(Rules::default()) }
}

impl Moderator {

	/// Judges a comment.
	pub fn judge(&self, c: &Comment, known: Option<&Commenter>) -> Verdict {
		match self {
			Self::Rules(r)	=> r.judge(c, known),
		}
	}
}

/// The arithmetic moderator.
#[derive(Clone, Debug)]
pub struct Rules {
	/// How many links a comment may carry before it is held. A comment is prose with the occasional
	/// reference; a list of links is an advertisement.
	pub link_limit:		usize,
	/// Whether a commenter the site has approved before skips the queue.
	pub trust_returning:	bool,
}

impl Default for Rules {
	fn default() -> Self {
		Self {
			link_limit:		2,
			trust_returning:	true,
		}
	}
}

impl Rules {

	/// Judges a comment by what can be counted.
	///
	/// The order matters and is deliberate: **blocked first** (nothing else about a blocked commenter
	/// is interesting), then the things that are true of the comment whoever sent it, then trust.
	/// Trust is last because it is the only thing that *lets a comment through*, and it should not be
	/// able to carry one past a rule that would otherwise have caught it.
	pub fn judge(&self, c: &Comment, known: Option<&Commenter>) -> Verdict {
		if let Some(k) = known {
			if k.blocked {
				return Verdict::Spam(fmt!("the commenter is blocked"));
			}
		}
		if !valid_body(&c.body) {
			return Verdict::Spam(fmt!("the comment is empty or longer than {} bytes", BODY_MAX));
		}
		let links = count_links(&c.body);
		if links > self.link_limit {
			return Verdict::Hold(fmt!("{} links, more than the {} a comment may carry",
				links, self.link_limit));
		}
		if self.trust_returning {
			if let Some(k) = known {
				if k.trusted {
					return Verdict::Allow;
				}
			}
		}
		// Everybody else waits once. An identity with no handle waits every time, because there is
		// nothing to remember them by -- see `Identity::handle`.
		match c.author.handle() {
			Some(_)	=> Verdict::Hold(fmt!("a first comment from this commenter")),
			None	=> Verdict::Hold(fmt!("no address given, so the commenter cannot be recognised")),
		}
	}
}

/// How many links a run of source carries.
///
/// Counts both the Markdown form and a bare URL, because a spammer writes whichever works. It
/// over-counts a link written both ways in one comment, and over-counting sends a comment to a human
/// rather than to a reader, which is the direction an inexact count should err in.
pub fn count_links(body: &str) -> usize {
	let markdown = body.matches("](").count();
	let bare = body.matches("http://").count() + body.matches("https://").count();
	markdown.max(bare)
}

/// What order comments come back in.
///
/// The seam of row 49. Chronological is how a conversation reads and is what is built; an arm that
/// weighs a comment by something other than when it arrived is the shape of what may come.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Ranker {
	/// Oldest first, which is the order a conversation happened in.
	#[default]
	Chronological,
	/// Newest first.
	Recent,
}

impl Ranker {

	/// Orders a run of comments in place.
	pub fn rank(&self, items: &mut [Comment]) {
		match self {
			Self::Chronological	=> items.sort_by(|a, b| a.created.cmp(&b.created)),
			Self::Recent		=> items.sort_by(|a, b| b.created.cmp(&a.created)),
		}
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ STORE                                                                     │
// └───────────────────────────────────────────────────────────────────────────┘

/// Writes a comment.
pub fn put<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	c:	&Comment,
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	let guard = lock_read!(db_arc);
	res!(guard.insert(key_of(&c.slug, &c.id), c.to_dat(), *user, None));
	Ok(())
}

/// Reads one comment by post and name.
pub fn get<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
	id:	&str,
)
	-> Outcome<Option<Comment>>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	match res!(guard.get(&key_of(slug, id), None)) {
		Some((v, _))	=> Ok(Some(res!(Comment::from_dat(&v)))),
		None		=> Ok(None),
	}
}

/// Every comment on a post, whatever state it is in.
///
/// The scan selects keys and the reads fetch values, which is not an optimisation to undo: scan v1
/// answers `Dat::Empty` for every value whatever `include_values` asks of it, and says so in a log
/// line rather than an error.
pub fn list_for_post<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
	id:	&str,
)
	-> Outcome<Vec<Comment>>
{
	let (db_arc, _) = db;
	let prefix = post_prefix(slug);
	let found = {
		let guard = lock_read!(db_arc);
		let mut opts = ScanOpts::default();
		opts.prefix = Some(dat!(prefix.clone()));
		opts.include_values = false;
		res!(guard.scan(&opts, None))
	};
	let mut out = Vec::new();
	for (k, _, _) in &found {
		let s = match k {
			Dat::Str(s)	=> s,
			_		=> continue,
		};
		let name = match s.strip_prefix(&prefix) {
			Some(n)	=> n,
			None	=> continue,
		};
		// A key the scan offered and the read cannot make sense of costs that comment, not the page.
		match get(db, slug, name) {
			Ok(Some(c))	=> out.push(c),
			Ok(None)	=> {}
			Err(e)		=> debug!("{}: publish: comment '{}/{}' will not read: {}",
						id, slug, name, e),
		}
	}
	Ok(out)
}

/// The comments on a post that a reader may see, in the ranker's order.
pub fn public_for_post<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
	ranker:	Ranker,
	id:	&str,
)
	-> Outcome<Vec<Comment>>
{
	let mut items: Vec<Comment> = res!(list_for_post(db, slug, id))
		.into_iter()
		.filter(|c| c.state.is_public())
		.collect();
	ranker.rank(&mut items);
	Ok(items)
}

/// Every comment awaiting a decision, across every post.
///
/// What the moderation queue reads. A whole-prefix scan, which is the one place this module walks
/// more than one post's worth -- the queue is a page about the site rather than about a post, and
/// there is no cheaper way to ask "what is waiting" than to look.
pub fn queue<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	want:	Option<CommentState>,
	id:	&str,
)
	-> Outcome<Vec<Comment>>
{
	let (db_arc, _) = db;
	let found = {
		let guard = lock_read!(db_arc);
		let mut opts = ScanOpts::default();
		opts.prefix = Some(dat!(KEY_PREFIX));
		opts.include_values = false;
		res!(guard.scan(&opts, None))
	};
	let mut out = Vec::new();
	for (k, _, _) in &found {
		let s = match k {
			Dat::Str(s)	=> s,
			_		=> continue,
		};
		let rest = match s.strip_prefix(KEY_PREFIX) {
			Some(r)	=> r,
			None	=> continue,
		};
		let (slug, name) = match rest.split_once('/') {
			Some(p)	=> p,
			None	=> continue,
		};
		match get(db, slug, name) {
			Ok(Some(c)) => {
				if want.map(|w| c.state == w).unwrap_or(true) {
					out.push(c);
				}
			}
			Ok(None)	=> {}
			Err(e)		=> debug!("{}: publish: comment '{}' will not read: {}", id, rest, e),
		}
	}
	out.sort_by(|a, b| b.created.cmp(&a.created));
	Ok(out)
}

/// How many comments a post has that a reader may see.
pub fn count_public<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
	id:	&str,
)
	-> Outcome<usize>
{
	Ok(res!(list_for_post(db, slug, id)).iter().filter(|c| c.state.is_public()).count())
}

/// Moves a comment to a state, recording why.
///
/// Answers whether there was a comment there to move. Approving a comment **also trusts its author**,
/// where they have a handle: that is the whole of "held once, then trusted", and doing it here rather
/// than at the call site means every path that approves gets it.
pub fn set_state<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
	id:	&str,
	state:	CommentState,
	reason:	Option<String>,
)
	-> Outcome<bool>
{
	let mut c = match res!(get(db, slug, id)) {
		Some(c)	=> c,
		None	=> return Ok(false),
	};
	c.state = state;
	c.reason = reason;
	res!(put(db, &c));

	if state == CommentState::Approved {
		if let Some(h) = c.author.handle() {
			res!(set_trust(db, &h, true, c.from.as_deref(), &c.created));
		}
	}
	Ok(true)
}

/// Deletes a comment outright.
///
/// Distinct from [`CommentState::Removed`], which is a comment taken down and still on file. This is
/// for erasing something that should not be kept at all -- what somebody asking to be forgotten is
/// owed, and what a piece of abuse deserves.
pub fn erase<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
	id:	&str,
)
	-> Outcome<bool>
{
	if res!(get(db, slug, id)).is_none() {
		return Ok(false);
	}
	let (db_arc, user) = db;
	let guard = lock_read!(db_arc);
	res!(guard.delete(&key_of(slug, id), *user, None));
	Ok(true)
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ COMMENTERS                                                                │
// └───────────────────────────────────────────────────────────────────────────┘

/// Reads what is remembered about a commenter.
pub fn commenter<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	handle:	&str,
)
	-> Outcome<Option<Commenter>>
{
	let (db_arc, _) = db;
	let guard = lock_read!(db_arc);
	match res!(guard.get(&author_key(handle), None)) {
		Some((v, _))	=> Ok(Some(res!(Commenter::from_dat(&v)))),
		None		=> Ok(None),
	}
}

/// Sets whether a commenter is trusted, remembering them if they are new.
pub fn set_trust<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	handle:	&str,
	trusted: bool,
	from:	Option<&str>,
	now:	&str,
)
	-> Outcome<()>
{
	let mut rec = res!(commenter(db, handle)).unwrap_or_else(|| Commenter {
		handle:		handle.to_string(),
		from:		None,
		trusted:	false,
		blocked:	false,
		first_seen:	now.to_string(),
	});
	rec.trusted = trusted;
	// Trust is granted to a commenter *as seen*, so a later comment must arrive the same way.
	if trusted {
		rec.from = from.map(|f| f.to_string());
	}
	// Trusting somebody who was blocked unblocks them: the admin's later decision is the operative
	// one, and leaving both flags set would be a record that contradicts itself.
	if trusted {
		rec.blocked = false;
	}
	let (db_arc, user) = db;
	let guard = lock_read!(db_arc);
	res!(guard.insert(author_key(handle), rec.to_dat(), *user, None));
	Ok(())
}

/// Blocks or unblocks a commenter.
pub fn set_blocked<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	handle:	&str,
	blocked: bool,
	now:	&str,
)
	-> Outcome<()>
{
	let mut rec = res!(commenter(db, handle)).unwrap_or_else(|| Commenter {
		handle:		handle.to_string(),
		from:		None,
		trusted:	false,
		blocked:	false,
		first_seen:	now.to_string(),
	});
	rec.blocked = blocked;
	if blocked {
		rec.trusted = false;
	}
	let (db_arc, user) = db;
	let guard = lock_read!(db_arc);
	res!(guard.insert(author_key(handle), rec.to_dat(), *user, None));
	Ok(())
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ THREADING                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// A comment and the replies beneath it.
#[derive(Clone, Debug)]
pub struct Thread {
	/// The comment at this node.
	pub comment:	Comment,
	/// Its replies, in the same order the ranker gave.
	pub replies:	Vec<Thread>,
}

/// Arranges a flat run of comments into threads.
///
/// A reply whose parent is not in the run -- because it was never approved, or was erased -- is
/// **raised to the top level rather than dropped**: the words were addressed to somebody, and losing
/// them because the thing they answered is gone would lose a side of a conversation. It reads a
/// little oddly and says everything that was said, which is the right way round.
pub fn thread(items: Vec<Comment>) -> Vec<Thread> {
	let present: std::collections::HashSet<String> =
		items.iter().map(|c| c.id.clone()).collect();

	// Children by parent, and the roots in the order they arrived.
	let mut children: std::collections::HashMap<String, Vec<Comment>> =
		std::collections::HashMap::new();
	let mut roots: Vec<Comment> = Vec::new();
	for c in items {
		match &c.parent {
			Some(p) if present.contains(p)	=> children.entry(p.clone()).or_default().push(c),
			_				=> roots.push(c),
		}
	}
	// A cycle -- two comments naming each other -- leaves no roots, and every comment in it would
	// simply vanish from the page with nothing said. Anything still held after the roots are taken
	// is raised, on the same reasoning an orphan is: words that were written should be readable.
	if roots.is_empty() && !children.is_empty() {
		let orphaned: Vec<Comment> = children.drain().flat_map(|(_, v)| v).collect();
		return orphaned.into_iter().map(|c| Thread { comment: c, replies: Vec::new() }).collect();
	}
	roots.into_iter().map(|r| build(r, &mut children, 0)).collect()
}

/// One node and everything under it, to the depth the module allows.
fn build(
	c:		Comment,
	children:	&mut std::collections::HashMap<String, Vec<Comment>>,
	depth:		usize,
)
	-> Thread
{
	let kids = children.remove(&c.id).unwrap_or_default();
	// At the floor every descendant, however deep, is flattened onto this node rather than nested
	// further. Draining the whole subtree is the point: handling only the next level or two would
	// silently lose anything below, which is exactly the bug the depth test was written to catch.
	let replies = if depth + 1 >= DEPTH_MAX {
		let mut flat = Vec::new();
		for k in kids {
			flat.extend(drain(k, children));
		}
		flat.into_iter().map(|x| Thread { comment: x, replies: Vec::new() }).collect()
	} else {
		kids.into_iter().map(|k| build(k, children, depth + 1)).collect()
	};
	Thread { comment: c, replies }
}

/// A comment and every descendant it has, in reading order, flat.
///
/// Used at the depth floor. Recursive over the remaining children, so a chain of any length comes out
/// whole: the depth rule bounds how deeply a thread is *drawn*, never how much of it is kept.
fn drain(
	c:		Comment,
	children:	&mut std::collections::HashMap<String, Vec<Comment>>,
)
	-> Vec<Comment>
{
	let kids = children.remove(&c.id).unwrap_or_default();
	let mut out = vec![c];
	for k in kids {
		out.extend(drain(k, children));
	}
	out
}

/// How many top-level threads a page of a conversation shows.
///
/// Counted in threads rather than comments: a reply belongs with what it answers, and splitting a
/// thread across a page boundary would leave an answer on one page and its question on another.
pub const PAGE_THREADS: usize = 25;

/// One page of a threaded conversation, and how many pages there are.
pub fn page_of(threads: Vec<Thread>, page: usize) -> (Vec<Thread>, usize, usize) {
	let pages = threads.len().div_ceil(PAGE_THREADS).max(1);
	let at = page.max(1).min(pages);
	let from = (at - 1) * PAGE_THREADS;
	let upto = (from + PAGE_THREADS).min(threads.len());
	(threads[from..upto].to_vec(), at, pages)
}

/// How many comments a run of threads holds, at every depth.
pub fn count_threads(threads: &[Thread]) -> usize {
	threads.iter().map(|t| 1 + count_threads(&t.replies)).sum()
}


#[cfg(test)]
mod tests {
	use super::*;

	fn c(id: &str, body: &str) -> Comment {
		Comment {
			id:		fmt!("{}", id),
			slug:		fmt!("a-post"),
			body:		fmt!("{}", body),
			created:	fmt!("2026-07-20T10:00:00Z"),
			..Default::default()
		}
	}
	fn named(id: &str, email: Option<&str>) -> Comment {
		let mut x = c(id, "a perfectly ordinary remark");
		x.author = Identity::Local {
			name:	fmt!("Ada"),
			email:	email.map(|e| fmt!("{}", e)),
		};
		x
	}

	/// An unknown state reads as pending, never as approved: a record a later version wrote must not
	/// publish itself by being unreadable.
	#[test]
	fn test_an_unknown_state_is_pending_00() -> Outcome<()> {
		assert_eq!(CommentState::of("approved"), CommentState::Approved);
		assert_eq!(CommentState::of("spam"), CommentState::Spam);
		assert_eq!(CommentState::of("removed"), CommentState::Removed);
		for unknown in ["", "published", "live", "APPROVED", "whatever-comes-next"] {
			assert_eq!(CommentState::of(unknown), CommentState::Pending,
				"'{}' was not read as pending", unknown);
			assert!(!CommentState::of(unknown).is_public(), "'{}' would have been shown", unknown);
		}
		Ok(())
	}

	/// Only an address or a vouched id is a handle. A typed name is not, or a stranger could inherit
	/// somebody else's approval by typing their name.
	#[test]
	fn test_a_name_is_not_an_identity_01() -> Outcome<()> {
		let anon = Identity::Anon;
		assert_eq!(anon.handle(), None);

		let just_a_name = Identity::Local { name: fmt!("Ada"), email: None };
		assert_eq!(just_a_name.handle(), None, "a bare name was taken for an identity");

		let with_email = Identity::Local { name: fmt!("Ada"), email: Some(fmt!("  Ada@Example.COM ")) };
		assert_eq!(with_email.handle(), Some(fmt!("email:ada@example.com")),
			"an address was not normalised into a stable handle");

		let blank = Identity::Local { name: fmt!("Ada"), email: Some(fmt!("   ")) };
		assert_eq!(blank.handle(), None, "an empty address became a handle");

		let vouched = Identity::Vouched { id: fmt!("abc"), name: fmt!("Ada") };
		assert_eq!(vouched.handle(), Some(fmt!("vouched:abc")));
		Ok(())
	}

	/// A commenter's address never reaches what a reader sees.
	#[test]
	fn test_an_address_is_never_displayed_02() -> Outcome<()> {
		let x = named("a", Some("ada@example.com"));
		assert_eq!(x.author.display_name(), "Ada");
		assert!(!x.author.display_name().contains('@'), "the display name carries an address");
		// It round-trips through storage, because a reply notification needs it.
		let back = res!(Comment::from_dat(&x.to_dat()));
		assert_eq!(back.author.email(), Some("ada@example.com"));
		Ok(())
	}

	/// A comment round-trips through the store's daticle form.
	#[test]
	fn test_a_comment_round_trips_03() -> Outcome<()> {
		let mut x = named("abc", Some("ada@example.com"));
		x.parent = Some(fmt!("parent-id"));
		x.state = CommentState::Approved;
		x.reason = Some(fmt!("because"));
		x.from = Some(fmt!("deadbeef"));
		let back = res!(Comment::from_dat(&x.to_dat()));
		assert_eq!(back.id, x.id);
		assert_eq!(back.slug, x.slug);
		assert_eq!(back.parent, x.parent);
		assert_eq!(back.body, x.body);
		assert_eq!(back.state, x.state);
		assert_eq!(back.reason, x.reason);
		assert_eq!(back.from, x.from);
		assert_eq!(back.author, x.author);
		Ok(())
	}

	/// A record with no id is refused rather than stored under a name it does not have.
	#[test]
	fn test_a_record_without_an_id_is_refused_04() -> Outcome<()> {
		let mut m = DaticleMap::new();
		m.insert(dat!("slug"), dat!("a-post".to_string()));
		assert!(Comment::from_dat(&Dat::Map(m)).is_err());
		assert!(Comment::from_dat(&dat!("not a map".to_string())).is_err());
		Ok(())
	}

	/// The first comment waits; the same person, once approved, does not wait again.
	#[test]
	fn test_held_once_then_trusted_05() -> Outcome<()> {
		let m = Moderator::default();
		let x = named("a", Some("ada@example.com"));

		// Nobody knows them yet.
		assert!(matches!(m.judge(&x, None), Verdict::Hold(_)), "a first comment was not held");

		// Approved once.
		let known = Commenter {
			handle: fmt!("email:ada@example.com"), from: None, trusted: true, blocked: false,
			first_seen: fmt!("2026-07-01T00:00:00Z"),
		};
		assert_eq!(m.judge(&x, Some(&known)), Verdict::Allow, "a trusted commenter was still held");

		// Blocked beats everything.
		let blocked = Commenter { trusted: true, blocked: true, ..known.clone() };
		assert!(matches!(m.judge(&x, Some(&blocked)), Verdict::Spam(_)),
			"a blocked commenter got through");
		Ok(())
	}

	/// Somebody who gives no address is held every time, because there is nothing to remember.
	#[test]
	fn test_an_anonymous_commenter_is_always_held_06() -> Outcome<()> {
		let m = Moderator::default();
		let x = named("a", None);
		match m.judge(&x, None) {
			Verdict::Hold(r)	=> assert!(r.contains("recognised"), "unhelpful reason: {}", r),
			other			=> panic!("an anonymous comment was not held: {:?}", other),
		}
		Ok(())
	}

	/// Trust does not carry a comment past a rule that would otherwise catch it.
	#[test]
	fn test_trust_does_not_overrule_the_counting_07() -> Outcome<()> {
		let m = Moderator::default();
		let known = Commenter {
			handle: fmt!("email:ada@example.com"), from: None, trusted: true, blocked: false,
			first_seen: fmt!("2026-07-01T00:00:00Z"),
		};
		let mut spammy = named("a", Some("ada@example.com"));
		spammy.body = fmt!("buy https://a.example buy https://b.example buy https://c.example");
		assert!(matches!(m.judge(&spammy, Some(&known)), Verdict::Hold(_)),
			"a trusted commenter's link-stuffed comment went straight through");

		// And an empty one is refused whoever sent it.
		let mut empty = named("b", Some("ada@example.com"));
		empty.body = fmt!("   ");
		assert!(matches!(m.judge(&empty, Some(&known)), Verdict::Spam(_)));
		Ok(())
	}

	/// Links are counted in both the forms a spammer writes them in.
	#[test]
	fn test_links_are_counted_either_way_08() -> Outcome<()> {
		assert_eq!(count_links("no links here"), 0);
		assert_eq!(count_links("one [a](https://x.example) link"), 1);
		assert_eq!(count_links("bare https://x.example and https://y.example"), 2);
		assert_eq!(count_links("[a](x) [b](y) [c](z)"), 3);
		Ok(())
	}

	/// The strictest verdict wins, and nothing later can loosen an earlier refusal.
	#[test]
	fn test_the_strictest_verdict_wins_09() -> Outcome<()> {
		let allow = Verdict::Allow;
		let hold = Verdict::Hold(fmt!("h"));
		let spam = Verdict::Spam(fmt!("s"));

		assert_eq!(allow.clone().and_then(allow.clone()), Verdict::Allow);
		assert!(matches!(allow.clone().and_then(hold.clone()), Verdict::Hold(_)));
		assert!(matches!(hold.clone().and_then(allow.clone()), Verdict::Hold(_)),
			"a later Allow overturned a Hold");
		assert!(matches!(spam.clone().and_then(allow.clone()), Verdict::Spam(_)),
			"a later Allow overturned a Spam");
		assert!(matches!(hold.clone().and_then(spam.clone()), Verdict::Spam(_)));
		assert!(matches!(spam.clone().and_then(hold.clone()), Verdict::Spam(_)));
		Ok(())
	}

	/// A verdict maps to the state it should, and carries its reason.
	#[test]
	fn test_a_verdict_names_a_state_10() -> Outcome<()> {
		assert_eq!(Verdict::Allow.state(), CommentState::Approved);
		assert_eq!(Verdict::Hold(fmt!("r")).state(), CommentState::Pending);
		assert_eq!(Verdict::Spam(fmt!("r")).state(), CommentState::Spam);
		assert_eq!(Verdict::Allow.reason(), None);
		assert_eq!(Verdict::Hold(fmt!("r")).reason(), Some(fmt!("r")));
		Ok(())
	}

	/// The proof verifies a digest computed **outside this program**.
	///
	/// The value below came from python's hashlib, not from another call to `pow_verify`. That
	/// matters: the first version of this hashed SHA3-256 on the server while the browser hashed
	/// SHA-256, so no proof ever verified, every comment from a reader with scripting was refused
	/// before it was stored, and each of those readers was thanked for it. The test that was supposed
	/// to catch it checked SHA3 against SHA3 and passed happily. An oracle from outside is the only
	/// kind that could have failed.
	#[test]
	fn test_the_proof_agrees_with_an_outside_digest_20() -> Outcome<()> {
		// sha256("0123456789abcdef"*4 + "4494") begins 0x0009..., which is 12 leading zero bits.
		let challenge = "0123456789abcdef".repeat(4);
		assert!(pow_verify(&challenge, "4494", 12),
			"a proof computed by hashlib did not verify: the server is not using SHA-256");
		assert!(!pow_verify(&challenge, "4494", 20),
			"a 12-bit proof passed at 20 bits");
		assert!(!pow_verify(&challenge, "4493", 12), "a wrong nonce passed");
		Ok(())
	}

	/// A challenge stands for its window and not for ever.
	#[test]
	fn test_a_proof_expires_21() -> Outcome<()> {
		let secret = b"a-site-secret";
		let now = pow_challenge_at("a-post", secret, &pow_window(0));
		let prev = pow_challenge_at("a-post", secret, &pow_window(1));
		let ancient = pow_challenge_at("a-post", secret, "1");

		assert_ne!(now, prev, "two windows share a challenge, so a solve never expires");
		assert!(pow_challenge_current(&now, "a-post", secret));
		// The window before is accepted, so a form opened at 10:59 still posts at 11:01.
		assert!(pow_challenge_current(&prev, "a-post", secret));
		assert!(!pow_challenge_current(&ancient, "a-post", secret), "an old proof still stands");
		assert!(!pow_challenge_current(&now, "another-post", secret), "a proof travelled between posts");
		Ok(())
	}

	/// A parent is an id this module minted, or it is nothing at all.
	#[test]
	fn test_a_parent_is_an_id_22() -> Outcome<()> {
		assert!(valid_id(&mint_id()));
		assert!(!valid_id(""));
		assert!(!valid_id("short"));
		assert!(!valid_id(&"a".repeat(ID_LEN + 1)));
		assert!(!valid_id(&"A".repeat(ID_LEN)), "an off-alphabet id was accepted");
		assert!(!valid_id(&"!".repeat(ID_LEN)));
		// The field that had no bound at all: megabytes of anything.
		assert!(!valid_id(&"a".repeat(8_000_000)), "an unbounded parent was accepted");
		Ok(())
	}

	/// Trust is honoured only where the sender matches the one it was granted to.
	#[test]
	fn test_trust_does_not_travel_23() -> Outcome<()> {
		let m = Moderator::default();
		let mut x = named("a", Some("ada@example.com"));
		x.from = Some(fmt!("the-place-ada-comments-from"));

		let granted = Commenter {
			handle:		fmt!("email:ada@example.com"),
			from:		Some(fmt!("the-place-ada-comments-from")),
			trusted:	true,
			blocked:	false,
			first_seen:	fmt!("2026-07-01T00:00:00Z"),
		};
		// Ada, from where Ada comments: allowed.
		assert_eq!(m.judge(&x, Some(&granted)), Verdict::Allow);

		// Somebody else typing Ada's address, from elsewhere: the trust must not follow the address.
		// (The filtering happens in `receive`; this asserts the record carries what that needs.)
		assert_eq!(granted.from.as_deref(), Some("the-place-ada-comments-from"));
		let forged_from = Some(fmt!("somewhere-else"));
		assert_ne!(granted.from, forged_from, "trust would have been inherited by an address alone");
		Ok(())
	}

	/// The rate limit switches off cleanly, and a site that says nothing gets the defaults.
	#[test]
	fn test_the_rate_limit_is_policy_26() -> Outcome<()> {
		use crate::srv::publish::PublishConfig;

		// A site that says nothing is rate limited, because most sites should be.
		let cfg = res!(PublishConfig::from_datmap(&DaticleMap::new()));
		assert_eq!(cfg.comment_rate_secs, 30);
		assert_eq!(cfg.comment_rate_hourly, 10);

		// A site behind a shared address turns it off, and is taken at its word.
		// Written as a bare zero, which the grammar types as narrowly as it can -- the shape an
		// operator actually writes, and the one a match on `Dat::U64` alone would refuse.
		let mut m = DaticleMap::new();
		m.insert(dat!("comment_rate_secs"), dat!(0u8));
		m.insert(dat!("comment_rate_hourly"), dat!(0u8));
		let cfg = res!(PublishConfig::from_datmap(&m));
		assert_eq!(cfg.comment_rate_secs, 0);
		assert_eq!(cfg.comment_rate_hourly, 0);

		// And a value that is not a count is refused rather than guessed at.
		let mut m = DaticleMap::new();
		m.insert(dat!("comment_rate_secs"), dat!("often".to_string()));
		assert!(PublishConfig::from_datmap(&m).is_err());
		Ok(())
	}

	/// A submission cannot claim to be the site's author, whatever it sends.
	#[test]
	fn test_a_submission_cannot_claim_authorship_25() -> Outcome<()> {
		// The struct's own default is false, and `receive` sets it explicitly rather than taking it
		// from anything a form carried -- there is no field on `Submission` that could reach it.
		let x = Comment::default();
		assert!(!x.by_site_author);

		// It survives a round trip when the console does set it.
		let mut y = named("a", Some("me@example.com"));
		y.by_site_author = true;
		assert!(res!(Comment::from_dat(&y.to_dat())).by_site_author);

		// And a record that says nothing about it is not the author's.
		let mut m = DaticleMap::new();
		m.insert(dat!("id"), dat!("abcdefghijklmnop".to_string()));
		assert!(!res!(Comment::from_dat(&Dat::Map(m))).by_site_author);
		Ok(())
	}

	/// An edit token names one comment and cannot be guessed from another.
	#[test]
	fn test_an_edit_token_names_one_comment_28() -> Outcome<()> {
		let secret = b"a-site-secret";
		let a = edit_token("aaaaaaaaaaaaaaaa", secret);
		let b = edit_token("bbbbbbbbbbbbbbbb", secret);
		assert_ne!(a, b, "two comments share an edit token");
		assert!(edit_token_ok("aaaaaaaaaaaaaaaa", secret, &a));
		assert!(!edit_token_ok("aaaaaaaaaaaaaaaa", secret, &b), "another comment's token was taken");
		assert!(!edit_token_ok("aaaaaaaaaaaaaaaa", secret, ""), "an empty token was taken");
		assert!(!edit_token_ok("aaaaaaaaaaaaaaaa", b"another-secret", &a),
			"a token from another site was taken");
		Ok(())
	}

	/// The window closes, and an unreadable stamp is not editable.
	#[test]
	fn test_the_edit_window_closes_29() -> Outcome<()> {
		let mut c = c("aaaaaaaaaaaaaaaa", "words");
		c.created = fmt!("2026-07-20T10:00:00Z");
		let at = match parse_stamp_secs(&c.created) {
			Some(t)	=> t,
			None	=> panic!("the test's own stamp will not parse"),
		};
		assert!(editable(&c, at), "a comment was not editable the moment it was written");
		assert!(editable(&c, at + EDIT_WINDOW_SECS - 1), "the window closed early");
		assert!(!editable(&c, at + EDIT_WINDOW_SECS), "the window did not close");
		assert!(!editable(&c, at + 86_400), "a day-old comment was still editable");

		// A stamp that will not read is not editable: an unreadable time cannot be shown to be
		// recent, and guessing permissively would make the window unbounded.
		for bad in ["not a time at all", "", "2026-07-20", "20260720T100000Z", "yyyy-mm-ddThh:mm:ss"] {
			c.created = fmt!("{}", bad);
			assert!(!editable(&c, at), "'{}' granted an unbounded window", bad);
		}
		Ok(())
	}

	/// The stamp reader agrees with known instants, and refuses what is not one.
	#[test]
	fn test_the_stamp_reader_is_strict_30() -> Outcome<()> {
		// Values from `date -u -d "<stamp>" +%s`, not from this function and not from memory. The
		// first draft of this test had one of them three days out, written from recall; the code
		// was right and the expectation was wrong.
		assert_eq!(parse_stamp_secs("1970-01-01T00:00:00Z"), Some(0));
		assert_eq!(parse_stamp_secs("2000-01-01T00:00:00Z"), Some(946_684_800));
		assert_eq!(parse_stamp_secs("2026-07-20T03:00:00Z"), Some(1_784_516_400));
		// A leap day, which a naive day count gets wrong.
		assert_eq!(parse_stamp_secs("2024-02-29T00:00:00Z"), Some(1_709_164_800));
		// And the shapes that are not a stamp.
		for bad in ["", "not a time", "2026-07-20", "2026-13-01T00:00:00Z",
			"2026-07-32T00:00:00Z", "2026-07-20T25:00:00Z", "2026-07-20T00:60:00Z",
			"20xx-07-20T00:00:00Z"] {
			assert_eq!(parse_stamp_secs(bad), None, "'{}' was read as a time", bad);
		}
		Ok(())
	}

	/// A comment claiming a known commenter from a new place is held and says so.
	#[test]
	fn test_a_claim_from_elsewhere_is_flagged_27() -> Outcome<()> {
		// The record `receive` consults, and what it would compare against.
		let granted = Commenter {
			handle:		fmt!("email:ada@example.com"),
			from:		Some(fmt!("where-ada-comments-from")),
			trusted:	true,
			blocked:	false,
			first_seen:	fmt!("2026-07-01T00:00:00Z"),
		};
		// Somebody typing Ada's address from somewhere else: the mismatch is detectable, which is
		// what `receive` turns into a held comment carrying a reason.
		let forged = Some(fmt!("somewhere-ada-has-never-been"));
		let is_mismatch = granted.trusted
			&& granted.from.is_some()
			&& granted.from != forged;
		assert!(is_mismatch, "an impersonation attempt would not have been noticed");

		// And Ada herself, from her usual place, is not flagged.
		let hers = granted.from.clone();
		assert!(!(granted.trusted && granted.from.is_some() && granted.from != hers),
			"a regular commenter was flagged as an impostor");
		Ok(())
	}

	/// A cycle does not swallow the comments in it.
	#[test]
	fn test_a_cycle_loses_nothing_24() -> Outcome<()> {
		let mut a = c("aaaaaaaaaaaaaaaa", "one");
		let mut b = c("bbbbbbbbbbbbbbbb", "two");
		a.parent = Some(b.id.clone());
		b.parent = Some(a.id.clone());
		let threads = thread(vec![a, b]);
		assert_eq!(count_threads(&threads), 2, "a cycle swallowed its comments");
		Ok(())
	}

	/// The proof is over the challenge, so a proof for one post is not a proof for another, and a
	/// wrong nonce does not pass.
	#[test]
	fn test_a_proof_is_bound_to_its_challenge_11() -> Outcome<()> {
		let secret = b"a-per-process-secret";
		let a = pow_challenge_at("post-one", secret, "w");
		let b = pow_challenge_at("post-two", secret, "w");
		assert_ne!(a, b, "two posts share a challenge");
		assert_eq!(a, pow_challenge_at("post-one", secret, "w"), "a challenge is not stable");

		// Find a real proof at a width cheap enough for a test, then check it does not travel.
		let bits = 8;
		let mut nonce = 0u64;
		let solved = loop {
			if pow_verify(&a, &fmt!("{}", nonce), bits) { break fmt!("{}", nonce); }
			nonce += 1;
			assert!(nonce < 1_000_000, "no proof found at {} bits", bits);
		};
		assert!(pow_verify(&a, &solved, bits));
		assert!(!pow_verify(&b, &solved, bits), "a proof for one post solved another");
		assert!(!pow_verify(&a, "0", bits + 24), "a proof passed at a width it cannot have met");
		Ok(())
	}

	/// The address a comment came from is stored one-way and salted.
	#[test]
	fn test_an_address_is_stored_one_way_12() -> Outcome<()> {
		let h = from_hash("203.0.113.7", b"site-salt");
		assert!(!h.contains("203"), "the address survived in its own hash: {}", h);
		assert_eq!(h, from_hash("203.0.113.7", b"site-salt"), "the hash is not stable");
		assert_ne!(h, from_hash("203.0.113.8", b"site-salt"), "two addresses collided");
		assert_ne!(h, from_hash("203.0.113.7", b"other-salt"),
			"the salt does not change the hash, so it is portable between sites");
		Ok(())
	}

	/// Replies nest under what they answer, and the order within a level is the ranker's.
	#[test]
	fn test_replies_nest_13() -> Outcome<()> {
		let mut a = c("a", "root one");
		a.created = fmt!("2026-07-20T10:00:00Z");
		let mut b = c("b", "reply to a");
		b.parent = Some(fmt!("a"));
		b.created = fmt!("2026-07-20T10:01:00Z");
		let mut d = c("d", "root two");
		d.created = fmt!("2026-07-20T10:02:00Z");

		let threads = thread(vec![a, b, d]);
		assert_eq!(threads.len(), 2, "replies did not nest: {:?}", threads.len());
		assert_eq!(threads[0].comment.id, "a");
		assert_eq!(threads[0].replies.len(), 1);
		assert_eq!(threads[0].replies[0].comment.id, "b");
		assert_eq!(threads[1].comment.id, "d");
		assert_eq!(count_threads(&threads), 3);
		Ok(())
	}

	/// A reply whose parent is gone is raised rather than dropped: one side of a conversation is
	/// still worth reading.
	#[test]
	fn test_an_orphan_is_raised_not_dropped_14() -> Outcome<()> {
		let mut orphan = c("b", "answering something that was removed");
		orphan.parent = Some(fmt!("a-comment-that-is-not-here"));
		let threads = thread(vec![orphan]);
		assert_eq!(threads.len(), 1, "an orphaned reply was dropped");
		assert_eq!(threads[0].comment.id, "b");
		assert_eq!(count_threads(&threads), 1);
		Ok(())
	}

	/// Nothing is lost to the depth rule: a reply below the floor is kept, flattened.
	#[test]
	fn test_depth_loses_nothing_15() -> Outcome<()> {
		let mut items = vec![c("c0", "root")];
		for i in 1..8 {
			let mut x = c(&fmt!("c{}", i), "deeper");
			x.parent = Some(fmt!("c{}", i - 1));
			x.created = fmt!("2026-07-20T10:0{}:00Z", i);
			items.push(x);
		}
		let threads = thread(items);
		assert_eq!(count_threads(&threads), 8, "the depth rule lost comments");
		Ok(())
	}

	/// The ranker orders a level, both ways.
	#[test]
	fn test_the_ranker_orders_16() -> Outcome<()> {
		let mut a = c("a", "first"); a.created = fmt!("2026-07-20T10:00:00Z");
		let mut b = c("b", "second"); b.created = fmt!("2026-07-20T11:00:00Z");
		let mut items = vec![b.clone(), a.clone()];

		Ranker::Chronological.rank(&mut items);
		assert_eq!(items[0].id, "a", "chronological did not put the oldest first");

		Ranker::Recent.rank(&mut items);
		assert_eq!(items[0].id, "b", "recent did not put the newest first");
		Ok(())
	}

	/// A name is words, not control characters, and a body has bounds.
	#[test]
	fn test_what_is_accepted_17() -> Outcome<()> {
		assert!(valid_name("Ada"));
		assert!(valid_name("  Ada Lovelace  "));
		assert!(!valid_name(""));
		assert!(!valid_name("   "));
		assert!(!valid_name("Ada\nLovelace"), "a newline in a name was accepted");
		assert!(!valid_name("Ada\u{0}"), "a NUL in a name was accepted");
		assert!(!valid_name(&"a".repeat(NAME_MAX + 1)));

		assert!(valid_body("a remark"));
		assert!(!valid_body(""));
		assert!(!valid_body("   "));
		assert!(!valid_body(&"a".repeat(BODY_MAX + 1)));
		Ok(())
	}

	/// A comment's prose reaches HTML through the policy, so a stranger's script does not run.
	#[test]
	fn test_a_comment_renders_within_the_policy_18() -> Outcome<()> {
		let mut x = c("a", "");
		x.body = fmt!("Nice post. [click me](javascript:alert(1)) and <script>steal()</script>\n\n\
			![tracker](https://tracker.example/p.gif)");
		let html = res!(x.render());
		assert!(!html.contains("javascript:"), "a script destination reached the page: {}", html);
		assert!(!html.contains("<script>"), "a script tag reached the page: {}", html);
		assert!(!html.contains("<img"), "a remote image reached the page: {}", html);
		assert!(!html.contains("tracker.example"), "a tracker's address reached the page: {}", html);
		// And the words survive.
		assert!(html.contains("Nice post."), "the prose was lost: {}", html);
		assert!(html.contains("click me"), "the link's words were lost: {}", html);

		// A link a stranger did get to keep carries rel, so the site lends it nothing.
		let mut y = c("b", "");
		y.body = fmt!("See [this](https://example.com/x).");
		let html = res!(y.render());
		assert!(html.contains("rel=\"nofollow ugc noopener\""),
			"a commenter's link carried no rel: {}", html);
		Ok(())
	}

	/// Ids are unguessable and do not repeat.
	#[test]
	fn test_ids_are_minted_19() -> Outcome<()> {
		let mut seen = std::collections::HashSet::new();
		for _ in 0..256 {
			let id = mint_id();
			assert_eq!(id.len(), ID_LEN);
			assert!(id.chars().all(|c| ID_ALPHABET.contains(c)), "'{}' is off the alphabet", id);
			assert!(seen.insert(id), "a minted id repeated within 256");
		}
		Ok(())
	}
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ RECEIVING                                                                 │
// └───────────────────────────────────────────────────────────────────────────┘

/// What a submitted comment turned into.
///
/// A caller answers a reader with the same page either way; this says what to tell them, as a **code**
/// that the page turns into words -- never as text that travels in a URL. It never says *which rule*
/// refused, deliberately: a spammer tuning against a precise reason is being given a test suite, and a
/// reader who wrote something ordinary does not need to know the machinery.
pub enum Received {
	/// Stored and published at once, because the site already knows the commenter.
	Published,
	/// Stored and waiting for the author to see it.
	Held,
	/// Not stored. The reader is told the same thing as `Held` -- see below.
	Refused(String),
}

impl Received {

	/// What a reader is told.
	///
	/// **A refusal reads like a hold.** Anything else is an oracle: a machine that is told "your proof
	/// was wrong" retries with a better proof, and one told "held for review" learns nothing about
	/// whether it worked. The cost is that a genuine reader whose comment was binned is told it is
	/// waiting; the alternative is a tuning signal for everybody, which is worse.
	pub fn tell_reader(&self) -> &'static str {
		match self {
			Self::Published	=> "published",
			Self::Held
			| Self::Refused(_)
				=> "held",
		}
	}
}

/// Everything a submitted comment arrives with.
pub struct Submission<'a> {
	/// The post being commented on.
	pub slug:	&'a str,
	/// The comment being replied to, where one is.
	pub parent:	Option<String>,
	/// The name given.
	pub name:	String,
	/// The address given, where one was.
	pub email:	Option<String>,
	/// The prose.
	pub body:	String,
	/// The honeypot field. Anything in it and the sender is not a person.
	pub honeypot:	String,
	/// The challenge the form carried.
	pub challenge:	String,
	/// The nonce the browser found, where it found one.
	pub nonce:	String,
	/// Who sent it, for the salted hash.
	pub from:	Option<String>,
	/// When it arrived.
	pub now:	String,
}

/// Renders a comment's prose as the reader would see it, storing nothing.
///
/// A preview is a rendering service offered to anybody, which is why it is bounded rather than
/// merely offered: the body is capped as a comment's is, and a sender is held to the same interval
/// as a comment. It counts against **its own** budget, not the comment budget -- previewing twice
/// then posting should not find the post refused, which is what sharing one counter would do.
pub fn preview<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:		&(Arc<RwLock<DB>>, UID),
	body:		&str,
	from:		Option<&str>,
	salt:		&[u8],
	interval:	u64,
)
	-> Outcome<Option<String>>
{
	if !valid_body(body) {
		return Ok(None);
	}
	if let Some(addr) = from {
		let key = fmt!("preview:{}", from_hash(addr, salt));
		if !res!(rate_allows(db, &key, interval, 0)) {
			return Ok(None);
		}
	}
	let c = Comment { body: body.to_string(), ..Default::default() };
	Ok(Some(res!(c.render())))
}

/// Takes a submitted comment: checks it, judges it, stores it.
///
/// The order is the point. **What can be decided without touching the database is decided first** --
/// the honeypot and the shape of the fields cost nothing and refuse most of what arrives. The proof
/// is checked next, and is *not* a condition of being heard: a browser with no scripting sends no
/// nonce, and that comment is held for a person rather than refused, because a reader without
/// JavaScript is still a reader. Only then does anything read from or write to the store.
pub async fn receive<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:		&(Arc<RwLock<DB>>, UID),
	moderator:	&Moderator,
	// The site's AI, where it has one, and the connection to reach it. Consulted on a comment the
	// rules would make wait, and never on one they already refused -- asking a model about a comment
	// the arithmetic binned is both a waste and a way for a persuasive comment to talk its way out.
	ai_settings:	Option<&ai::AiSettings>,
	tls:		&Option<Arc<ClientConfig>>,
	rate:		(u64, u32),
	sub:		Submission<'_>,
	salt:		&[u8],
	secret:		&[u8],
	id:		&str,
)
	-> Outcome<(Received, Option<String>)>
{
	// The honeypot. A field no person can see, so anything in it was put there by something filling
	// every field it found. Nothing is stored and nothing is logged beyond the count.
	if !sub.honeypot.trim().is_empty() {
		info!("{}: publish: a comment on '{}' filled the honeypot", id, sub.slug);
		return Ok((Received::Refused(fmt!("honeypot")), None));
	}

	let name = sub.name.trim().to_string();
	let body = sub.body.trim().to_string();
	if !valid_body(&body) {
		return Ok((Received::Refused(fmt!("the comment is empty or too long")), None));
	}
	// A name is optional in the sense that a reader may leave it blank and be anonymous; a name that
	// is *given* must be a name.
	if !name.is_empty() && !valid_name(&name) {
		return Ok((Received::Refused(fmt!("that is not a name")), None));
	}

	// The proof, where one was sent. A wrong proof is a refusal -- it was attempted and failed, which
	// a browser does not do by accident. No proof at all is not: see the note above.
	let proved = if sub.nonce.trim().is_empty() {
		false
	} else {
		if !pow_challenge_current(&sub.challenge, sub.slug, secret) {
			return Ok((Received::Refused(fmt!("the proof answers a challenge this site did not set")), None));
		}
		if !pow_verify(&sub.challenge, sub.nonce.trim(), POW_BITS) {
			return Ok((Received::Refused(fmt!("the proof does not meet the width")), None));
		}
		true
	};

	let email = sub.email
		.map(|e| e.trim().to_lowercase())
		.filter(|e| !e.is_empty());
	// An address that is given must look like one; one that is not given is fine. It is never shown,
	// so a malformed address is only ever a reply that will not arrive -- worth refusing at the door
	// rather than storing something useless.
	if let Some(e) = &email {
		if !crate::srv::publish::subscribe::valid_email(e) {
			return Ok((Received::Refused(fmt!("that is not an address")), None));
		}
	}

	let author = if name.is_empty() && email.is_none() {
		Identity::Anon
	} else {
		Identity::Local {
			name:	if name.is_empty() { fmt!("Anonymous") } else { name },
			email:	email,
		}
	};

	let c = Comment {
		id:		mint_id(),
		slug:		sub.slug.to_string(),
		// A parent is an id this module minted or it is nothing. Unchecked, it was the one field
		// with no bound at all: a caller could store megabytes of their own choosing per request,
		// since the field went straight into the record.
		parent:		sub.parent.filter(|p| valid_id(p)),
		author,
		body,
		created:	sub.now.clone(),
		state:		CommentState::Pending,
		reason:		None,
		// Never from the form. A submission cannot claim this, whatever it sends.
		by_site_author:	false,
		from:		sub.from.as_deref().map(|a| from_hash(a, salt)),
	};

	// What is already waiting on this post. Read before anything is written, so a full queue costs a
	// read rather than a row.
	// What the sender is allowed, before the post's own ceilings are read. A refusal here costs one
	// read and writes nothing, which is the point of putting it first.
	if let Some(addr) = &sub.from {
		let hashed = from_hash(addr, salt);
		if !res!(rate_allows(db, &hashed, rate.0, rate.1)) {
			info!("{}: publish: a sender is commenting faster than this site allows", id);
			return Ok((Received::Refused(fmt!("too many comments from one sender")), None));
		}
	}

	let held = res!(list_for_post(db, sub.slug, id));
	// A ceiling on the whole thread, not only on what is waiting. Every comment on a post is read
	// back on every public view of it, so an unbounded store is an unbounded cost on every reader --
	// the write is cheap and permanent and the reading of it is neither.
	if held.len() >= POST_MAX {
		info!("{}: publish: '{}' holds {} comments and is taking no more", id, sub.slug, held.len());
		return Ok((Received::Refused(fmt!("the post has all the comments it will take")), None));
	}
	let waiting = held.iter().filter(|x| x.state == CommentState::Pending).count();
	if waiting >= PENDING_MAX {
		info!("{}: publish: '{}' has {} comments waiting and is taking no more",
			id, sub.slug, waiting);
		return Ok((Received::Refused(fmt!("the post's queue is full")), None));
	}

	// What the site already knows about this commenter, where there is anything to know.
	let known = match c.author.handle() {
		Some(h)	=> res!(commenter(db, &h)),
		None	=> None,
	};

	// Trust attaches to a handle, and a handle is an address somebody typed: nothing has proved they
	// own it. So it is only honoured where the sender also matches the one the trust was granted to.
	// An attacker who knows an approved address still has to arrive from the same place. This is a
	// weaker claim than a confirmed address would be, and it is stated rather than hidden: see
	// `Commenter::from`.
	let mismatch = known.as_ref().map(|k: &Commenter| {
		k.trusted && k.from.is_some() && k.from.as_deref() != c.from.as_deref()
	}).unwrap_or(false);
	let known = known.filter(|k| {
		!k.trusted || k.from.is_none() || k.from.as_deref() == c.from.as_deref()
	});

	let mut verdict = moderator.judge(&c, known.as_ref());

	// The model refines an ordinary hold, and only that. A comment the rules would publish (a trusted
	// commenter) or bin (a blocked one, a shape that fails) is already decided; the model is asked
	// only about the comment that would otherwise sit in the queue -- a stranger's first say -- and it
	// may publish it, bin it, or leave it waiting. Its decision replaces the rules' hold, but the
	// security holds below are applied *after* and strictest-wins, so the model can never carry a
	// comment past a missing proof of work or a trusted address arriving from a new place. A model
	// that is not configured, or will not answer, changes nothing: the comment simply waits, which is
	// what it would have done without any AI at all.
	if let Verdict::Hold(_) = &verdict {
		if let (Some(settings), Some(tls_arc)) = (ai_settings, tls.as_ref()) {
			if settings.ready() {
				match ai::judge_comment(settings, tls_arc.clone(), &c.body).await {
					Some(ai::CommentVerdict::Approve)	=> verdict = Verdict::Allow,
					Some(ai::CommentVerdict::Spam)		=>
						verdict = Verdict::Spam(fmt!("judged spam by the site's model")),
					Some(ai::CommentVerdict::Hold)		=>
						verdict = Verdict::Hold(fmt!("held for a person by the site's model")),
					// The model could not be reached; the comment keeps the hold it already had.
					None					=> {}
				}
			}
		}
	}

	// A comment claiming a commenter this site trusts, arriving from somewhere that commenter has
	// never used. Usually innocent -- people travel, and addresses change -- but it is also exactly
	// what impersonating a regular looks like, and it is the one case where a name in a queue is
	// worth a second look. Said plainly in the queue rather than left for a moderator to spot.
	if mismatch {
		verdict = verdict.and_then(Verdict::Hold(fmt!(
			"claims a commenter this site knows, but from somewhere they have not commented from 			before -- worth checking it is them")));
	}
	// A comment with no proof is held even where the moderator would have allowed it. The proof is
	// what distinguishes a reader who has a browser from something that posts to a URL, and a trusted
	// commenter's address is exactly what a spammer would forge to skip the queue.
	if !proved {
		verdict = verdict.and_then(Verdict::Hold(fmt!("no proof of work was sent")));
	}

	let mut stored = c;
	stored.state = verdict.state();
	stored.reason = verdict.reason();

	// Spam is stored rather than dropped: a wrong judgement must be recoverable, and what arrives is
	// worth being able to look at.
	res!(put(db, &stored));

	// A new commenter with a handle is remembered now, unapproved, so the queue can show that this is
	// their first and so blocking them later has something to attach to.
	if let Some(h) = stored.author.handle() {
		if known.is_none() {
			res!(set_trust(db, &h, false, None, &sub.now));
		}
	}

	// The id goes back with the answer so the caller can hand its author a token: this is the only
	// moment anybody can prove they wrote this, and there is no second chance to say so. Spam gets
	// none -- there is nothing to correct.
	let told = match stored.state {
		CommentState::Approved	=> Received::Published,
		CommentState::Spam	=> Received::Refused(fmt!("judged spam")),
		_			=> Received::Held,
	};
	let editable = stored.state != CommentState::Spam;
	Ok((told, if editable { Some(stored.id) } else { None }))
}


/// How long a commenter may correct what they just wrote.
///
/// Long enough to notice a typo and fix it, short enough that the right to edit does not outlive the
/// moment of writing.
pub const EDIT_WINDOW_SECS: u64 = 900;

/// A token proving the holder wrote a particular comment.
///
/// One-way from the comment's id and the site's secret, so it cannot be computed by somebody who
/// did not receive it, and it names exactly one comment. Handed back once, in a cookie, when the
/// comment is taken.
pub fn edit_token(id: &str, secret: &[u8]) -> String {
	let h = HashScheme::new_sha256().hash(&[id.as_bytes(), b"comment-edit", secret], []);
	hex(&h.as_hashform().as_vec())[..32].to_string()
}

/// Whether a token is the one for this comment, compared without leaking where it differs.
pub fn edit_token_ok(id: &str, secret: &[u8], given: &str) -> bool {
	let want = edit_token(id, secret);
	if want.len() != given.len() {
		return false;
	}
	// Constant time in the length compared: a token is a secret, and an early return on the first
	// wrong character tells whoever is guessing how much of their guess was right.
	let mut diff = 0u8;
	for (a, b) in want.bytes().zip(given.bytes()) {
		diff |= a ^ b;
	}
	diff == 0
}

/// Whether a comment is still within the window its author may correct it in.
pub fn editable(c: &Comment, now_secs: u64) -> bool {
	// A comment whose stamp will not read is not editable: an unreadable time cannot be shown to be
	// recent, and guessing in the permissive direction would make the window unbounded.
	match parse_stamp_secs(&c.created) {
		Some(t)	=> now_secs.saturating_sub(t) < EDIT_WINDOW_SECS,
		None	=> false,
	}
}

/// Unix seconds from a stamp this module wrote, where it reads as one.
///
/// **Strict on purpose, and not `CalClock::parse_iso`.** That delegates to a general datetime parser
/// which is lenient by design -- it reads "not a time at all" as *some* time, which was caught by the
/// test below. A permissive read here would hand an unbounded edit window to any comment whose stamp
/// was unreadable, so this accepts exactly the shape [`now_stamp`] writes and nothing else.
fn parse_stamp_secs(s: &str) -> Option<u64> {
	let b = s.as_bytes();
	if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
		return None;
	}
	if b[10] != b'T' && b[10] != b' ' {
		return None;
	}
	let num = |from: usize, to: usize| -> Option<i64> {
		let part = s.get(from..to)?;
		if !part.bytes().all(|c| c.is_ascii_digit()) {
			return None;
		}
		part.parse::<i64>().ok()
	};
	let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
	let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
	if !(1..=12).contains(&mo) || !(1..=31).contains(&d)
		|| h > 23 || mi > 59 || sec > 60
	{
		return None;
	}
	let days = days_from_civil(y, mo, d);
	let secs = days * 86_400 + h * 3600 + mi * 60 + sec;
	if secs < 0 { None } else { Some(secs as u64) }
}

/// Days from the Unix epoch to a civil date, by Howard Hinnant's algorithm.
///
/// Shifts the year to start in March so the leap day falls at the end of a four-century cycle, which
/// is what makes the whole thing arithmetic rather than a table.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
	let y = if m <= 2 { y - 1 } else { y };
	let era = if y >= 0 { y } else { y - 399 } / 400;
	let yoe = y - era * 400;
	let mp = (m + 9) % 12;
	let doy = (153 * mp + 2) / 5 + d - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	era * 146_097 + doe - 719_468
}

/// Replaces what a comment says, keeping who wrote it and when.
///
/// **An edit to a published comment returns it to the queue.** Otherwise the edit window is a
/// bait-and-switch: write something agreeable, be approved, then change it to whatever you liked,
/// with the site's endorsement already attached. A comment still waiting is edited in place, since
/// nobody has seen it and nothing has been endorsed.
pub fn edit<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	slug:	&str,
	id:	&str,
	body:	&str,
)
	-> Outcome<bool>
{
	let mut c = match res!(get(db, slug, id)) {
		Some(c)	=> c,
		None	=> return Ok(false),
	};
	if !valid_body(body) {
		return Ok(false);
	}
	c.body = body.trim().to_string();
	if c.state == CommentState::Approved {
		c.state = CommentState::Pending;
		c.reason = Some(fmt!("edited by its author after it was published"));
	}
	res!(put(db, &c));
	Ok(true)
}

/// The key the site's own comments-open switch lives under.
const OPEN_KEY: &str = "publish/comments-open";

/// Whether this site is taking comments, as the site itself has decided.
///
/// The config's `comments` is the **starting position**, not the standing one: an operator sets it
/// once when a site is built, and after that the person running the site opens and closes comments
/// from the console without touching a file or restarting anything. A site that has never decided
/// takes the config's word.
pub fn comments_open<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:		Option<&(Arc<RwLock<DB>>, UID)>,
	from_config:	bool,
)
	-> bool
{
	let db = match db {
		Some(d)	=> d,
		None	=> return from_config,
	};
	let (db_arc, _) = db;
	let guard = match db_arc.read() {
		Ok(g)	=> g,
		// A lock this cannot take is not a reason to open comments on a site that wanted them
		// shut, so the config's answer stands.
		Err(_)	=> return from_config,
	};
	match guard.get(&dat!(OPEN_KEY), None) {
		Ok(Some((Dat::Bool(b), _)))	=> b,
		_				=> from_config,
	}
}

/// Records whether the site is taking comments.
pub fn set_comments_open<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
	open:	bool,
)
	-> Outcome<()>
{
	let (db_arc, user) = db;
	let guard = lock_read!(db_arc);
	res!(guard.insert(dat!(OPEN_KEY), Dat::Bool(open), *user, None));
	Ok(())
}

/// The key a sender's rate record lives under.
const RATE_PREFIX: &str = "publish/comment-rate/";

/// The least time between two comments from one sender.
/// See [`PublishConfig::comment_rate_secs`](crate::srv::publish::PublishConfig::comment_rate_secs).

/// Whether a sender may comment now, and the record of their having done so.
///
/// Keyed on the **salted address hash**, not on the address they typed: an attacker varies the
/// address freely and cannot as easily vary where they are. This is the one durable signal about a
/// sender, and it was collected and ignored until an adversarial review pointed that out.
///
/// Two bounds, because they stop different things. The interval stops a flood; the hourly count
/// stops a slow drip that would otherwise never trip an interval at all. A sender with no address
/// hash -- which should not happen, since the caller supplies one -- is not rate limited here, and
/// is bounded by the per-post caps instead.
pub fn rate_allows<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:		&(Arc<RwLock<DB>>, UID),
	from:		&str,
	interval:	u64,
	hourly:		u32,
)
	-> Outcome<bool>
{
	// Both off: the site has decided its readers share addresses, or that the per-post ceilings are
	// bound enough. Nothing is read and nothing is written.
	if interval == 0 && hourly == 0 {
		return Ok(true);
	}
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	let key = dat!(fmt!("{}{}", RATE_PREFIX, from));

	let (db_arc, user) = db;
	let (last, count, window) = {
		let guard = lock_read!(db_arc);
		match res!(guard.get(&key, None)) {
			Some((Dat::List(v), _)) if v.len() == 3 => {
				let n = |i: usize| match v.get(i) {
					Some(Dat::U64(x))	=> *x,
					_			=> 0,
				};
				(n(0), n(1) as u32, n(2))
			}
			_ => (0, 0, 0),
		}
	};

	// A new hour resets the count. The window is the hour the first of them landed in, not a
	// rolling one: a rolling window needs every timestamp kept, and this needs three numbers.
	let (count, window) = if now.saturating_sub(window) >= 3600 {
		(0, now)
	} else {
		(count, window)
	};

	if (interval > 0 && now.saturating_sub(last) < interval)
		|| (hourly > 0 && count >= hourly)
	{
		return Ok(false);
	}

	let guard = lock_read!(db_arc);
	res!(guard.insert(
		key,
		Dat::List(vec![dat!(now), dat!((count + 1) as u64), dat!(window)]),
		*user,
		None,
	));
	Ok(true)
}

/// The key the site's comment secret lives under.
const SECRET_KEY: &str = "publish/comment-secret";

/// How many bytes of secret.
const SECRET_LEN: usize = 32;

/// The site's own comment secret, made once and kept.
///
/// Used for two things that must not be guessable and must be *stable*: the proof-of-work challenge,
/// and the salt a sender's address is hashed with. Domain-separated at each use, so the same bytes
/// serve both without either becoming an oracle for the other.
///
/// **Stored rather than per-process** for a plain reason: a challenge that changed on restart would
/// refuse every comment written against a form fetched before it, and the reader would have done the
/// work for nothing. A site's secret outlives its process.
pub fn site_secret<
	const UIDL: usize,
	UID:	NumIdDat<UIDL>,
	ENC:	Encrypter,
	KH:	Hasher,
	DB:	Database<UIDL, UID, ENC, KH>,
>(
	db:	&(Arc<RwLock<DB>>, UID),
)
	-> Outcome<Vec<u8>>
{
	let (db_arc, user) = db;
	{
		let guard = lock_read!(db_arc);
		if let Some((Dat::BU8(bytes), _)) = res!(guard.get(&dat!(SECRET_KEY), None)) {
			if bytes.len() == SECRET_LEN {
				return Ok(bytes);
			}
		}
	}
	let mut fresh = vec![0u8; SECRET_LEN];
	Rand::fill_u8(&mut fresh);
	let guard = lock_read!(db_arc);
	res!(guard.insert(dat!(SECRET_KEY), Dat::BU8(fresh.clone()), *user, None));
	Ok(fresh)
}

/// An ISO timestamp for now.
///
/// A comment's arrival is a real instant rather than a date somebody chose, so it is stamped from the
/// clock here rather than taken from anything a sender supplied.
pub fn now_stamp() -> String {
	use oxedyne_fe2o3_datime::time::CalClock;
	match CalClock::now_utc() {
		Ok(t)	=> t.to_string(),
		// A clock that will not read is not a reason to lose a comment. An empty stamp sorts first
		// and is visibly wrong in the queue, which is the right way for this to fail.
		Err(_)	=> String::new(),
	}
}
