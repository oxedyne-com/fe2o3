//! A pure stillness detector: has a changing reading gone quiet?
//!
//! [`Quiet`] holds no clock and does no I/O of its own. The caller takes its own reading of
//! whatever it is watching -- a directory survey's fingerprint, a poll loop's snapshot, anything
//! comparable by equality -- and hands it to [`Quiet::poll`] along with the time of the poll.
//! `Quiet` answers with a [`Stillness`]: still moving, settling, still, or restarted.
//!
//! This is `ore edit`'s trigger ("the tree has changed and has been still for `N` seconds")
//! pulled out so that a second caller, `fe2o3_austenite::watch`'s poll-based rebuild, can gain the
//! same debounce without reimplementing it.
//!
//! # The gap guard
//!
//! A poll loop is not always running: a laptop can suspend between one poll and the next. Waking
//! up is not an hour of stillness, so a gap between two polls more than `gap_factor` times the
//! caller's own expected poll interval restarts the quiet window, reporting [`Stillness::Restarted`]
//! rather than [`Stillness::Still`] at once. The interval is supplied on each call rather than
//! fixed at construction, since a caller may lengthen its own poll interval over a run (as `ore
//! edit` does, to keep a large tree's survey under a fixed share of one core).
//!
//! [Written with AI](https://need2know.ai/with-ai/code)\
//! Anthropic Claude

use std::time::{
	Duration,
	Instant,
};


/// What one poll found, relative to the reading and the time of the poll before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stillness {
	Moved,					// the reading differs from the one at the previous poll
	Settling(Duration),		// unchanged, but short of the quiet window; time still owed
	Still,					// unchanged for at least the quiet window
	Restarted,				// the gap since the last poll exceeded the guard; the window restarts here
}

/// The stillness state machine.
///
/// Construct with [`Quiet::new`] and feed it with [`Quiet::poll`]. `T` is whatever the caller
/// compares readings as -- a hash, a fingerprint struct, a `BTreeMap` snapshot -- `Quiet` never
/// looks inside it, only at whether it equals the last one seen.
#[derive(Clone, Debug)]
pub struct Quiet<T> {
	quiet:		Duration,			// how long a reading must hold before it counts as still
	gap_factor:	u32,				// a poll gap beyond this multiple of the poll interval restarts the window
	reading:	Option<T>,			// the reading last seen
	changed_at:	Option<Instant>,	// when `reading` last changed
	last_poll:	Option<Instant>,	// when `poll` was last called, for the gap guard
}

impl<T: PartialEq> Quiet<T> {

	/// A quiet window of `quiet`, with the default gap guard of 3 poll intervals.
	pub fn new(quiet: Duration) -> Self {
		Self::with_gap_factor(quiet, 3)
	}

	/// As [`Quiet::new`], with the gap guard's multiple of the poll interval given explicitly.
	pub fn with_gap_factor(quiet: Duration, gap_factor: u32) -> Self {
		Self {
			quiet,
			gap_factor,
			reading:	None,
			changed_at:	None,
			last_poll:	None,
		}
	}

	/// Feeds one poll's reading, taken at `now`, with polls expected roughly `poll_interval` apart.
	///
	/// `last_poll` is stamped here on every call, whichever arm is taken below -- a run of ordinary
	/// polls must never by itself trip the gap guard, which is the bug `ore-quiet-mark`'s bash
	/// loop had: it stamped the "last seen" time on only some of its paths, so its own regular
	/// polling could restart its window.
	pub fn poll(&mut self, reading: T, now: Instant, poll_interval: Duration) -> Stillness {
		let gap = self.last_poll.map(|last| now.saturating_duration_since(last));
		self.last_poll = Some(now);
		if let Some(gap) = gap {
			if gap > poll_interval.saturating_mul(self.gap_factor) {
				self.reading	= Some(reading);
				self.changed_at	= Some(now);
				return Stillness::Restarted;
			}
		}
		match &self.reading {
			Some(prev) if *prev == reading => {
				let since = now.saturating_duration_since(self.changed_at.unwrap_or(now));
				if since >= self.quiet {
					Stillness::Still
				} else {
					Stillness::Settling(self.quiet - since)
				}
			},
			// No previous reading (the first poll), or the reading has changed.
			_ => {
				self.reading	= Some(reading);
				self.changed_at	= Some(now);
				Stillness::Moved
			},
		}
	}

	/// The reading last fed to [`Quiet::poll`], if any.
	pub fn reading(&self) -> Option<&T> {
		self.reading.as_ref()
	}
}
