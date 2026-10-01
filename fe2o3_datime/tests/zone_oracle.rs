//! The zone rules checked against GNU date, which reads the host's own
//! zoneinfo. Zones with and without daylight saving, a 30 minute shift
//! (Lord Howe) and a +05:45 offset (Kathmandu) are taken across both of
//! 2026's transitions and well past the end of a "fat" file's table, and "slim"
//! files, which end their table and leave the rest to the footer, are made with
//! the host's `zic` and run the same way.
//!
//! Where a wall time falls in a fold, GNU date picks one occurrence by the
//! accident of its search, not always the earlier, so the check there is that
//! both occurrences display as that wall time and GNU's choice is one of them.

mod common;

use common::*;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_datime::time::{
	CalClockZone,
	LocalTimeResult,
	LocalTimeType,
	TZifData,
	TZifParser,
};

use std::{
	fs,
	path::{Path, PathBuf},
	process::Command,
};

const ZONES: [&str; 5] = [
	"America/New_York",
	"Europe/London",
	"Australia/Perth",
	"Australia/Lord_Howe",
	"Asia/Kathmandu",
];

// The embedded table's zones, which new_embedded answers without the host.
const EMBEDDED: [&str; 3] = [
	"America/New_York",
	"Europe/London",
	"Australia/Sydney",
];

// Offsets over a grid of UTC seconds, and at the second either side of every
// change the zone reports. Returns how many changes there were.
fn offsets_agree(
	zone:	&CalClockZone,
	tz:		&str,
	tzdir:	Option<&Path>,
	from:	i64,
	to:		i64,
	step:	i64,
)
	-> Outcome<usize>
{
	let grid: Vec<i64> = (from..to).step_by(step as usize).collect();
	let lines: Vec<String> = grid.iter().map(|t| fmt!("@{}", t)).collect();
	let said = res!(date_batch(tz, tzdir, &lines, "+%z"));
	let mut prev: Option<(i64, i64)> = None;
	let mut edges: Vec<(i64, i64, i64)> = Vec::new(); // first second under the new offset, old, new
	for (i, t) in grid.iter().enumerate() {
		let mine = res!(zone.offset_millis_at_time(t * 1000)) as i64 / 1000;
		let theirs = z_secs(said[i].as_deref().unwrap_or("+0000"));
		assert_eq!(mine, theirs, "{} at UTC second {}", tz, t);
		if let Some((pt, po)) = prev {
			if po != mine {
				// Bisect to the first second under the new offset.
				let (mut lo, mut hi) = (pt, *t);
				while hi - lo > 1 {
					let mid = lo + (hi - lo) / 2;
					if res!(zone.offset_millis_at_time(mid * 1000)) as i64 / 1000 == po {
						lo = mid;
					} else {
						hi = mid;
					}
				}
				edges.push((hi, po, mine));
			}
		}
		prev = Some((*t, mine));
	}
	let around: Vec<String> = edges.iter().flat_map(|(t, _, _)| [fmt!("@{}", t - 1), fmt!("@{}", t)]).collect();
	let said = res!(date_batch(tz, tzdir, &around, "+%z"));
	for (k, (t, old, new)) in edges.iter().enumerate() {
		let before = z_secs(said[2 * k].as_deref().unwrap_or("+0000"));
		let after = z_secs(said[2 * k + 1].as_deref().unwrap_or("+0000"));
		assert_eq!(before, *old, "{} one second before the change at {}", tz, t);
		assert_eq!(after, *new, "{} at the change at {}", tz, t);
	}
	Ok(edges.len())
}

// Wall times over a grid of local seconds: a time GNU date refuses is one the
// zone must report as skipped, and one it dates is one the zone must reach
// there. Returns how many grid times were skipped and how many came twice.
fn locals_agree(
	zone:	&CalClockZone,
	tz:		&str,
	tzdir:	Option<&Path>,
	from:	i64,
	to:		i64,
	step:	i64,
)
	-> Outcome<(usize, usize)>
{
	let locals: Vec<i64> = (from..to).step_by(step as usize).collect();
	let wall = res!(walls(&locals));
	let said = res!(date_batch(tz, tzdir, &wall, "+%s"));
	let (mut gaps, mut folds) = (0, 0);
	let mut twice: Vec<(usize, i64, i64)> = Vec::new();
	for (i, n) in locals.iter().enumerate() {
		let theirs: Option<i64> = said[i].as_deref().and_then(|s| s.parse().ok());
		match (zone.local_to_utc(n * 1000), theirs) {
			(LocalTimeResult::Single(u), Some(g)) => {
				assert_eq!(u, g * 1000, "{} reads {} at a different instant", tz, wall[i]);
			},
			(LocalTimeResult::None, None) => gaps += 1,
			(LocalTimeResult::Ambiguous(a, b), Some(g)) => {
				folds += 1;
				assert!(a < b, "{} {}: the occurrences are not in order", tz, wall[i]);
				assert!(g == a / 1000 || g == b / 1000,
					"{} {}: GNU date's {} is neither of {} and {}", tz, wall[i], g, a / 1000, b / 1000);
				twice.push((i, a / 1000, b / 1000));
			},
			(mine, theirs) => panic!(
				"{} {}: the zone says {:?} and GNU date says {:?}", tz, wall[i], mine, theirs),
		}
	}
	// Each instant of a fold must itself show the wall time.
	let both: Vec<String> = twice.iter().flat_map(|(_, a, b)| [fmt!("@{}", a), fmt!("@{}", b)]).collect();
	let shown = res!(date_batch(tz, tzdir, &both, "+%F %T"));
	for (k, (i, a, b)) in twice.iter().enumerate() {
		assert_eq!(shown[2 * k].as_deref(), Some(wall[*i].as_str()), "{} the earlier, {}", tz, a);
		assert_eq!(shown[2 * k + 1].as_deref(), Some(wall[*i].as_str()), "{} the later, {}", tz, b);
	}
	Ok((gaps, folds))
}

#[test]
fn offsets_match_gnu_date_across_2026() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	// Transitions per zone in 2026, from zdump.
	let changes = [2, 2, 0, 2, 0];
	for (name, want) in ZONES.iter().zip(changes) {
		if let Some(zone) = res!(zone_from(ZONEINFO, name)) {
			let got = res!(offsets_agree(&zone, name, None, Y2026, Y2027, 1200));
			assert_eq!(got, want, "{} changes in 2026", name);
		}
	}
	Ok(())
}

#[test]
fn local_times_match_gnu_date_across_2026() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	// A 15 minute grid puts 4 times in an hour's gap and fold, 2 in half an hour's.
	let skipped_and_twice = [(4, 4), (4, 4), (0, 0), (2, 2), (0, 0)];
	for (name, want) in ZONES.iter().zip(skipped_and_twice) {
		if let Some(zone) = res!(zone_from(ZONEINFO, name)) {
			let got = res!(locals_agree(&zone, name, None, Y2026, Y2027, 900));
			assert_eq!(got, want, "{} gap and fold times in 2026", name);
		}
	}
	Ok(())
}

#[test]
fn the_embedded_rules_match_gnu_date_across_2026() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	// New York and London go through both changes; Sydney, in the other
	// hemisphere, is in summer time across the new year.
	let skipped_and_twice = [(4, 4), (4, 4), (4, 4)];
	for (name, want) in EMBEDDED.iter().zip(skipped_and_twice) {
		let zone = res!(CalClockZone::new_embedded(*name));
		assert!(zone.tzif_data().is_none(), "{} was meant to be the embedded rules", name);
		assert_eq!(res!(offsets_agree(&zone, name, None, Y2026, Y2027, 1200)), 2, "{}", name);
		let got = res!(locals_agree(&zone, name, None, Y2026, Y2027, 900));
		assert_eq!(got, want, "{} gap and fold times in 2026", name);
	}
	Ok(())
}

#[test]
fn years_past_a_fat_table_follow_the_footer() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	for name in ZONES {
		if let Some(zone) = res!(zone_from(ZONEINFO, name)) {
			if let Some(data) = zone.tzif_data() {
				// Fat or not, the table must end before the span tested.
				let last = data.transition_times.last().copied().unwrap_or(0);
				assert!(last < Y2043, "{} has a table that outruns the test", name);
			}
			res!(offsets_agree(&zone, name, None, Y2038, Y2043, 3600));
			res!(locals_agree(&zone, name, None, Y2038 + 86400 * 400, Y2038 + 86400 * 770, 900));
		}
	}
	Ok(())
}

#[test]
fn slim_files_agree_with_gnu_date() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	let source = Path::new(ZONEINFO).join("tzdata.zi");
	if !source.is_file() {
		eprintln!("skipped: no {:?} to make slim files from", source);
		return Ok(());
	}
	let out: PathBuf = std::env::temp_dir().join(fmt!("datime-slim-{}", std::process::id()));
	let _ = fs::remove_dir_all(&out);
	let made = Command::new("zic").arg("-b").arg("slim").arg("-d").arg(&out).arg(&source).output();
	match made {
		Ok(o) if o.status.success() => (),
		_ => {
			eprintln!("skipped: zic could not make slim files");
			return Ok(());
		},
	}
	let slim_dir = out.to_string_lossy().to_string();
	for name in ZONES {
		let (fat, slim) = match (res!(zone_from(ZONEINFO, name)), res!(zone_from(&slim_dir, name))) {
			(Some(f), Some(s)) => (f, s),
			_ => continue,
		};
		let (fat_data, slim_data) = match (fat.tzif_data(), slim.tzif_data()) {
			(Some(f), Some(s)) => (f, s),
			_ => continue,
		};
		let footer = slim_data.posix_tz_string.clone().unwrap_or_default();
		assert!(!footer.is_empty(), "{} slim file has no footer", name);
		if name == "America/New_York" || name == "Europe/London" {
			// The tables must differ for the footer to be what answers.
			assert!(slim_data.transition_times.len() < fat_data.transition_times.len(),
				"{} slim has {} transitions to fat's {}", name,
				slim_data.transition_times.len(), fat_data.transition_times.len());
			let last = slim_data.transition_times.last().copied().unwrap_or(0);
			assert!(last < Y2026, "{} slim table still runs to {}", name, last);
		}
		res!(offsets_agree(&slim, name, Some(&out), Y2026, Y2027, 1200));
		res!(offsets_agree(&slim, name, Some(&out), Y2038, Y2043, 3600));
		res!(locals_agree(&slim, name, Some(&out), Y2026, Y2027, 900));
		res!(locals_agree(&slim, name, Some(&out), Y2038 + 86400 * 400, Y2038 + 86400 * 770, 900));
	}
	let _ = fs::remove_dir_all(&out);
	Ok(())
}

// A zone that is nothing but a footer, so the rule is all that answers.
fn footer_zone(rule: &str) -> Outcome<CalClockZone> {
	let data = TZifData {
		version:					3,
		transition_times:			Vec::new(),
		transition_types:			Vec::new(),
		local_time_types:			vec![LocalTimeType { utc_offset: 0, is_dst: false, abbreviation_index: 0 }],
		abbreviations:				String::new(),
		leap_seconds:				Vec::new(),
		standard_wall_indicators:	Vec::new(),
		ut_local_indicators:		Vec::new(),
		posix_tz_string:			Some(rule.to_string()),
	};
	CalClockZone::from_tzif_data("footer", data)
}

#[test]
fn footer_rules_match_gnu_date() -> Outcome<()> {
	if skip_without_oracle() { return Ok(()); }
	// Both hemispheres, a rule time other than 02:00, a negative one, one past
	// a day, the last week of a month, a half hour of daylight saving, a
	// negative one (Dublin's, where winter is the daylight time), and the two
	// day-of-year forms.
	let rules = [
		"EST5EDT,M3.2.0,M11.1.0",
		"CET-1CEST,M3.5.0,M10.5.0/3",
		"<+1030>-10:30<+11>-11,M10.1.0,M4.1.0",
		"AEST-10AEDT,M10.1.0,M4.1.0/3",
		"NZST-12NZDT,M9.5.0,M4.1.0/3",
		"<-0330>3:30<-0230>,M3.2.0,M11.1.0",
		"IST-1GMT0,M10.5.0,M3.5.0/1",
		"EST5EDT,M3.2.0/-1,M11.1.0/26",
		"EST5EDT,J60,J300",
		"EST5EDT,59,299",
		"<+05>-5",
	];
	for rule in rules {
		let zone = res!(footer_zone(rule));
		res!(offsets_agree(&zone, rule, None, Y2026, Y2027, 1800));
		res!(offsets_agree(&zone, rule, None, Y2038, Y2043, 7200));
		res!(locals_agree(&zone, rule, None, Y2026, Y2027, 1800));
	}
	Ok(())
}

#[test]
fn a_footer_that_cannot_be_read_is_an_error_not_a_guess() -> Outcome<()> {
	for rule in ["EST5EDT,M13.1.0,M11.1.0", "EST5EDT,M3.2.0", "ES", "EST5EDT,M3.2.0,M11.1.0,x", "EST"] {
		let zone = res!(footer_zone(rule));
		assert!(zone.offset_millis_at_time(Y2026 * 1000).is_err(), "'{}' was read", rule);
		assert_eq!(zone.local_to_utc(Y2026 * 1000), LocalTimeResult::None, "'{}'", rule);
	}
	Ok(())
}

// Answers taken from GNU date once and kept, so a machine with none still checks
// the transitions. Each wall time is given as seconds of local time read as UTC,
// and each answer as the UTC milliseconds of the occurrence or occurrences.
#[test]
fn fixed_answers_across_the_transitions() -> Outcome<()> {
	use LocalTimeResult::{Ambiguous, None as Skipped, Single};
	let ms = |secs: i64| secs * 1000;
	// America/New_York: 2026-03-08 02:30 is skipped, 2026-11-01 01:30 comes twice.
	let mut zones = vec![res!(CalClockZone::new_embedded("America/New_York"))];
	zones.extend(res!(zone_from(ZONEINFO, "America/New_York")));
	for zone in &zones {
		assert_eq!(zone.local_to_utc(ms(1_772_933_400)), Single(ms(1_772_951_400)), "01:30 is EST");
		assert_eq!(zone.local_to_utc(ms(1_772_937_000)), Skipped, "02:30 is skipped");
		assert_eq!(zone.local_to_utc(ms(1_772_940_600)), Single(ms(1_772_955_000)), "03:30 is EDT");
		assert_eq!(zone.local_to_utc(ms(1_793_496_600)), Ambiguous(ms(1_793_511_000), ms(1_793_514_600)),
			"01:30 on 2026-11-01 comes in EDT and then EST");
		// A time between the seconds keeps its milliseconds.
		assert_eq!(zone.local_to_utc(ms(1_793_496_600) + 250),
			Ambiguous(ms(1_793_511_000) + 250, ms(1_793_514_600) + 250));
	}
	// Europe/London: 01:30 on 2026-03-29 is skipped, 01:30 on 2026-10-25 comes twice.
	let mut zones = vec![res!(CalClockZone::new_embedded("Europe/London"))];
	zones.extend(res!(zone_from(ZONEINFO, "Europe/London")));
	for zone in &zones {
		assert_eq!(zone.local_to_utc(ms(1_774_747_800)), Skipped);
		assert_eq!(zone.local_to_utc(ms(1_792_891_800)), Ambiguous(ms(1_792_888_200), ms(1_792_891_800)));
	}
	// Australia/Sydney, the other hemisphere: 02:30 on 2026-04-05 comes twice.
	let mut zones = vec![res!(CalClockZone::new_embedded("Australia/Sydney"))];
	zones.extend(res!(zone_from(ZONEINFO, "Australia/Sydney")));
	for zone in &zones {
		assert_eq!(zone.local_to_utc(ms(1_775_356_200)), Ambiguous(ms(1_775_316_600), ms(1_775_320_200)));
		assert_eq!(zone.local_to_utc(ms(1_768_478_400)), Single(ms(1_768_438_800)),
			"2026-01-15 12:00 is summer time");
	}
	// Australia/Lord_Howe, half an hour: 02:15 on 2026-10-04 is skipped, 01:45
	// on 2026-04-05 comes twice.
	if let Some(zone) = res!(zone_from(ZONEINFO, "Australia/Lord_Howe")) {
		assert_eq!(zone.local_to_utc(ms(1_791_080_100)), Skipped);
		assert_eq!(zone.local_to_utc(ms(1_775_353_500)), Ambiguous(ms(1_775_313_900), ms(1_775_315_700)));
	}
	// Asia/Kathmandu is +05:45 all year.
	if let Some(zone) = res!(zone_from(ZONEINFO, "Asia/Kathmandu")) {
		assert_eq!(zone.local_to_utc(ms(1_782_907_200)), Single(ms(1_782_886_500)));
	}
	Ok(())
}
