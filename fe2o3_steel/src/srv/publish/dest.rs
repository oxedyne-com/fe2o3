//! Where a post goes beyond the site's own pages.
//!
//! The site's own pages are the *origin*: a local write to the store that either happens or does not,
//! and cannot half-succeed. Everywhere else -- a Mastodon server, a Bluesky PDS, an inbox -- is a
//! [`Delivery`]: an unreliable network reached over time, with its own state, its own retries and its
//! own returned address. So a post carries one [`PostState`](super::PostState) for itself and a
//! [`Delivery`] for each remote, and the two are not the same kind of thing.
//!
//! # A destination is described, not hard-coded into each caller
//!
//! Each [`Destination`] answers a [`Capability`]: how long a body it takes, whether it carries a link
//! without penalty, whether media can ride along, and what a post there costs. The composer reads the
//! capability to derive a default [`Rendition`] and to count length as the author types; the sender
//! reads it to know what it may send. A new destination is a new variant and a new capability, and
//! nothing that already works has to change -- which is the test the design is meant to pass.
//!
//! # This module is data, not delivery
//!
//! Everything here is a pure value: the enum, the capability, the rendition, the delivery state and
//! the retry policy. Nothing here opens a socket or reads a clock. The sender fills a
//! [`DeliveryState::Sent`] with the moment and the permalink the remote returned; the worker consults
//! [`DeliveryState::backoff_secs`] against a clock it owns. Keeping the model clock-free is what lets
//! it be tested without either.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_jdat::prelude::*;


/// A place a post is delivered to, besides the site's own pages.
///
/// The own site is not here: it is the origin, a store write, not a delivery over a network. A
/// `Destination` is always a remote, and always something that can be slow, refuse, or vanish.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Destination {
	/// Subscribers, reached through the site's own DKIM sender.
	Email,
	/// A Mastodon server, by a static bearer token.
	Mastodon,
	/// A Bluesky PDS, by an app password exchanged for a session.
	Bluesky,
	/// A Substack publication. Per-post has no sanctioned API and rides an unofficial one; the
	/// supported path is a bulk feed import, so this is named but not wired.
	Substack,
	/// X, pay-per-use, behind an OAuth 2.0 client.
	X,
	/// Threads, behind Meta's OAuth and app review.
	Threads,
}

impl Destination {

	/// Every destination the module names, in the order a picker shows them: the free and wired first,
	/// the costed and the unbuilt after.
	pub const ALL: [Self; 6] = [
		Self::Email,
		Self::Mastodon,
		Self::Bluesky,
		Self::Substack,
		Self::X,
		Self::Threads,
	];

	/// The word a record stores.
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Email	=> "email",
			Self::Mastodon	=> "mastodon",
			Self::Bluesky	=> "bluesky",
			Self::Substack	=> "substack",
			Self::X		=> "x",
			Self::Threads	=> "threads",
		}
	}

	/// The destination a word names, or nothing where the word is one this version does not know.
	///
	/// Not a lenient default, and not an error either. A record naming a destination this version
	/// cannot place is a record a later version wrote, and the safe reading is to drop *that delivery*
	/// -- never to guess a different remote (which would post to the wrong place) and never to refuse
	/// the whole post (which would make every new destination a migration). The caller skips a `None`.
	pub fn of(s: &str) -> Option<Self> {
		match s {
			"email"		=> Some(Self::Email),
			"mastodon"	=> Some(Self::Mastodon),
			"bluesky"	=> Some(Self::Bluesky),
			"substack"	=> Some(Self::Substack),
			"x"		=> Some(Self::X),
			"threads"	=> Some(Self::Threads),
			_		=> None,
		}
	}

	/// What this destination will take, and what it costs.
	pub fn capability(&self) -> Capability {
		match self {
			// The newsletter is the whole post, not a blurb, so it has no length worth enforcing here.
			// `wired` stays false because email is not a per-remote queue delivery with a single
			// permalink to return: it is a fan-out to the site's own subscriber list, sent from the
			// subscribers page (`/manage/subscribers`) through the site's DKIM sender, not queued as a
			// `Delivery` on the post the way Mastodon and Bluesky are.
			Self::Email	=> Capability {
				name:		"Email",
				max_chars:	None,
				links:		true,
				media:		true,
				cost_micros:	0,
				link_micros:	0,
				wired:		false,
			},
			Self::Mastodon	=> Capability {
				name:		"Mastodon",
				max_chars:	Some(500),
				links:		true,
				media:		true,
				cost_micros:	0,
				link_micros:	0,
				wired:		false,
			},
			Self::Bluesky	=> Capability {
				name:		"Bluesky",
				max_chars:	Some(300),
				links:		true,
				media:		true,
				cost_micros:	0,
				link_micros:	0,
				wired:		false,
			},
			// The bulk feed import carries the whole post and needs no per-post rendition; the per-post
			// unofficial API is deferred, so nothing here is wired.
			Self::Substack	=> Capability {
				name:		"Substack",
				max_chars:	None,
				links:		true,
				media:		true,
				cost_micros:	0,
				link_micros:	0,
				wired:		false,
			},
			// $0.015 a post, and $0.20 when the post carries a link: the base is 15,000 micros and the
			// link adds 185,000 more, to 200,000.
			Self::X		=> Capability {
				name:		"X",
				max_chars:	Some(280),
				links:		true,
				media:		true,
				cost_micros:	15_000,
				link_micros:	185_000,
				wired:		false,
			},
			Self::Threads	=> Capability {
				name:		"Threads",
				max_chars:	Some(500),
				links:		true,
				media:		true,
				cost_micros:	0,
				link_micros:	0,
				wired:		false,
			},
		}
	}
}


/// What a destination will take, and what it costs.
///
/// Read by the composer to derive a default rendition and to count length as an author types, and by
/// the sender to know what it may send. Every field is a fact about the remote, not a preference of the
/// site's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capability {
	/// The name a picker shows.
	pub name:		&'static str,
	/// The longest body the remote accepts, in characters, or nothing where the limit is not worth
	/// enforcing here (an email carries the whole post).
	pub max_chars:		Option<usize>,
	/// Whether a link in the body is carried as written, rather than stripped or charged extra for.
	pub links:		bool,
	/// Whether an image can ride along with the words.
	pub media:		bool,
	/// What a post costs, in millionths of a US dollar. Zero for the free remotes.
	pub cost_micros:	u64,
	/// What a post costs *extra* when its body carries a link, in the same millionths. Zero where a
	/// link is free.
	pub link_micros:	u64,
	/// Whether this module can actually deliver to the remote yet, or only names it. A picker greys out
	/// what is not wired rather than offering a post that will never go.
	pub wired:		bool,
}

impl Capability {

	/// What a post costs here, given whether its body carries a link, in millionths of a US dollar.
	pub fn cost_of(&self, has_link: bool) -> u64 {
		if has_link {
			self.cost_micros + self.link_micros
		} else {
			self.cost_micros
		}
	}
}


/// The words a destination gets, as against the words the site's own page gets.
///
/// A remote is not the site: a 280-character timeline cannot take a 2,000-word essay, and pushing one
/// there unedited reads as a machine, which costs the following the post was written to build. So each
/// delivery carries its own rendition -- derived by default so nothing has to be typed twice, editable
/// so nothing reads like a bot, and remembered once edited so a hand-written one is not overwritten by
/// the next automatic pass.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Rendition {
	/// The body this destination is sent.
	pub text:	String,
	/// Whether this is still the derived default (`true`) or has been edited by hand (`false`). The
	/// distinction is the whole reason renditions are remembered: an automatic pass may replace an
	/// automatic rendition and must never replace a hand-written one.
	pub auto:	bool,
}

impl Rendition {

	/// The default body for a length-limited social destination: the post's title and its canonical
	/// link, trimmed to fit.
	///
	/// The link is kept whole -- a truncated URL is a broken one -- and the title gives way to make
	/// room. Where even the link alone will not fit, the link alone is sent, because a link that works
	/// is worth more than a title that is cut off before it.
	pub fn promo(title: &str, url: &str, max_chars: Option<usize>) -> Self {
		let joined = |t: &str| -> String {
			if t.is_empty() {
				url.to_string()
			} else {
				fmt!("{}\n\n{}", t, url)
			}
		};
		let text = match max_chars {
			None	=> joined(title),
			Some(max)	=> {
				let full = joined(title);
				if full.chars().count() <= max {
					full
				} else {
					// Reserve the link and the two newlines before it, and give the rest to the title.
					let url_len = url.chars().count();
					let reserve = url_len + 2;
					if reserve >= max {
						// Not even the link fits with a title; send the link on its own.
						url.to_string()
					} else {
						let room = max - reserve;
						// A trimmed title ends on an ellipsis, so leave a place for it.
						let keep = room.saturating_sub(1);
						let cut = title.char_indices()
							.take(keep)
							.filter(|(_, c)| c.is_whitespace())
							.map(|(i, _)| i)
							.last()
							.unwrap_or_else(|| title.char_indices()
								.nth(keep)
								.map(|(i, _)| i)
								.unwrap_or(title.len()));
						let mut t = title[..cut].trim_end().to_string();
						t.push('…');
						joined(&t)
					}
				}
			}
		};
		Self { text, auto: true }
	}

	/// The rendition as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("text"), dat!(self.text.clone()));
		m.insert(dat!("auto"), Dat::Bool(self.auto));
		Dat::Map(m)
	}

	/// The rendition from a daticle. A missing or ill-typed field takes its default -- an absent body
	/// is the empty one, and an absent `auto` reads as a default still (the safe reading: an automatic
	/// pass may overwrite it, where treating an unknown as hand-written would freeze a bad default).
	pub fn from_dat(d: &Dat) -> Self {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Self::default(),
		};
		let text = match m.get(&dat!("text")) {
			Some(Dat::Str(s))	=> s.clone(),
			_			=> String::new(),
		};
		let auto = match m.get(&dat!("auto")) {
			Some(Dat::Bool(b))	=> *b,
			_			=> true,
		};
		Self { text, auto }
	}
}


/// The most times a failed delivery is retried before it is left alone.
///
/// A remote that has refused this many times is not going to take the post because it was asked once
/// more, and a queue that retries for ever is a queue that never drains. The number is arbitrary;
/// having one, so the queue is bounded, is not.
pub const MAX_RETRIES: u32 = 5;

/// The first wait after a failure, in seconds. Each further failure doubles it, to [`BACKOFF_CAP_SECS`].
pub const BACKOFF_BASE_SECS: u64 = 60;

/// The longest wait between retries, in seconds. Doubling stops here, so a stubborn remote is retried
/// hourly rather than never.
pub const BACKOFF_CAP_SECS: u64 = 3600;


/// Where one delivery to one remote has got to.
///
/// The state of the post at the remote, as against [`PostState`](super::PostState), which is the state
/// of the post at home. A remote can have taken a post the site still calls a draft, or refused one it
/// calls live: the two states are about different places and do not track each other.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryState {
	/// Waiting for the worker to try it, or to try it again.
	Queued,
	/// Taken. `at` is the moment the remote confirmed it, `permalink` the address it gave back -- kept
	/// so the site can say "also on Mastodon" and link to where the post actually landed.
	Sent {
		/// When the remote confirmed the post, as the sender recorded it.
		at:		String,
		/// Where the remote put it, for a backlink.
		permalink:	String,
	},
	/// Refused. `at` is when, `err` is what the remote or the wire said, `retries` how many attempts
	/// have failed. At [`MAX_RETRIES`] the worker stops trying and the failure stands.
	Failed {
		/// When the attempt failed.
		at:		String,
		/// What went wrong, for the author to read.
		err:		String,
		/// How many attempts have failed.
		retries:	u32,
	},
}

impl DeliveryState {

	/// How long to wait before the next attempt, given how many have already failed.
	///
	/// Exponential from [`BACKOFF_BASE_SECS`], doubling each failure, capped at [`BACKOFF_CAP_SECS`]:
	/// a remote briefly down is retried soon, a remote long down is not hammered. Pure, so the worker
	/// supplies the clock and this supplies only the interval.
	pub fn backoff_secs(retries: u32) -> u64 {
		let shifted = BACKOFF_BASE_SECS.checked_shl(retries).unwrap_or(u64::MAX);
		shifted.min(BACKOFF_CAP_SECS)
	}

	/// Whether the worker is done with this delivery -- it has been taken, or refused past retrying.
	pub fn is_terminal(&self) -> bool {
		match self {
			Self::Queued		=> false,
			Self::Sent { .. }	=> true,
			Self::Failed { retries, .. }	=> *retries >= MAX_RETRIES,
		}
	}

	/// The word a record stores for which state this is.
	fn tag(&self) -> &'static str {
		match self {
			Self::Queued		=> "queued",
			Self::Sent { .. }	=> "sent",
			Self::Failed { .. }	=> "failed",
		}
	}

	/// The state as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("state"), dat!(self.tag().to_string()));
		match self {
			Self::Queued	=> {},
			Self::Sent { at, permalink }	=> {
				m.insert(dat!("at"),		dat!(at.clone()));
				m.insert(dat!("permalink"),	dat!(permalink.clone()));
			}
			Self::Failed { at, err, retries }	=> {
				m.insert(dat!("at"),		dat!(at.clone()));
				m.insert(dat!("err"),		dat!(err.clone()));
				m.insert(dat!("retries"),	Dat::U32(*retries));
			}
		}
		Dat::Map(m)
	}

	/// The state from a daticle.
	///
	/// A state word this version cannot read is **not** taken as queued, because a queued delivery is
	/// re-sent and a post this version cannot understand the state of might already have gone -- sending
	/// it again would double-post. So an unreadable state is a spent failure: inert, retried by nobody,
	/// and visible as a failure to whoever looks.
	pub fn from_dat(d: &Dat) -> Self {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return Self::spent("a delivery state that is not a map"),
		};
		let tag = match m.get(&dat!("state")) {
			Some(Dat::Str(s))	=> s.clone(),
			_			=> return Self::spent("a delivery with no state"),
		};
		let get_str = |key: &str| -> String {
			match m.get(&dat!(key)) {
				Some(Dat::Str(s))	=> s.clone(),
				_			=> String::new(),
			}
		};
		match tag.as_str() {
			"queued"	=> Self::Queued,
			"sent"		=> Self::Sent {
				at:		get_str("at"),
				permalink:	get_str("permalink"),
			},
			"failed"	=> Self::Failed {
				at:		get_str("at"),
				err:		get_str("err"),
				retries:	as_u32(m.get(&dat!("retries"))),
			},
			other		=> Self::spent(&fmt!("a delivery state this version cannot read: '{}'", other)),
		}
	}

	/// A failure that has used up its retries: a terminal state for a delivery nothing can safely act
	/// on.
	fn spent(why: &str) -> Self {
		Self::Failed {
			at:		String::new(),
			err:		why.to_string(),
			retries:	MAX_RETRIES,
		}
	}
}


/// One post's delivery to one remote: where it goes, in what words, and how far it has got.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Delivery {
	/// The remote.
	pub dest:	Destination,
	/// The words that remote is sent.
	pub rendition:	Rendition,
	/// How far the delivery has got.
	pub state:	DeliveryState,
}

impl Delivery {

	/// A fresh delivery to a remote, queued and not yet tried.
	pub fn new(dest: Destination, rendition: Rendition) -> Self {
		Self { dest, rendition, state: DeliveryState::Queued }
	}

	/// The delivery as a daticle.
	pub fn to_dat(&self) -> Dat {
		let mut m = DaticleMap::new();
		m.insert(dat!("dest"),		dat!(self.dest.as_str().to_string()));
		m.insert(dat!("rendition"),	self.rendition.to_dat());
		m.insert(dat!("state"),		self.state.to_dat());
		Dat::Map(m)
	}

	/// The delivery from a daticle, or nothing where its destination is one this version does not know
	/// -- a delivery the caller drops rather than misroutes.
	pub fn from_dat(d: &Dat) -> Option<Self> {
		let m = match d {
			Dat::Map(m)	=> m,
			_		=> return None,
		};
		let dest = match m.get(&dat!("dest")) {
			Some(Dat::Str(s))	=> ok!(Destination::of(s)),
			_			=> return None,
		};
		let rendition = match m.get(&dat!("rendition")) {
			Some(d)	=> Rendition::from_dat(d),
			None	=> Rendition::default(),
		};
		let state = match m.get(&dat!("state")) {
			Some(d)	=> DeliveryState::from_dat(d),
			None	=> DeliveryState::Queued,
		};
		Some(Self { dest, rendition, state })
	}
}


/// A `u32` out of a daticle that may hold one under any of the widths jdat writes an integer as, or
/// zero where there is no readable number.
fn as_u32(d: Option<&Dat>) -> u32 {
	match d {
		Some(Dat::U32(n))	=> *n,
		Some(Dat::U16(n))	=> *n as u32,
		Some(Dat::U8(n))	=> *n as u32,
		Some(Dat::U64(n))	=> *n as u32,
		_			=> 0,
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	/// Every destination's word round-trips, and a word from outside the set is nobody's destination.
	#[test]
	fn test_a_destination_round_trips_by_word_00() -> Outcome<()> {
		for d in Destination::ALL {
			assert_eq!(Destination::of(d.as_str()), Some(d));
		}
		assert_eq!(Destination::of("myspace"), None);
		Ok(())
	}

	/// A capability's cost answers whether a link is in the body. X is the only costed one.
	#[test]
	fn test_x_costs_more_with_a_link_01() -> Outcome<()> {
		let x = Destination::X.capability();
		assert_eq!(x.cost_of(false), 15_000);
		assert_eq!(x.cost_of(true), 200_000);
		let m = Destination::Mastodon.capability();
		assert_eq!(m.cost_of(true), 0);
		Ok(())
	}

	/// A promo that fits is left whole; the link is always kept whole, and the title gives way.
	#[test]
	fn test_a_promo_keeps_the_link_whole_02() -> Outcome<()> {
		let url = "https://example.com/asides/on-rent";
		// Fits: title and link, untouched.
		let r = Rendition::promo("On rent", url, Some(280));
		assert!(r.text.contains("On rent"));
		assert!(r.text.ends_with(url));
		assert!(r.auto);
		// Does not fit: the whole URL still survives, the title is cut.
		let long = "On rent and the long slow theft of the thing a person stands on and calls their own";
		let r = Rendition::promo(long, url, Some(60));
		assert!(r.text.ends_with(url), "the link must survive whole: {:?}", r.text);
		assert!(r.text.chars().count() <= 60, "over the limit: {:?}", r.text);
		assert!(r.text.contains('…'), "a cut title should show it was cut: {:?}", r.text);
		Ok(())
	}

	/// Where not even the link fits with a title, the link goes on its own rather than truncated.
	#[test]
	fn test_a_promo_sends_the_link_alone_when_it_must_03() -> Outcome<()> {
		let url = "https://example.com/asides/on-rent";
		let r = Rendition::promo("On rent", url, Some(url.chars().count() + 1));
		assert_eq!(r.text, url);
		Ok(())
	}

	/// The backoff doubles from the base and stops at the cap.
	#[test]
	fn test_backoff_doubles_then_caps_04() -> Outcome<()> {
		assert_eq!(DeliveryState::backoff_secs(0), 60);
		assert_eq!(DeliveryState::backoff_secs(1), 120);
		assert_eq!(DeliveryState::backoff_secs(2), 240);
		// Well past the cap, and no overflow panic at a silly retry count.
		assert_eq!(DeliveryState::backoff_secs(6), 3600);
		assert_eq!(DeliveryState::backoff_secs(1000), 3600);
		Ok(())
	}

	/// The worker is done with a delivery once it is sent, or once it has failed past retrying.
	#[test]
	fn test_a_delivery_is_terminal_when_done_05() -> Outcome<()> {
		assert!(!DeliveryState::Queued.is_terminal());
		assert!(DeliveryState::Sent {
			at: fmt!("t"), permalink: fmt!("p"),
		}.is_terminal());
		assert!(!DeliveryState::Failed {
			at: fmt!("t"), err: fmt!("e"), retries: 1,
		}.is_terminal());
		assert!(DeliveryState::Failed {
			at: fmt!("t"), err: fmt!("e"), retries: MAX_RETRIES,
		}.is_terminal());
		Ok(())
	}

	/// A delivery survives the trip through a daticle, in each of its states.
	#[test]
	fn test_a_delivery_round_trips_06() -> Outcome<()> {
		let cases = [
			DeliveryState::Queued,
			DeliveryState::Sent { at: fmt!("2026-07-18T10:00:00Z"), permalink: fmt!("https://m/1") },
			DeliveryState::Failed { at: fmt!("2026-07-18T10:00:00Z"), err: fmt!("429"), retries: 2 },
		];
		for state in cases {
			let d = Delivery {
				dest:		Destination::Mastodon,
				rendition:	Rendition { text: fmt!("On rent https://x"), auto: false },
				state:		state.clone(),
			};
			let back = match Delivery::from_dat(&d.to_dat()) {
				Some(b)	=> b,
				None	=> return Err(err!("a delivery did not round-trip: {:?}", d; Test, Missing)),
			};
			assert_eq!(back, d);
		}
		Ok(())
	}

	/// A delivery to a destination this version does not know is dropped, not misrouted or fatal.
	#[test]
	fn test_an_unknown_destination_is_dropped_07() -> Outcome<()> {
		let mut m = DaticleMap::new();
		m.insert(dat!("dest"),		dat!("myspace"));
		m.insert(dat!("rendition"),	Rendition::default().to_dat());
		m.insert(dat!("state"),		DeliveryState::Queued.to_dat());
		assert_eq!(Delivery::from_dat(&Dat::Map(m)), None);
		Ok(())
	}

	/// A delivery state this version cannot read is a spent failure, not a queued re-send: a post whose
	/// state is unreadable might already have gone, and must not be sent twice.
	#[test]
	fn test_an_unreadable_state_will_not_resend_08() -> Outcome<()> {
		let mut m = DaticleMap::new();
		m.insert(dat!("state"), dat!("halfway-out-the-door"));
		let state = DeliveryState::from_dat(&Dat::Map(m));
		assert!(state.is_terminal(), "an unreadable state must not be retried: {:?}", state);
		match state {
			DeliveryState::Failed { retries, .. }	=> assert_eq!(retries, MAX_RETRIES),
			other	=> return Err(err!("expected a spent failure, got {:?}", other; Test, Mismatch)),
		}
		Ok(())
	}
}
