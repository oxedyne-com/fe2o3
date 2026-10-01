//! The reader of a moment a person typed, checked against GNU date: what it
//! reads, in which zone, and what it refuses.

mod common;

use common::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_datime::{
	parser::moment::{
		self,
		Moment,
		FORMS,
	},
	time::CalClockZone,
};

const ZONES: [&str; 5] = [
	"Australia/Perth",
	"America/New_York",
	"Europe/London",
	"Australia/Lord_Howe",
	"Asia/Kathmandu",
];

const MICROS: i64 = 1_000_000;

// 2026-10-01 18:23:30.25 UTC, which is already the 2nd of October at Perth, Lord
// Howe and Kathmandu and still the 1st at New York and London.
const NOW: i64 = 1_790_879_010 * MICROS + 250_000;

fn zones() -> Outcome<Vec<(&'static str, CalClockZone)>> {
	let mut zones = Vec::new();
	for name in ZONES {
		if let Some(zone) = res!(zone_from(ZONEINFO, name)) {
			zones.push((name, zone));
		}
	}
	Ok(zones)
}

// GNU date's reading of one string, in UTC seconds.
fn gnu(tz: &str, said: &str) -> Outcome<Option<i64>> {
	let out = res!(date_batch(tz, None, &[said.to_string()], "+%s"));
	Ok(out[0].as_deref().and_then(|s| s.parse().ok()))
}

// The local date on which an instant falls, as GNU date writes it.
fn gnu_day(tz: &str, secs: i64) -> Outcome<String> {
	let out = res!(date_batch(tz, None, &[fmt!("@{}", secs)], "+%F"));
	Ok(res!(out[0].clone().ok_or_else(|| err!("GNU date refused @{}", secs; Invalid, Mismatch))))
}

// What a refusal says, or a note that there was none.
fn refusal(said: Outcome<i64>) -> String {
	match said {
		Ok(micros)	=> fmt!("no refusal: {}", micros),
		Err(e)		=> fmt!("{}", e),
	}
}

#[test]
fn a_full_stamp_with_no_offset_is_read_in_the_zone() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	for (name, zone) in res!(zones()) {
		for said in ["2026-06-15 17:30:15", "2026-06-15T17:30", "2026-01-15 00:00:00", "2026-12-31 23:59:59",
			"2024-02-29 12:00"]
		{
			let want = res!(gnu(name, &said.replace('T', " "))).map(|s| s * MICROS);
			assert_eq!(Some(res!(moment::read(said, NOW, &zone))), want, "{}: '{}'", name, said);
		}
		// The fraction rides along.
		let a = res!(moment::read("2026-06-15 17:30:15", NOW, &zone));
		let b = res!(moment::read("2026-06-15 17:30:15.123456", NOW, &zone));
		let c = res!(moment::read("2026-06-15 17:30:15.5", NOW, &zone));
		assert_eq!((b - a, c - a), (123_456, 500_000), "{}", name);
	}
	Ok(())
}

#[test]
fn a_stamp_that_gives_its_offset_ignores_the_zone() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	let cases = [
		"2026-06-15 17:30:15Z",
		"2026-06-15 17:30:15 Z",
		"2026-06-15T17:30:15Z",
		"2026-06-15t17:30:15z",
		"2026-06-15 17:30:15+08:00",
		"2026-06-15 17:30:15 +08:00",
		"2026-06-15T17:30:15 -04:00",
		"2026-06-15 17:30:15 +0545",
		"2026-06-15 17:30 -03:30",
		"2026-11-01 01:30:00-04:00",
		"2026-11-01 01:30:00-05:00",
		"0001-01-01 00:00:00Z",
		"9999-12-31 23:59:59Z",
		"2000-02-29 12:00Z",
	];
	// The one answer must come from every zone alike, Perth to New York.
	for said in cases {
		let spaced = said.replace('T', " ").replace('t', " ").replace(" Z", "Z").replace(" z", "Z");
		let want = res!(gnu("UTC", &spaced.replace('z', "Z"))).map(|s| s * MICROS);
		assert!(want.is_some(), "GNU date would not read '{}'", spaced);
		for (name, zone) in res!(zones()) {
			assert_eq!(Some(res!(moment::read(said, NOW, &zone))), want, "{}: '{}'", name, said);
		}
	}
	Ok(())
}

#[test]
fn a_clock_time_is_today_unless_that_is_still_to_come() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	for (name, zone) in res!(zones()) {
		let day = res!(gnu_day(name, NOW.div_euclid(MICROS)));
		let here_now = {
			// The local time of day of NOW, as GNU writes it.
			let out = res!(date_batch(name, None, &[fmt!("@{}", NOW.div_euclid(MICROS))], "+%H:%M:%S"));
			out[0].clone().unwrap_or_default()
		};
		for said in ["00:00", "03:15", "09:30:45", "12:00", "17:30", "23:59:59"] {
			let today = res!(gnu(name, &fmt!("{} {}", day, said))).map(|s| s * MICROS);
			let yesterday = res!(gnu(name, &fmt!("{} {} yesterday", day, said))).map(|s| s * MICROS);
			let want = match today {
				// The reading is later than now, so it is yesterday's.
				Some(t) if t > NOW	=> yesterday,
				other				=> other,
			};
			assert_eq!(Some(res!(moment::read(said, NOW, &zone))), want,
				"{}: '{}' with the time {} on {}", name, said, here_now, day);
		}
		// The very second of now is not later than it.
		let now_clock = here_now.clone();
		let at_now = res!(moment::read(&now_clock, NOW - 250_000, &zone));
		assert_eq!(at_now, NOW - 250_000, "{}: now to the second", name);
	}
	Ok(())
}

#[test]
fn today_and_yesterday_alone_are_now_and_24_hours_ago() -> Outcome<()> {
	for (name, zone) in res!(zones()) {
		assert_eq!(res!(moment::read("today", NOW, &zone)), NOW, "{}", name);
		assert_eq!(res!(moment::read("yesterday", NOW, &zone)), NOW - 86_400 * MICROS, "{}", name);
		assert_eq!(res!(moment::read("  Yesterday ", NOW, &zone)), NOW - 86_400 * MICROS, "{}", name);
		assert_eq!(res!(moment::read("TODAY", NOW, &zone)), NOW, "{}", name);
	}
	Ok(())
}

#[test]
fn today_and_yesterday_with_a_clock_take_the_local_date() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	for (name, zone) in res!(zones()) {
		let day = res!(gnu_day(name, NOW.div_euclid(MICROS)));
		for clock in ["00:00", "05:30", "17:30:15", "23:59"] {
			let today = res!(gnu(name, &fmt!("{} {}", day, clock))).map(|s| s * MICROS);
			let yesterday = res!(gnu(name, &fmt!("{} {} yesterday", day, clock))).map(|s| s * MICROS);
			// A time still to come today is read as it is: only a bare clock is moved back.
			assert_eq!(Some(res!(moment::read(&fmt!("today {}", clock), NOW, &zone))), today,
				"{}: today {}", name, clock);
			assert_eq!(Some(res!(moment::read(&fmt!("yesterday {}", clock), NOW, &zone))), yesterday,
				"{}: yesterday {}", name, clock);
		}
	}
	Ok(())
}

#[test]
fn offsets_count_back_from_now() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	// GNU date's relative items, which take a date to be relative to, are the
	// oracle; in UTC a day is 24 hours, as it is here.
	let now_text = res!(date_batch("UTC", None, &[fmt!("@{}", NOW.div_euclid(MICROS))], "+%F %T"));
	let now_text = now_text[0].clone().unwrap_or_default();
	// GNU date's "ago" turns over only the item before it, so each item has its own.
	let cases = [
		("10m", "10 minutes ago"),
		("90s", "90 seconds ago"),
		("1h30m", "1 hour ago 30 minutes ago"),
		("2d", "2 days ago"),
		("1w", "1 week ago"),
		("1w2d3h4m5s", "1 week ago 2 days ago 3 hours ago 4 minutes ago 5 seconds ago"),
		("0s", "0 seconds ago"),
		("30m1h", "30 minutes ago 1 hour ago"),
	];
	for (said, relative) in cases {
		let want = res!(gnu("UTC", &fmt!("{}Z {}", now_text, relative))).map(|s| s * MICROS + 250_000);
		assert!(want.is_some(), "GNU date would not read '{}'", relative);
		for (name, zone) in res!(zones()) {
			assert_eq!(Some(res!(moment::read(said, NOW, &zone))), want, "{}: '{}'", name, said);
		}
	}
	Ok(())
}

#[test]
fn a_wall_time_in_a_spring_gap_is_refused_naming_the_gap() -> Outcome<()> {
	let ny = match res!(zone_from(ZONEINFO, "America/New_York")) { Some(z) => z, None => return Ok(()) };
	// 2026-03-08 03:00 is the first moment after the gap.
	assert_eq!(res!(moment::read("2026-03-08 03:00", NOW, &ny)), 1_772_953_200 * MICROS);
	let text = refusal(moment::read("2026-03-08 02:30", NOW, &ny));
	for part in ["2026-03-08 02:30", "does not exist", "America/New_York", "from 02:00 to 03:00", "-05:00 to -04:00"] {
		assert!(text.contains(part), "the refusal '{}' lacks '{}'", text, part);
	}
	// On the day itself, 08:00 EDT, today's 02:30 is the gap.
	let on_the_day = 1_772_971_200 * MICROS;
	for said in ["02:30", "today 02:30"] {
		let text = refusal(moment::read(said, on_the_day, &ny));
		assert!(text.contains("2026-03-08 02:30") && text.contains("does not exist"), "'{}': {}", said, text);
	}
	// Yesterday's 02:30 is fine, and so is the day after, when 02:30 has passed.
	assert!(moment::read("yesterday 02:30", on_the_day, &ny).is_ok());
	assert!(moment::read("02:30", 1_773_055_200 * MICROS, &ny).is_ok());
	// Just after midnight the next day, today's 02:30 is still to come, so the
	// yesterday that stands in for it is the gap day, which is refused as well.
	let early = 1_773_032_400 * MICROS; // 2026-03-09 01:00 EDT
	let text = refusal(moment::read("02:30", early, &ny));
	assert!(text.contains("2026-03-08 02:30") && text.contains("does not exist"), "{}", text);
	// Half an hour gaps are named as well: Lord Howe skips 02:00 to 02:30.
	if let Some(lh) = res!(zone_from(ZONEINFO, "Australia/Lord_Howe")) {
		let text = refusal(moment::read("2026-10-04 02:15", NOW, &lh));
		for part in ["2026-10-04 02:15", "does not exist", "from 02:00 to 02:30", "+10:30 to +11:00"] {
			assert!(text.contains(part), "the refusal '{}' lacks '{}'", text, part);
		}
	}
	Ok(())
}

#[test]
fn a_wall_time_in_an_autumn_fold_takes_the_earlier_and_says_so() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	let ny = match res!(zone_from(ZONEINFO, "America/New_York")) { Some(z) => z, None => return Ok(()) };
	let earlier = res!(gnu("UTC", "2026-11-01 01:30:00-04:00")).unwrap_or(0) * MICROS;
	let later = res!(gnu("UTC", "2026-11-01 01:30:00-05:00")).unwrap_or(0) * MICROS;
	assert_eq!(later - earlier, 3600 * MICROS);
	let got = res!(moment::read_noted("2026-11-01 01:30", NOW, &ny));
	assert_eq!(got, Moment { micros: earlier, fold: true });
	// Either occurrence can be asked for by its offset, and then it is not a fold.
	let got = res!(moment::read_noted("2026-11-01 01:30 -05:00", NOW, &ny));
	assert_eq!(got, Moment { micros: later, fold: false });
	// Neighbours are not folds.
	for said in ["2026-11-01 00:59:59", "2026-11-01 02:00", "2026-10-31 01:30"] {
		assert!(!res!(moment::read_noted(said, NOW, &ny)).fold, "'{}'", said);
	}
	// A bare clock time in the fold, when the first occurrence has passed and when
	// it has not. Now is 01:10 EST (the second 01:10), then 01:10 EDT (the first).
	let at_0110_est = 1_793_513_400 * MICROS;
	let at_0110_edt = 1_793_509_800 * MICROS;
	assert_eq!(res!(moment::read_noted("01:30", at_0110_est, &ny)), Moment { micros: earlier, fold: true });
	// The first 01:30 has not come yet, so yesterday's is meant.
	let got = res!(moment::read_noted("01:30", at_0110_edt, &ny));
	assert_eq!(got, Moment { micros: earlier - 86_400 * MICROS, fold: false });
	Ok(())
}

#[test]
fn the_reader_is_strict() -> Outcome<()> {
	let zone = CalClockZone::utc();
	let refused = [
		"", "   ", "foo", "10", "3", "0", "10M", "10x", "1h30", "h", "m", "1h 30m", "-10m", "+10m", "1.5h",
		"25:00", "24:00", "12:60", "12:30:60", "9", "9:5", "12:30.5", "12:30:15.1234567", "12:30:15.",
		"2026-6-15 17:30", "2026-10-01", "2026-10-01T", "2026-02-30 10:00", "2026-13-01 10:00", "2026-00-10 10:00",
		"2026-10-00 10:00", "2100-02-29 10:00", "26-10-01 10:00", "2026/10/01 10:00", "2026-10-01  25:00",
		"2026-10-01 10:00 Zulu", "2026-10-01 10:00 +24:00", "2026-10-01 10:00 +08:60", "2026-10-01 10:00Z Z",
		"2026-10-01 10:00 +08:00 +08:00", "2026-10-01 10:00 +8", "2026-10-01 10:00 extra words here",
		"today 17", "today foo", "yesterday 25:00", "today 17:30 extra", "yesterday yesterday",
		"tomorrow", "last friday", "17:30 yesterday", "0000-01-01 00:00Z",
		"99999999999999999999s", "9999999999999w", "1h9999999999999999999m",
	];
	for said in refused {
		assert!(moment::read(said, NOW, &zone).is_err(), "'{}' was read", said);
	}
	Ok(())
}

#[test]
fn a_refusal_lists_the_forms() -> Outcome<()> {
	let utc = CalClockZone::utc();
	let text = refusal(moment::read("last friday", NOW, &utc));
	assert!(text.contains("'last friday'"), "{}", text);
	for (form, _) in FORMS {
		assert!(text.contains(form), "the refusal lacks the form {}: {}", form, text);
	}
	// A reading that has the shape of a time and a wrong number says which.
	let text = refusal(moment::read("25:00", NOW, &utc));
	assert!(text.contains("hour") && text.contains("25"), "{}", text);
	let text = refusal(moment::read("2026-02-30 10:00", NOW, &utc));
	assert!(text.contains("2026-02-30"), "{}", text);
	Ok(())
}

#[test]
fn show_matches_gnu_date_and_reads_back() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	// A spread of instants, with both sides of each 2026 change in each zone.
	let mut instants: Vec<i64> = vec![0, 1, 951_782_400, 1_790_879_010, 1_772_953_199, 1_772_953_200,
		1_793_509_199, 1_793_509_200, 1_793_512_799, 1_793_512_800, 4_102_444_799, 253_402_214_399, -1, -86_400];
	let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
	for _ in 0..300 {
		seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
		instants.push((seed >> 33) as i64 % 4_000_000_000);
	}
	let lines: Vec<String> = instants.iter().map(|t| fmt!("@{}", t)).collect();
	for (name, zone) in res!(zones()) {
		let said = res!(date_batch(name, None, &lines, "+%F %T %:z"));
		for (t, want) in instants.iter().zip(said) {
			let shown = res!(moment::show(t * MICROS + 999_999, &zone));
			assert_eq!(Some(shown.as_str()), want.as_deref(), "{} at {}", name, t);
			// What is shown is what is read, to the second, and it names its own offset.
			assert_eq!(res!(moment::read(&shown, NOW, &zone)), t * MICROS, "{}: '{}'", name, shown);
		}
	}
	let said = res!(date_batch("UTC", None, &lines, "+%F %T"));
	for (t, want) in instants.iter().zip(said) {
		let shown = res!(moment::show_utc(*t * MICROS));
		assert_eq!(Some(shown.trim_end_matches('Z')), want.as_deref(), "UTC at {}", t);
		assert_eq!(res!(moment::read(&shown, NOW, &CalClockZone::utc())), t * MICROS, "'{}'", shown);
	}
	Ok(())
}

#[test]
fn random_wall_times_match_gnu_date_in_every_zone() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	// Local times from 1971 to 2099, over both hemispheres' rules, and over the
	// leap days that the Julian day arithmetic could get wrong.
	let mut locals: Vec<i64> = Vec::new();
	let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
	for _ in 0..4000 {
		seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
		locals.push(((seed >> 33) as i64 % 4_000_000_000) + 31_536_000);
	}
	let mut wall = res!(walls(&locals));
	// 29 February and the days about it, in a leap year, a century leap year and a
	// century that is not one.
	for day in ["2024-02-28", "2024-02-29", "2024-03-01", "2000-02-29", "2096-02-29", "2100-02-28", "2100-03-01"] {
		wall.push(fmt!("{} 12:00:00", day));
	}
	for (name, zone) in res!(zones()) {
		let said = res!(date_batch(name, None, &wall, "+%s"));
		for (w, g) in wall.iter().zip(said) {
			let g: Option<i64> = g.and_then(|s| s.parse().ok());
			// The stamp's own text, which is what a person would type.
			let got = moment::read_noted(w, NOW, &zone);
			match (got, g) {
				(Ok(m), Some(g)) if !m.fold	=> assert_eq!(m.micros, g * MICROS, "{}: {}", name, w),
				(Ok(m), Some(g))			=> {
					// In a fold GNU date takes either occurrence, an hour or so apart.
					let gap = (m.micros - g * MICROS).abs();
					assert!(gap <= 86_400 * MICROS && gap % (1800 * MICROS) == 0, "{}: {} fold {} vs {}", name, w, m.micros, g);
				},
				(Err(_), None)				=> (), // a skipped time is refused by both
				(got, g)					=> panic!("{}: {} -> {:?} but GNU date says {:?}", name, w, got, g),
			}
		}
	}
	Ok(())
}
