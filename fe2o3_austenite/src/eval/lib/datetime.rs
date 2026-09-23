// U3 owns this file. `datetime` and `duration`: constructors and methods, `display` with the `time`
// crate's format-description syntax Typst uses (`[year]-[month]-[day]`, `[month repr:long]`), and the
// date arithmetic the operators need (`datetime_add`, `datetime_sub`, for `ops.rs`). `today` reads the host
// clock; a document compiled across midnight may see two dates, which Typst shares.

use crate::eval::args::Args;
use crate::eval::func::{
	Func,
	NativeFunc,
};
use crate::eval::lib::foundations::{
	finish,
	mismatch,
	no_method,
	receiver,
};
use crate::eval::scope::Scope;
use crate::eval::value::{
	Datetime,
	Duration,
	Type,
	Value,
};
use crate::eval::Engine;
use crate::syntax::Span;

use oxedyne_fe2o3_core::prelude::*;

native_fns! {
	pub enum DatetimeFn {
		Datetime	=> "datetime",
		Today		=> "today",
		Display		=> "display",
		Year		=> "year",
		Month		=> "month",
		Weekday		=> "weekday",
		Day			=> "day",
		Hour		=> "hour",
		Minute		=> "minute",
		Second		=> "second",
		Ordinal		=> "ordinal",
		Duration	=> "duration",
		Seconds		=> "seconds",
		Minutes		=> "minutes",
		Hours		=> "hours",
		Days		=> "days",
		Weeks		=> "weeks",
	}
}

pub fn define(_scope: &mut Scope) {}

pub fn method(name: &str) -> Option<DatetimeFn> {
	let f = match name {
		"display"	=> DatetimeFn::Display,
		"year"		=> DatetimeFn::Year,
		"month"		=> DatetimeFn::Month,
		"weekday"	=> DatetimeFn::Weekday,
		"day"		=> DatetimeFn::Day,
		"hour"		=> DatetimeFn::Hour,
		"minute"	=> DatetimeFn::Minute,
		"second"	=> DatetimeFn::Second,
		"ordinal"	=> DatetimeFn::Ordinal,
		other		=> return duration_method(other),
	};
	Some(f)
}

pub fn duration_method(name: &str) -> Option<DatetimeFn> {
	let f = match name {
		"seconds"	=> DatetimeFn::Seconds,
		"minutes"	=> DatetimeFn::Minutes,
		"hours"		=> DatetimeFn::Hours,
		"days"		=> DatetimeFn::Days,
		"weeks"		=> DatetimeFn::Weeks,
		_			=> return None,
	};
	Some(f)
}

/// `datetime.today` as a function value.
pub fn today_func() -> Value { Value::Func(Func::Native(NativeFunc::Datetime(DatetimeFn::Today))) }

// Calendar arithmetic, after Howard Hinnant's `days_from_civil`: days since 1970-01-01.

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
	let y = if m <= 2 { y - 1 } else { y };
	let era = if y >= 0 { y } else { y - 399 } / 400;
	let yoe = y - era * 400;
	let mp = (m as i64 + 9) % 12;
	let doy = (153 * mp + 2) / 5 + d as i64 - 1;
	let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
	era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
	let z = z + 719_468;
	let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
	let doe = z - era * 146_097;
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let y = yoe + era * 400;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
	let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
	(if m <= 2 { y + 1 } else { y }, m, d)
}

fn is_leap(y: i64) -> bool { (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 }

fn days_in_month(y: i64, m: u32) -> u32 {
	match m {
		1 | 3 | 5 | 7 | 8 | 10 | 12	=> 31,
		4 | 6 | 9 | 11				=> 30,
		_							=> if is_leap(y) { 29 } else { 28 },
	}
}

/// Monday is 1, Sunday 7.
fn weekday(y: i64, m: u32, d: u32) -> u32 {
	let z = days_from_civil(y, m, d);
	((z + 3).rem_euclid(7) + 1) as u32
}

fn ordinal(y: i64, m: u32, d: u32) -> u32 { (days_from_civil(y, m, d) - days_from_civil(y, 1, 1) + 1) as u32 }

// The ISO 8601 week-numbering year and week.
fn iso_week(y: i64, m: u32, d: u32) -> (i64, u32) {
	let ord = ordinal(y, m, d) as i64;
	let wd = weekday(y, m, d) as i64;
	let w = (ord - wd + 10) / 7;
	if w < 1 {
		let py = y - 1;
		(py, weeks_in_year(py))
	} else if w > weeks_in_year(y) as i64 {
		(y + 1, 1)
	} else {
		(y, w as u32)
	}
}

fn weeks_in_year(y: i64) -> u32 {
	let p = |y: i64| (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400)).rem_euclid(7);
	if p(y) == 4 || p(y - 1) == 3 { 53 } else { 52 }
}

fn has_date(d: &Datetime) -> bool { d.year.is_some() && d.month.is_some() && d.day.is_some() }

fn has_time(d: &Datetime) -> bool { d.hour.is_some() && d.minute.is_some() && d.second.is_some() }

// Days since the epoch of a datetime's date part, and seconds since midnight of its time part; each
// called only once `has_date`/`has_time` has said the part is there.
fn epoch_days(dt: &Datetime) -> i64 {
	days_from_civil(dt.year.unwrap_or(1970) as i64, dt.month.unwrap_or(1) as u32, dt.day.unwrap_or(1) as u32)
}

fn clock_secs(dt: &Datetime) -> i64 {
	dt.hour.unwrap_or(0) as i64 * 3600 + dt.minute.unwrap_or(0) as i64 * 60 + dt.second.unwrap_or(0) as i64
}

fn set_date(out: &mut Datetime, days: i64) -> bool {
	let (y, m, d) = civil_from_days(days);
	if !(-9999..=9999).contains(&y) {
		return false;
	}
	out.year = Some(y as i32);
	out.month = Some(m as u8);
	out.day = Some(d as u8);
	true
}

fn set_clock(out: &mut Datetime, secs: i64) {
	out.hour = Some((secs / 3600) as u8);
	out.minute = Some((secs / 60 % 60) as u8);
	out.second = Some((secs % 60) as u8);
}

/// `datetime + duration`: a date moves by whole days, a time wraps round the clock, and a date with a time
/// carries across midnight, as Typst's. `None` past the representable years.
pub fn datetime_add(dt: &Datetime, dur: &Duration) -> Option<Datetime> {
	let secs = dur.secs.trunc() as i64;
	let mut out = *dt;
	match (has_date(dt), has_time(dt)) {
		(true, true) => {
			let total = epoch_days(dt) * 86_400 + clock_secs(dt) + secs;
			if !set_date(&mut out, total.div_euclid(86_400)) {
				return None;
			}
			set_clock(&mut out, total.rem_euclid(86_400));
		}
		(true, false) => if !set_date(&mut out, epoch_days(dt) + secs.div_euclid(86_400)) {
			return None;
		},
		(false, true) => set_clock(&mut out, (clock_secs(dt) + secs).rem_euclid(86_400)),
		(false, false) => return None,
	}
	Some(out)
}

/// `datetime - datetime`, when both have the same parts.
pub fn datetime_sub(a: &Datetime, b: &Datetime) -> Option<Duration> {
	if has_date(a) != has_date(b) || has_time(a) != has_time(b) {
		return None;
	}
	let secs = |dt: &Datetime| -> i64 {
		let d = if has_date(dt) { epoch_days(dt) * 86_400 } else { 0 };
		d + if has_time(dt) { clock_secs(dt) } else { 0 }
	};
	Some(Duration { secs: (secs(a) - secs(b)) as f64 })
}

fn now() -> Option<(i64, u32, u32)> {
	#[cfg(target_arch = "wasm32")]
	{
		let ms = js_sys::Date::now();
		Some(civil_from_days((ms / 86_400_000.0).floor() as i64))
	}
	#[cfg(not(target_arch = "wasm32"))]
	{
		match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
			Ok(d)	=> Some(civil_from_days((d.as_secs() / 86_400) as i64)),
			Err(_)	=> None,
		}
	}
}

fn opt_int(engine: &mut Engine, span: Span, v: Option<Value>) -> Outcome<Option<i64>> {
	match v {
		None				=> Ok(None),
		Some(Value::Int(i))	=> Ok(Some(i)),
		Some(other)			=> Err(mismatch(engine, span, "integer", &other)),
	}
}

fn dt_recv(engine: &mut Engine, args: &mut Args) -> Outcome<Datetime> {
	match res!(receiver(args)) {
		Value::Datetime(d)	=> Ok(d),
		other				=> Err(mismatch(engine, args.span, "datetime", &other)),
	}
}

fn dur_recv(engine: &mut Engine, args: &mut Args) -> Outcome<Duration> {
	match res!(receiver(args)) {
		Value::Duration(d)	=> Ok(d),
		other				=> Err(mismatch(engine, args.span, "duration", &other)),
	}
}

fn opt(v: Option<u8>) -> Value { v.map(|x| Value::Int(x as i64)).unwrap_or(Value::None) }

pub fn call(f: DatetimeFn, engine: &mut Engine, mut args: Args) -> Outcome<Value> {
	let span = args.span;
	// Datetime and duration share one method table; each name belongs to one of the two types.
	let owner = if duration_method(f.name()) == Some(f) {
		Some(Type::Duration)
	} else if method(f.name()) == Some(f) {
		Some(Type::Datetime)
	} else {
		None
	};
	if let (Some(owner), Some(a)) = (owner, args.items.iter().find(|a| a.name.is_none())) {
		let ty = a.value.ty();
		if ty != owner {
			return Err(no_method(engine, span, ty, f.name()));
		}
	}
	let out = match f {
		DatetimeFn::Datetime	=> {
			let y = res!(args.named::<Value>("year"));
			let y = res!(opt_int(engine, span, y));
			let mo = res!(args.named::<Value>("month"));
			let mo = res!(opt_int(engine, span, mo));
			let d = res!(args.named::<Value>("day"));
			let d = res!(opt_int(engine, span, d));
			let h = res!(args.named::<Value>("hour"));
			let h = res!(opt_int(engine, span, h));
			let mi = res!(args.named::<Value>("minute"));
			let mi = res!(opt_int(engine, span, mi));
			let s = res!(args.named::<Value>("second"));
			let s = res!(opt_int(engine, span, s));
			res!(construct(engine, span, [y, mo, d], [h, mi, s]))
		}
		DatetimeFn::Today		=> {
			let offset = res!(args.named::<Value>("offset"));
			let hours = match offset {
				None | Some(Value::Auto)	=> 0,
				Some(Value::Int(h))			=> h,
				Some(other)					=> return Err(mismatch(engine, span, "auto or integer", &other)),
			};
			let (y, m, d) = match now() {
				Some(t)	=> t,
				None	=> return Err(engine.error(span, "unable to get the current date")),
			};
			let (y, m, d) = if hours != 0 {
				civil_from_days(days_from_civil(y, m, d) + hours.div_euclid(24))
			} else {
				(y, m, d)
			};
			Value::Datetime(Datetime {
				year: Some(y as i32), month: Some(m as u8), day: Some(d as u8), hour: None, minute: None, second: None,
			})
		}
		DatetimeFn::Display		=> {
			let dt = res!(dt_recv(engine, &mut args));
			let pat = res!(args.eat::<Value>());
			let pat = match pat {
				None | Some(Value::Auto) => match (has_date(&dt), has_time(&dt)) {
					(true, true)	=> "[year]-[month]-[day] [hour]:[minute]:[second]".to_string(),
					(true, false)	=> "[year]-[month]-[day]".to_string(),
					_				=> "[hour]:[minute]:[second]".to_string(),
				},
				Some(Value::Str(s)) => (*s).clone(),
				Some(other) => return Err(mismatch(engine, span, "auto or string", &other)),
			};
			match display(&dt, &pat) {
				Ok(s)	=> Value::str(s),
				Err(m)	=> return Err(engine.error(span, m)),
			}
		}
		DatetimeFn::Year		=> res!(dt_recv(engine, &mut args)).year.map(|y| Value::Int(y as i64)).unwrap_or(Value::None),
		DatetimeFn::Month		=> opt(res!(dt_recv(engine, &mut args)).month),
		DatetimeFn::Day			=> opt(res!(dt_recv(engine, &mut args)).day),
		DatetimeFn::Hour		=> opt(res!(dt_recv(engine, &mut args)).hour),
		DatetimeFn::Minute		=> opt(res!(dt_recv(engine, &mut args)).minute),
		DatetimeFn::Second		=> opt(res!(dt_recv(engine, &mut args)).second),
		DatetimeFn::Weekday		=> {
			let dt = res!(dt_recv(engine, &mut args));
			match (dt.year, dt.month, dt.day) {
				(Some(y), Some(m), Some(d))	=> Value::Int(weekday(y as i64, m as u32, d as u32) as i64),
				_							=> Value::None,
			}
		}
		DatetimeFn::Ordinal		=> {
			let dt = res!(dt_recv(engine, &mut args));
			match (dt.year, dt.month, dt.day) {
				(Some(y), Some(m), Some(d))	=> Value::Int(ordinal(y as i64, m as u32, d as u32) as i64),
				_							=> Value::None,
			}
		}
		DatetimeFn::Duration	=> {
			let mut secs: i64 = 0;
			for (name, unit) in [("seconds", 1i64), ("minutes", 60), ("hours", 3600), ("days", 86_400), ("weeks", 604_800)] {
				let v = res!(args.named::<Value>(name));
				if let Some(n) = res!(opt_int(engine, span, v)) {
					secs = match n.checked_mul(unit).and_then(|x| secs.checked_add(x)) {
						Some(s)	=> s,
						None	=> return Err(engine.error(span, "duration is too large")),
					};
				}
			}
			Value::Duration(Duration { secs: secs as f64 })
		}
		DatetimeFn::Seconds		=> Value::Float(res!(dur_recv(engine, &mut args)).secs),
		DatetimeFn::Minutes		=> Value::Float(res!(dur_recv(engine, &mut args)).secs / 60.0),
		DatetimeFn::Hours		=> Value::Float(res!(dur_recv(engine, &mut args)).secs / 3600.0),
		DatetimeFn::Days		=> Value::Float(res!(dur_recv(engine, &mut args)).secs / 86_400.0),
		DatetimeFn::Weeks		=> Value::Float(res!(dur_recv(engine, &mut args)).secs / 604_800.0),
	};
	res!(finish(engine, args));
	Ok(out)
}

fn construct(engine: &mut Engine, span: Span, date: [Option<i64>; 3], time: [Option<i64>; 3]) -> Outcome<Value> {
	let names = [["year", "month", "day"], ["hour", "minute", "second"]];
	let date_n = date.iter().filter(|x| x.is_some()).count();
	let time_n = time.iter().filter(|x| x.is_some()).count();
	let missing = |parts: &[Option<i64>; 3], names: &[&str; 3]| -> String {
		let ms: Vec<String> = parts.iter().zip(names.iter()).filter(|(p, _)| p.is_none())
			.map(|(_, n)| fmt!("`{}`", n)).collect();
		match ms.len() {
			1	=> fmt!("add the {} argument", ms[0]),
			2	=> fmt!("add the {} and {} arguments", ms[0], ms[1]),
			_	=> fmt!("add the {}, {}, and {} arguments", ms[0], ms[1], ms[2]),
		}
	};
	if date_n == 0 && time_n == 0 {
		let e = engine.error_hint(span, "at least one of date or time must be fully specified",
			"add the `hour`, `minute`, and `second` arguments to get a valid time");
		if let Some(d) = engine.diags.last_mut() {
			d.hints.push("add the `year`, `month`, and `day` arguments to get a valid date".to_string());
		}
		return Err(e);
	}
	if date_n > 0 && date_n < 3 {
		let hint = fmt!("{} to get a valid date", missing(&date, &names[0]));
		return Err(engine.error_hint(span, "date is incomplete", hint));
	}
	if time_n > 0 && time_n < 3 {
		let hint = fmt!("{} to get a valid time", missing(&time, &names[1]));
		return Err(engine.error_hint(span, "time is incomplete", hint));
	}
	let mut dt = Datetime::default();
	if let [Some(y), Some(m), Some(d)] = date {
		if !(1..=12).contains(&m) {
			return Err(engine.error(span, "month is invalid"));
		}
		if !(-9999..=9999).contains(&y) || d < 1 || d > days_in_month(y, m as u32) as i64 {
			return Err(engine.error(span, "date is invalid"));
		}
		dt.year = Some(y as i32);
		dt.month = Some(m as u8);
		dt.day = Some(d as u8);
	}
	if let [Some(h), Some(mi), Some(s)] = time {
		if !(0..24).contains(&h) || !(0..60).contains(&mi) || !(0..60).contains(&s) {
			return Err(engine.error(span, "time is invalid"));
		}
		dt.hour = Some(h as u8);
		dt.minute = Some(mi as u8);
		dt.second = Some(s as u8);
	}
	Ok(Value::Datetime(dt))
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August",
	"September", "October", "November", "December"];
const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

const INSUFFICIENT: &str = "failed to format datetime (insufficient information)";

/// Formats with a `time`-crate format description; the error is Typst's message.
pub fn display(dt: &Datetime, pat: &str) -> std::result::Result<String, String> {
	let mut out = String::new();
	let chars: Vec<(usize, char)> = pat.char_indices().collect();
	let mut i = 0;
	while i < chars.len() {
		let (_, c) = chars[i];
		if c != '[' {
			out.push(c);
			i += 1;
			continue;
		}
		let open = chars[i].0;
		let close = match chars[i..].iter().find(|(_, x)| *x == ']') {
			Some((b, _))	=> *b,
			None			=> return Err(fmt!("unclosed opening bracket at index {}", open)),
		};
		let body = &pat[open + 1..close];
		let mut words = body.split_whitespace();
		let name = match words.next() {
			Some(n) if n.chars().all(|x| x.is_ascii_alphanumeric() || x == '_') => n,
			_ => return Err(fmt!("expected component name at index {}", open)),
		};
		let mut mods: Vec<(&str, &str)> = Vec::new();
		for w in words {
			match w.split_once(':') {
				Some(kv)	=> mods.push(kv),
				None		=> return Err(fmt!("expected modifier value at index {}", open)),
			}
		}
		let get = |k: &str| mods.iter().find(|(m, _)| *m == k).map(|(_, v)| *v);
		let pad = |n: i64, width: usize| -> String {
			let s = n.unsigned_abs().to_string();
			let body = match get("padding").unwrap_or("zero") {
				"none"	=> s,
				"space"	=> fmt!("{:>w$}", s, w = width),
				_		=> fmt!("{:0>w$}", s, w = width),
			};
			if n < 0 { fmt!("-{}", body) } else { body }
		};
		let date = match (dt.year, dt.month, dt.day) {
			(Some(y), Some(m), Some(d))	=> Some((y as i64, m as u32, d as u32)),
			_							=> None,
		};
		let time = match (dt.hour, dt.minute, dt.second) {
			(Some(h), Some(m), Some(s))	=> Some((h as u32, m as u32, s as u32)),
			_							=> None,
		};
		let text = match name {
			"day"		=> pad((match date { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) }).2 as i64, 2),
			"month"		=> {
				let m = (match date { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) }).1;
				match get("repr").unwrap_or("numerical") {
					"long"	=> MONTHS[(m - 1) as usize].to_string(),
					"short"	=> MONTHS[(m - 1) as usize][..3].to_string(),
					_		=> pad(m as i64, 2),
				}
			}
			"ordinal"	=> {
				let (y, m, d) = match date { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) };
				pad(ordinal(y, m, d) as i64, 3)
			}
			"weekday"	=> {
				let (y, m, d) = match date { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) };
				let wd = weekday(y, m, d);
				let one = get("one_indexed").unwrap_or("true") != "false";
				match get("repr").unwrap_or("long") {
					"short"		=> WEEKDAYS[(wd - 1) as usize][..3].to_string(),
					"sunday"	=> ((wd % 7) + if one { 1 } else { 0 }).to_string(),
					"monday"	=> ((wd - 1) + if one { 1 } else { 0 }).to_string(),
					_			=> WEEKDAYS[(wd - 1) as usize].to_string(),
				}
			}
			"week_number" => {
				let (y, m, d) = match date { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) };
				let ord = ordinal(y, m, d) as i64;
				let wd = weekday(y, m, d) as i64;
				let w = match get("repr").unwrap_or("iso") {
					"sunday"	=> (ord + 6 - (wd % 7)) / 7,
					"monday"	=> (ord + 6 - (wd - 1)) / 7,
					_			=> iso_week(y, m, d).1 as i64,
				};
				pad(w, 2)
			}
			"year"		=> {
				let (y0, m, d) = match date { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) };
				let y = if get("base") == Some("iso_week") { iso_week(y0, m, d).0 } else { y0 };
				let body = match get("repr").unwrap_or("full") {
					"last_two"	=> pad(y.rem_euclid(100), 2),
					"century"	=> pad(y.div_euclid(100), 2),
					_			=> pad(y, 4),
				};
				if get("sign") == Some("mandatory") && y >= 0 { fmt!("+{}", body) } else { body }
			}
			"hour"		=> {
				let h = (match time { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) }).0;
				if get("repr") == Some("12") {
					let h12 = if h % 12 == 0 { 12 } else { h % 12 };
					pad(h12 as i64, 2)
				} else {
					pad(h as i64, 2)
				}
			}
			"minute"	=> pad((match time { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) }).1 as i64, 2),
			"second"	=> pad((match time { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) }).2 as i64, 2),
			"period"	=> {
				let h = (match time { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) }).0;
				let p = if h < 12 { "AM" } else { "PM" };
				if get("case") == Some("lower") { p.to_lowercase() } else { p.to_string() }
			}
			"subsecond"	=> {
				(match time { Some(x) => x, None => return Err(INSUFFICIENT.to_string()) });
				let digits: usize = get("digits").and_then(|d| d.parse().ok()).unwrap_or(1);
				"0".repeat(digits.max(1))
			}
			"offset_hour" | "offset_minute" | "offset_second" | "unix_timestamp" => return Err(INSUFFICIENT.to_string()),
			"ignore" | "end" => String::new(),
			other => return Err(fmt!("invalid component name '{}' at index {}", other, open + 1)),
		};
		out.push_str(&text);
		i += chars[i..].iter().position(|(_, x)| *x == ']').unwrap_or(0) + 1;
	}
	Ok(out)
}
