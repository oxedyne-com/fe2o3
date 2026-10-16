//! A moment as a person types it, and as a person reads it back.
//!
//! This is stricter than `Parser`, which swaps a day for a month when the first
//! guess fails. Here the answer picks a point in a history to return to, so a
//! reading is either exactly one instant or an error that says why not. The
//! zone's rules come from `CalClockZone::local_to_utc`, so a wall time in a
//! spring gap is refused and one in an autumn fold takes the earlier instant.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
	calendar::CalendarDate,
	time::{
		CalClockZone,
		LocalTimeResult,
	},
};

use oxedyne_fe2o3_core::prelude::*;

const MICROS:	i64 = 1_000_000;	// per second
const DAY:		i64 = 86_400;		// seconds

/// The forms `read` accepts, each with its meaning, for a caller to give a
/// person whose words it could not read. The refusal names the words and the
/// reason and leaves this list to the caller, which knows how it is being read.
pub const FORMS: [(&str, &str); 5] = [
	("HH:MM[:SS]",						"a clock time today, or yesterday's if today's has not come yet"),
	("<n>s|m|h|d|w, repeated: 1h30m",	"that long ago, a day being 24 hours"),
	("today [HH:MM[:SS]]",				"now, or that clock time today"),
	("yesterday [HH:MM[:SS]]",			"24 hours ago, or that clock time yesterday"),
	("YYYY-MM-DD HH:MM[:SS[.ffffff]]",	"that date and time here, or with a trailing Z or +HH:MM the instant that offset gives"),
];

/// What a person typed, as an instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
	pub micros:	i64,	// UTC microseconds since the Unix epoch
	pub fold:	bool,	// the wall time came twice and the earlier was taken
}

// A clock reading: whole seconds after midnight and the microseconds beyond.
struct Clock {
	secs:	i64,
	micros:	i64,
}

/// The instant `said` names, in UTC microseconds. `now_utc_micros` is a single
/// clock reading taken by the caller, and `zone` is where a wall time is read.
pub fn read(
	said:			&str,
	now_utc_micros:	i64,
	zone:			&CalClockZone,
)
	-> Outcome<i64>
{
	Ok(res!(read_noted(said, now_utc_micros, zone)).micros)
}

/// As `read`, saying also whether the wall time fell in an autumn fold.
pub fn read_noted(
	said:			&str,
	now_utc_micros:	i64,
	zone:			&CalClockZone,
)
	-> Outcome<Moment>
{
	let words: Vec<&str> = said.split_ascii_whitespace().collect();
	let is = |w: &str, name: &str| w.eq_ignore_ascii_case(name);
	match words.as_slice() {
		[] => Err(err!("Nothing was said, so there is no moment to read."; Invalid, Input, Missing)),
		[w] if is(w, "today")		=> Ok(Moment { micros: now_utc_micros, fold: false }),
		[w] if is(w, "yesterday")	=> {
			let micros = res!(now_utc_micros.checked_sub(DAY * MICROS).ok_or_else(||
				err!("A day before {} is out of range.", now_utc_micros; Invalid, Input, Range)));
			Ok(Moment { micros, fold: false })
		},
		[w, c] if is(w, "today")	=> {
			let clock = res!(clock_of(c));
			let days = res!(today_of(now_utc_micros, zone));
			wall(days, &clock, zone)
		},
		[w, c] if is(w, "yesterday")	=> {
			let clock = res!(clock_of(c));
			let days = res!(today_of(now_utc_micros, zone)) - 1;
			wall(days, &clock, zone)
		},
		[w] => {
			if let Some(secs) = res!(offset(w)) {
				let back = res!(secs.checked_mul(MICROS).ok_or_else(||
					err!("The offset '{}' is too large.", w; Invalid, Input, Range)));
				let micros = res!(now_utc_micros.checked_sub(back).ok_or_else(||
					err!("The offset '{}' reaches before the earliest instant.", w;
						Invalid, Input, Range)));
				return Ok(Moment { micros, fold: false });
			}
			if let Some(clock) = res!(clock(w)) {
				// A bare clock time is today's, unless that is still to come.
				let days = res!(today_of(now_utc_micros, zone));
				let today = res!(wall(days, &clock, zone));
				if today.micros > now_utc_micros {
					return wall(days - 1, &clock, zone);
				}
				return Ok(today);
			}
			stamp(&words, zone)
		},
		_ => stamp(&words, zone),
	}
}

/// A moment as `read` takes it back: date, time to the second, and the offset
/// in force at that instant. A wall time that came twice is told apart by its
/// offset.
pub fn show(micros: i64, zone: &CalClockZone) -> Outcome<String> {
	let secs = micros.div_euclid(MICROS);
	let off = res!(zone.offset_millis_at_time(secs.saturating_mul(1000))) as i64 / 1000;
	let (date, sod) = res!(split_local(secs + off));
	let sign = if off < 0 { '-' } else { '+' };
	let a = off.abs();
	if a % 60 != 0 {
		Ok(fmt!("{} {:02}:{:02}:{:02} {}{:02}:{:02}:{:02}",
			date, sod / 3600, sod / 60 % 60, sod % 60, sign, a / 3600, a / 60 % 60, a % 60))
	} else {
		Ok(fmt!("{} {:02}:{:02}:{:02} {}{:02}:{:02}",
			date, sod / 3600, sod / 60 % 60, sod % 60, sign, a / 3600, a / 60 % 60))
	}
}

/// A moment in UTC, ending in Z.
pub fn show_utc(micros: i64) -> Outcome<String> {
	let (date, sod) = res!(split_local(micros.div_euclid(MICROS)));
	Ok(fmt!("{} {:02}:{:02}:{:02}Z", date, sod / 3600, sod / 60 % 60, sod % 60))
}

// The calendar date and the seconds into it of a count of local seconds.
fn split_local(local_secs: i64) -> Outcome<(String, i64)> {
	let date = res!(CalendarDate::from_days_since_epoch(
		local_secs.div_euclid(DAY),
		CalClockZone::utc(),
	));
	Ok((
		fmt!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day()),
		local_secs.rem_euclid(DAY),
	))
}

// The local date, as days since the epoch, on which `now` falls.
fn today_of(now: i64, zone: &CalClockZone) -> Outcome<i64> {
	match zone.utc_to_local(now.div_euclid(1000)) {
		LocalTimeResult::Single(local) => Ok(local.div_euclid(DAY * 1000)),
		_ => Err(err!(
			"The zone {} has no local time at {} microseconds.", zone.id(), now;
			Invalid, Input, Range)),
	}
}

// A wall time on a local date, in the zone.
fn wall(days: i64, clock: &Clock, zone: &CalClockZone) -> Outcome<Moment> {
	let local_ms = res!(days.checked_mul(DAY).ok_or_else(||
		err!("The day {} is out of range.", days; Invalid, Input, Range)))
		.saturating_add(clock.secs).saturating_mul(1000);
	match zone.local_to_utc(local_ms) {
		LocalTimeResult::Single(utc) => Ok(Moment {
			micros:	utc * 1000 + clock.micros,
			fold:	false,
		}),
		LocalTimeResult::Ambiguous(first, _) => Ok(Moment {
			micros:	first * 1000 + clock.micros,
			fold:	true,
		}),
		LocalTimeResult::None => {
			let date = res!(split_local(local_ms.div_euclid(1000))).0;
			Err(err!(
				"{} {} does not exist in {}{}.",
				date, clock_text(clock), zone.id(), gap_note(zone, local_ms);
				Invalid, Input, Range))
		},
	}
}

// What the clocks did, when a wall time falls where they skipped.
fn gap_note(zone: &CalClockZone, local_ms: i64) -> String {
	const WINDOW: i64 = 2 * DAY * 1000;
	let before = match zone.offset_millis_at_time(local_ms - WINDOW) {
		Ok(off) => off as i64,
		Err(_) => return String::new(),
	};
	let after = match zone.offset_millis_at_time(local_ms + WINDOW) {
		Ok(off) => off as i64,
		Err(_) => return String::new(),
	};
	if before == after {
		return String::new();
	}
	// The first second under the new offset.
	let mut lo = (local_ms - WINDOW).div_euclid(1000);
	let mut hi = (local_ms + WINDOW).div_euclid(1000);
	while hi - lo > 1 {
		let mid = lo + (hi - lo) / 2;
		match zone.offset_millis_at_time(mid * 1000) {
			Ok(off) if off as i64 == before	=> lo = mid,
			_								=> hi = mid,
		}
	}
	let stood = (hi * 1000 + before).div_euclid(1000);
	let became = (hi * 1000 + after).div_euclid(1000);
	match (split_local(stood), split_local(became)) {
		(Ok((_, a)), Ok((_, b))) => fmt!(
			": the clocks there went from {:02}:{:02} to {:02}:{:02}, {} to {}",
			a / 3600, a / 60 % 60, b / 3600, b / 60 % 60, offset_text(before), offset_text(after)),
		_ => String::new(),
	}
}

fn offset_text(millis: i64) -> String {
	let secs = millis / 1000;
	let a = secs.abs();
	fmt!("{}{:02}:{:02}", if secs < 0 { '-' } else { '+' }, a / 3600, a / 60 % 60)
}

fn clock_text(clock: &Clock) -> String {
	let (h, m, s) = (clock.secs / 3600, clock.secs / 60 % 60, clock.secs % 60);
	if clock.micros != 0 {
		fmt!("{:02}:{:02}:{:02}.{:06}", h, m, s, clock.micros)
	} else if s != 0 {
		fmt!("{:02}:{:02}:{:02}", h, m, s)
	} else {
		fmt!("{:02}:{:02}", h, m)
	}
}

// A clock time that must be one, for after today or yesterday.
fn clock_of(word: &str) -> Outcome<Clock> {
	match res!(clock(word)) {
		Some(clock) => Ok(clock),
		None => Err(err!(
			"'{}' is not a clock time; HH:MM or HH:MM:SS is read after today or yesterday.", word;
			Invalid, Input)),
	}
}

// H:MM or HH:MM, then optionally :SS and a fraction of up to six digits. None
// when the word is not shaped like a clock time, and an error when it is but
// its numbers cannot be.
fn clock(word: &str) -> Outcome<Option<Clock>> {
	let colon = match word.find(':') {
		Some(c) => c,
		None => return Ok(None),
	};
	let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
	let hour = &word[..colon];
	if !digits(hour) || hour.len() > 2 {
		return Ok(None);
	}
	let mut rest = word[colon + 1..].splitn(2, ':');
	let minute = rest.next().unwrap_or("");
	if !digits(minute) || minute.len() != 2 {
		return Ok(None);
	}
	let mut micros = 0;
	let second = match rest.next() {
		None => "00",
		Some(tail) => {
			let mut parts = tail.splitn(2, '.');
			let s = parts.next().unwrap_or("");
			if let Some(f) = parts.next() {
				if !digits(f) || f.len() > 6 {
					return Ok(None);
				}
				micros = res!(f.parse::<i64>().map_err(|e| err!(e, "{}", f; Invalid, Input)))
					* 10i64.pow(6 - f.len() as u32);
			}
			if !digits(s) || s.len() != 2 {
				return Ok(None);
			}
			s
		},
	};
	let h: i64 = res!(hour.parse().map_err(|e| err!(e, "{}", word; Invalid, Input)));
	let m: i64 = res!(minute.parse().map_err(|e| err!(e, "{}", word; Invalid, Input)));
	let s: i64 = res!(second.parse().map_err(|e| err!(e, "{}", word; Invalid, Input)));
	if h > 23 {
		return Err(err!("The hour in '{}' is {}, which is not between 0 and 23.", word, h;
			Invalid, Input, Range));
	}
	if m > 59 {
		return Err(err!("The minute in '{}' is {}, which is not between 0 and 59.", word, m;
			Invalid, Input, Range));
	}
	if s > 59 {
		return Err(err!("The second in '{}' is {}, which is not between 0 and 59.", word, s;
			Invalid, Input, Range));
	}
	Ok(Some(Clock { secs: h * 3600 + m * 60 + s, micros }))
}

// Runs of a number and a unit, 1h30m, as seconds. None when the word is not
// shaped like that, which includes a bare number.
fn offset(word: &str) -> Outcome<Option<i64>> {
	let b = word.as_bytes();
	if b.is_empty() || !b[0].is_ascii_digit() {
		return Ok(None);
	}
	let mut total: i64 = 0;
	let mut i = 0;
	while i < b.len() {
		let from = i;
		let mut n: i64 = 0;
		while i < b.len() && b[i].is_ascii_digit() {
			n = res!(n.checked_mul(10).and_then(|n| n.checked_add((b[i] - b'0') as i64))
				.ok_or_else(|| err!("The offset '{}' is too large.", word; Invalid, Input, Range)));
			i += 1;
		}
		if i == from || i >= b.len() {
			return Ok(None);
		}
		let unit = match b[i] {
			b's' => 1,
			b'm' => 60,
			b'h' => 3600,
			b'd' => DAY,
			b'w' => 7 * DAY,
			_ => return Ok(None),
		};
		i += 1;
		total = res!(n.checked_mul(unit).and_then(|n| total.checked_add(n))
			.ok_or_else(|| err!("The offset '{}' is too large.", word; Invalid, Input, Range)));
	}
	Ok(Some(total))
}

// A date and a time, joined by a space or T, with an optional Z or +HH:MM
// attached to the time or standing as a word of its own.
fn stamp(words: &[&str], zone: &CalClockZone) -> Outcome<Moment> {
	let said = words.join(" ");
	let bad = || err!("'{}' is not a time that can be read.", said; Invalid, Input);
	let splits_at_t = |w: &str| w.len() > 11 && matches!(w.as_bytes()[10], b'T' | b't');
	let (date, time, word_zone) = match words {
		[a] if splits_at_t(a)				=> (&a[..10], &a[11..], None),
		[a, z] if splits_at_t(a)			=> (&a[..10], &a[11..], Some(*z)),
		[a, t]								=> (*a, *t, None),
		[a, t, z]							=> (*a, *t, Some(*z)),
		_									=> return Err(bad()),
	};
	let days = match res!(date_of(date)) {
		Some(days)	=> days,
		None		=> return Err(bad()),
	};
	// A zone attached to the time, unless one was also given as a word.
	let (time, attached) = split_zone(time);
	let given = match (attached, word_zone) {
		(Some(_), Some(_))	=> return Err(err!(
			"'{}' gives its offset twice.", said; Invalid, Input)),
		(Some(z), None)		=> Some(z),
		(None, Some(z))		=> Some(z),
		(None, None)		=> None,
	};
	let clock = match res!(clock(time)) {
		Some(c)	=> c,
		None	=> return Err(bad()),
	};
	match given {
		None => wall(days, &clock, zone),
		Some(z) => {
			let east = match res!(zone_offset(z)) {
				Some(east)	=> east,
				None		=> return Err(bad()),
			};
			let secs = days * DAY + clock.secs - east;
			Ok(Moment { micros: secs * MICROS + clock.micros, fold: false })
		},
	}
}

// A time and the zone text on its end, if any: ...30Z, ...30+08:00.
fn split_zone(time: &str) -> (&str, Option<&str>) {
	if let Some(t) = time.strip_suffix('Z').or_else(|| time.strip_suffix('z')) {
		return (t, Some("Z"));
	}
	match time.rfind(|c| c == '+' || c == '-') {
		Some(at) if at > 0	=> (&time[..at], Some(&time[at..])),
		_					=> (time, None),
	}
}

// Seconds east of UTC for Z, +HH:MM or +HHMM; None for anything else.
fn zone_offset(text: &str) -> Outcome<Option<i64>> {
	if text == "Z" || text == "z" {
		return Ok(Some(0));
	}
	let (sign, rest) = match text.as_bytes().first() {
		Some(b'+')	=> (1, &text[1..]),
		Some(b'-')	=> (-1, &text[1..]),
		_			=> return Ok(None),
	};
	let (h, m) = match rest.len() {
		5 if rest.as_bytes()[2] == b':'	=> (&rest[..2], &rest[3..]),
		4								=> (&rest[..2], &rest[2..]),
		_								=> return Ok(None),
	};
	if !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) {
		return Ok(None);
	}
	let h: i64 = res!(h.parse().map_err(|e| err!(e, "{}", text; Invalid, Input)));
	let m: i64 = res!(m.parse().map_err(|e| err!(e, "{}", text; Invalid, Input)));
	if h > 23 || m > 59 {
		return Err(err!(
			"The offset '{}' is more than 23:59 from UTC.", text; Invalid, Input, Range));
	}
	Ok(Some(sign * (h * 3600 + m * 60)))
}

// YYYY-MM-DD as days since the epoch. None when the word is not shaped like a
// date, and an error when it is but there was no such day.
fn date_of(word: &str) -> Outcome<Option<i64>> {
	let b = word.as_bytes();
	if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
		return Ok(None);
	}
	let part = |from: usize, to: usize| -> Option<i32> {
		let s = &word[from..to];
		if s.bytes().all(|c| c.is_ascii_digit()) { s.parse().ok() } else { None }
	};
	let (y, m, d) = match (part(0, 4), part(5, 7), part(8, 10)) {
		(Some(y), Some(m), Some(d))	=> (y, m, d),
		_							=> return Ok(None),
	};
	if y < 1 {
		return Err(err!("The year in '{}' is before 0001.", word; Invalid, Input, Range));
	}
	let date = res!(CalendarDate::new(y, m as u8, d as u8, CalClockZone::utc()).map_err(|e|
		err!(e, "'{}' is not a date.", word; Invalid, Input, Range)));
	Ok(Some(res!(date.days_since_epoch())))
}
