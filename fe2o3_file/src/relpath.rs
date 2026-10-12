//! One spelling for a path relative to a root, refused rather than repaired.
//!
//! A path that arrives from outside -- a model's tool call, a stored reference, a user's mark --
//! was checked in several places, and each place tidied it differently: one trimmed whitespace,
//! one resolved `..`, one collapsed `a//b`.  A guard that tidied one way and an act that tidied
//! another then disagreed about which file was meant, and a mark on the folder `" sp"` went into
//! force as `sp` (Daimond HIST-SAFE U2 QA F1, 12 Oct 2026).  [`RelPath`] has no tidying at all: a
//! string either already is its one canonical form, or it is refused with the reason, so a guard
//! and the act it guards can only ever see the same string.
//!
//! [Written with AI](https://need2know.ai/with-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;


/// Why a string is not a [`RelPath`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
	Empty,			// "", which names no place
	Absolute,		// a leading "/"
	Backslash,		// "\", a separator on one platform and a name character on another
	Control,		// NUL, a tab or any other control character
	EmptySegment,	// "a//b", or a trailing "/"
	Dot,			// a "." segment, including "." alone
	DotDot,			// a ".." segment, which climbs
	EdgeSpace,		// a segment that begins or ends with whitespace
}

impl fmt::Display for Fault {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.write_str(match self {
			Self::Empty			=> "it is empty",
			Self::Absolute		=> "it begins with '/', so it is absolute",
			Self::Backslash		=> "it holds a backslash; the separator is '/'",
			Self::Control		=> "it holds a control character",
			Self::EmptySegment	=> "it has an empty segment ('//' or a trailing '/')",
			Self::Dot			=> "it has a '.' segment",
			Self::DotDot		=> "it has a '..' segment",
			Self::EdgeSpace		=> "a segment begins or ends with whitespace",
		})
	}
}

/// A non-empty path relative to a root, in its one canonical form: segments joined by `/`, none
/// empty, none `.` or `..`, none with whitespace at either edge, and no backslash or control
/// character anywhere.  Made only by [`RelPath::parse`], so holding one is the proof.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RelPath(String);

impl RelPath {

	/// Why `s` is not canonical, or `None` where it is.  The one rule; [`RelPath::parse`] is this
	/// with an error that names the value.
	pub fn fault(s: &str) -> Option<Fault> {
		if s.is_empty() {
			return Some(Fault::Empty);
		}
		if s.starts_with('/') {
			return Some(Fault::Absolute);
		}
		if s.contains('\\') {
			return Some(Fault::Backslash);
		}
		if s.chars().any(char::is_control) {
			return Some(Fault::Control);
		}
		for seg in s.split('/') {
			match seg {
				""		=> return Some(Fault::EmptySegment),
				"."		=> return Some(Fault::Dot),
				".."	=> return Some(Fault::DotDot),
				_		=> (),
			}
			let edge = |c: Option<char>| c.is_some_and(char::is_whitespace);
			if edge(seg.chars().next()) || edge(seg.chars().next_back()) {
				return Some(Fault::EdgeSpace);
			}
		}
		None
	}

	/// Is `s` already a canonical relative path?
	pub fn is_canonical(s: &str) -> bool {
		Self::fault(s).is_none()
	}

	pub fn parse(s: &str) -> Outcome<Self> {
		match Self::fault(s) {
			None		=> Ok(Self(s.to_string())),
			Some(why)	=> Err(err!(
				"The relative path {:?} is refused: {}.", s, why;
				Invalid, Input)),
		}
	}

	pub fn as_str(&self) -> &str { &self.0 }

	pub fn into_string(self) -> String { self.0 }

	pub fn segments(&self) -> std::str::Split<'_, char> { self.0.split('/') }

	/// The last segment.
	pub fn name(&self) -> &str {
		match self.0.rfind('/') {
			Some(i)	=> &self.0[i + 1..],
			None	=> &self.0,
		}
	}

	/// The path less its last segment, `None` for a single segment.
	pub fn parent(&self) -> Option<Self> {
		self.0.rfind('/').map(|i| Self(self.0[..i].to_string()))
	}

	/// `self` then `rest`, which is canonical because both are.
	pub fn join(&self, rest: &Self) -> Self {
		Self(fmt!("{}/{}", self.0, rest.0))
	}

	/// Is `self` `base` or beneath it, by whole segments, so `ab` is not within `a`?
	pub fn is_within(&self, base: &Self) -> bool {
		self.0 == base.0
			|| (self.0.len() > base.0.len()
				&& self.0.starts_with(&base.0)
				&& self.0.as_bytes()[base.0.len()] == b'/')
	}
}

impl fmt::Display for RelPath {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.write_str(&self.0)
	}
}

impl AsRef<str> for RelPath {
	fn as_ref(&self) -> &str { &self.0 }
}
