//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    calendar::CalendarDate,
    time::CalClockZone,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

/// RFC 8536, versions 1 to 3.
pub struct TZifParser {
    data: Vec<u8>,
    timezone_data: Option<TZifData>,
}

#[derive(Clone, Debug)]
pub struct TZifData {
    pub version: u8, // 1, 2 or 3
    pub transition_times: Vec<i64>, // UTC seconds since the Unix epoch
    pub transition_types: Vec<u8>, // indices into local_time_types
    pub local_time_types: Vec<LocalTimeType>,
    pub abbreviations: String,
    pub leap_seconds: Vec<LeapSecond>,
    pub standard_wall_indicators: Vec<bool>,
    pub ut_local_indicators: Vec<bool>,
    pub posix_tz_string: Option<String>, // version 2 and later
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalTimeType {
    pub utc_offset: i32, // seconds east of UTC
    pub is_dst: bool,
    pub abbreviation_index: usize, // into abbreviations
}

#[derive(Clone, Debug, PartialEq)]
pub struct LeapSecond {
    pub transition_time: i64, // UTC seconds since the Unix epoch
    pub correction: i32, // cumulative, not the step
}

/// A local time may be reached twice or not at all across a DST transition.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalTimeResult<T> {
    Single(T),
    Ambiguous(T, T), // the autumn fold: the earlier instant first, then the later
    None, // the spring gap, where the local time never occurs
}

/// What the zone's rules say about one instant: the offset, and whether it is
/// a summer one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneOffset {
    pub utc_offset: i32, // seconds east of UTC
    pub is_dst: bool,
}

impl TZifParser {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            timezone_data: None,
        }
    }

    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> Outcome<()> {
        self.data = res!(fs::read(path.as_ref()).map_err(|e| 
            err!("Failed to read TZif file {:?}: {}", path.as_ref(), e; IO, File)));
        self.parse()
    }

    pub fn load_from_bytes(&mut self, data: &[u8]) -> Outcome<()> {
        self.data = data.to_vec();
        self.parse()
    }

    pub fn timezone_data(&self) -> Option<&TZifData> {
        self.timezone_data.as_ref()
    }

    fn parse(&mut self) -> Outcome<()> {
        if self.data.len() < 44 {
            return Err(err!("TZif file too short: {} bytes", self.data.len(); Invalid, Input));
        }

        let mut cursor = Cursor::new(&self.data);
        let header = res!(self.parse_header(&mut cursor));
        
        // Parse version 1 data first (required for all versions)
        let v1_data = res!(self.parse_data_block(&mut cursor, &header, false));
        
        // For version 2+ files, parse the second data block with 64-bit timestamps
        let timezone_data = if header.version >= 2 {
            // Skip version 1 data and parse version 2+ header and data
            let v2_header = res!(self.parse_header(&mut cursor));
            let v2_data = res!(self.parse_data_block(&mut cursor, &v2_header, true));
            
            // Parse POSIX TZ string footer
            let posix_tz_string = res!(self.parse_posix_footer(&mut cursor));
            
            TZifData {
                version: header.version,
                posix_tz_string: Some(posix_tz_string),
                ..v2_data
            }
        } else {
            TZifData {
                version: header.version,
                posix_tz_string: None,
                ..v1_data
            }
        };

        self.timezone_data = Some(timezone_data);
        Ok(())
    }

    fn parse_header(&self, cursor: &mut Cursor<&Vec<u8>>) -> Outcome<TZifHeader> {
        let mut magic = [0u8; 4];
        res!(cursor.read_exact(&mut magic).map_err(|e| 
            err!("Failed to read magic number: {}", e; IO)));
        
        if &magic != b"TZif" {
            return Err(err!("Invalid TZif magic number: {:?}", magic; Invalid, Input));
        }

        let mut version_byte = [0u8; 1];
        res!(cursor.read_exact(&mut version_byte).map_err(|e| 
            err!("Failed to read version: {}", e; IO)));
        
        let version = match version_byte[0] {
            0 => 1,
            b'2' => 2,
            b'3' => 3,
            v => return Err(err!("Unsupported TZif version: {}", v; Invalid, Input)),
        };

        // Skip reserved bytes (15 bytes)
        let mut reserved = [0u8; 15];
        res!(cursor.read_exact(&mut reserved).map_err(|e| 
            err!("Failed to read reserved bytes: {}", e; IO)));

        // Read counts (6 * 4 bytes = 24 bytes)
        let tzh_utcnt = res!(read_u32_be(cursor));     // UT/local indicators
        let tzh_stdcnt = res!(read_u32_be(cursor));    // standard/wall indicators
        let tzh_leapcnt = res!(read_u32_be(cursor));   // leap second records
        let tzh_timecnt = res!(read_u32_be(cursor));   // transition times
        let tzh_typecnt = res!(read_u32_be(cursor));   // local time types
        let tzh_charcnt = res!(read_u32_be(cursor));   // abbreviation characters

        Ok(TZifHeader {
            version,
            tzh_utcnt,
            tzh_stdcnt,
            tzh_leapcnt,
            tzh_timecnt,
            tzh_typecnt,
            tzh_charcnt,
        })
    }

    fn parse_data_block(&self, cursor: &mut Cursor<&Vec<u8>>, header: &TZifHeader, is_64bit: bool) -> Outcome<TZifData> {
        // Parse transition times
        let mut transition_times = Vec::with_capacity(header.tzh_timecnt as usize);
        for _ in 0..header.tzh_timecnt {
            let time = if is_64bit {
                res!(read_i64_be(cursor))
            } else {
                res!(read_i32_be(cursor)) as i64
            };
            transition_times.push(time);
        }

        // Parse transition types
        let mut transition_types = Vec::with_capacity(header.tzh_timecnt as usize);
        for _ in 0..header.tzh_timecnt {
            let mut byte = [0u8; 1];
            res!(cursor.read_exact(&mut byte).map_err(|e| 
                err!("Failed to read transition type: {}", e; IO)));
            transition_types.push(byte[0]);
        }

        // Parse local time types
        let mut local_time_types = Vec::with_capacity(header.tzh_typecnt as usize);
        for _ in 0..header.tzh_typecnt {
            let utc_offset = res!(read_i32_be(cursor));
            
            let mut is_dst_byte = [0u8; 1];
            res!(cursor.read_exact(&mut is_dst_byte).map_err(|e| 
                err!("Failed to read DST flag: {}", e; IO)));
            let is_dst = is_dst_byte[0] != 0;
            
            let mut abbrev_index_byte = [0u8; 1];
            res!(cursor.read_exact(&mut abbrev_index_byte).map_err(|e| 
                err!("Failed to read abbreviation index: {}", e; IO)));
            let abbreviation_index = abbrev_index_byte[0] as usize;
            
            local_time_types.push(LocalTimeType {
                utc_offset,
                is_dst,
                abbreviation_index,
            });
        }

        // Parse abbreviations
        let mut abbrev_data = vec![0u8; header.tzh_charcnt as usize];
        res!(cursor.read_exact(&mut abbrev_data).map_err(|e| 
            err!("Failed to read abbreviations: {}", e; IO)));
        let abbreviations = String::from_utf8_lossy(&abbrev_data).to_string();

        // Parse leap seconds
        let mut leap_seconds = Vec::with_capacity(header.tzh_leapcnt as usize);
        for _ in 0..header.tzh_leapcnt {
            let transition_time = if is_64bit {
                res!(read_i64_be(cursor))
            } else {
                res!(read_i32_be(cursor)) as i64
            };
            let correction = res!(read_i32_be(cursor));
            leap_seconds.push(LeapSecond { transition_time, correction });
        }

        // Parse standard/wall indicators
        let mut standard_wall_indicators = Vec::with_capacity(header.tzh_stdcnt as usize);
        for _ in 0..header.tzh_stdcnt {
            let mut byte = [0u8; 1];
            res!(cursor.read_exact(&mut byte).map_err(|e| 
                err!("Failed to read standard/wall indicator: {}", e; IO)));
            standard_wall_indicators.push(byte[0] != 0);
        }

        // Parse UT/local indicators
        let mut ut_local_indicators = Vec::with_capacity(header.tzh_utcnt as usize);
        for _ in 0..header.tzh_utcnt {
            let mut byte = [0u8; 1];
            res!(cursor.read_exact(&mut byte).map_err(|e| 
                err!("Failed to read UT/local indicator: {}", e; IO)));
            ut_local_indicators.push(byte[0] != 0);
        }

        Ok(TZifData {
            version: header.version,
            transition_times,
            transition_types,
            local_time_types,
            abbreviations,
            leap_seconds,
            standard_wall_indicators,
            ut_local_indicators,
            posix_tz_string: None,
        })
    }

    fn parse_posix_footer(&self, cursor: &mut Cursor<&Vec<u8>>) -> Outcome<String> {
        // Skip newline
        let mut newline = [0u8; 1];
        res!(cursor.read_exact(&mut newline).map_err(|e| 
            err!("Failed to read newline before POSIX string: {}", e; IO)));
        
        if newline[0] != b'\n' {
            return Err(err!("Expected newline before POSIX string, got: {}", newline[0]; Invalid, Input));
        }

        // Read until final newline
        let mut posix_data = Vec::new();
        let mut byte = [0u8; 1];
        
        loop {
            match cursor.read_exact(&mut byte) {
                Ok(()) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    posix_data.push(byte[0]);
                },
                Err(_) => break, // EOF
            }
        }

        Ok(String::from_utf8_lossy(&posix_data).to_string())
    }
}

#[derive(Debug)]
struct TZifHeader {
    version: u8,
    tzh_utcnt: u32,    // UT/local indicators count
    tzh_stdcnt: u32,   // standard/wall indicators count  
    tzh_leapcnt: u32,  // leap second records count
    tzh_timecnt: u32,  // transition times count
    tzh_typecnt: u32,  // local time types count
    tzh_charcnt: u32,  // abbreviation characters count
}

impl TZifData {
    pub fn get_abbreviation(&self, local_time_type: &LocalTimeType) -> Outcome<&str> {
        if local_time_type.abbreviation_index >= self.abbreviations.len() {
            return Err(err!(
                "Abbreviation index {} out of bounds (len: {})", 
                local_time_type.abbreviation_index, self.abbreviations.len(); 
                Invalid, Input
            ));
        }

        let abbrev_start = local_time_type.abbreviation_index;
        let abbrev_end = self.abbreviations[abbrev_start..]
            .find('\0')
            .map(|pos| abbrev_start + pos)
            .unwrap_or(self.abbreviations.len());

        Ok(&self.abbreviations[abbrev_start..abbrev_end])
    }

    /// Data that is nothing but a POSIX TZ rule such as `AEST-10AEDT,M10.1.0,M4.1.0/3`,
    /// the form libc reads `TZ` in when no zoneinfo file answers to it. With no
    /// transitions the rule alone answers, as a footer does past the end of its table.
    pub fn from_posix_rule(rule: &str) -> Outcome<Self> {
        let zone = res!(PosixZone::parse(rule));
        let mut abbreviations = format!("{}\0", zone.std_name);
        let mut local_time_types = vec![
            LocalTimeType { utc_offset: zone.std, is_dst: false, abbreviation_index: 0 },
        ];
        if let Some(dst) = &zone.dst {
            local_time_types.push(LocalTimeType {
                utc_offset:         dst.off,
                is_dst:             true,
                abbreviation_index: abbreviations.len(),
            });
            abbreviations.push_str(&dst.name);
            abbreviations.push('\0');
        }
        Ok(Self {
            version:                    3,
            transition_times:           Vec::new(),
            transition_types:           Vec::new(),
            local_time_types,
            abbreviations,
            leap_seconds:               Vec::new(),
            standard_wall_indicators:   Vec::new(),
            ut_local_indicators:        Vec::new(),
            posix_tz_string:            Some(rule.to_string()),
        })
    }

    /// The offset in force at a UTC instant. Before the first transition the
    /// first local time type applies (RFC 8536, section 3.2). At or after the
    /// last, the footer's POSIX rule does where the file has one: a "slim" file
    /// ends its table where the rules stop changing and leaves the years after
    /// to that string, and a "fat" one only runs out of table in 2037.
    pub fn offset_at(&self, utc: i64) -> Outcome<ZoneOffset> {
        let n = self.transition_times.len().min(self.transition_types.len());
        let after = self.transition_times[..n].partition_point(|&t| t <= utc);
        if after == n {
            if let Some(text) = self.posix_tz_string.as_deref().filter(|t| !t.is_empty()) {
                return res!(PosixZone::parse(text)).offset_at(utc);
            }
        }
        let local_type = if after == 0 {
            self.local_time_types.first()
        } else {
            self.local_time_types.get(self.transition_types[after - 1] as usize)
        };
        match local_type {
            Some(t) => Ok(ZoneOffset { utc_offset: t.utc_offset, is_dst: t.is_dst }),
            None => Err(err!(
                "The TZif data has no local time type for UTC timestamp {}", utc;
                Invalid, Input, Missing)),
        }
    }

    pub fn utc_to_local(&self, utc_timestamp: i64) -> LocalTimeResult<(i64, ZoneOffset)> {
        match self.offset_at(utc_timestamp) {
            Ok(z) => LocalTimeResult::Single((utc_timestamp + z.utc_offset as i64, z)),
            Err(_) => LocalTimeResult::None,
        }
    }

    /// Every UTC instant at which the wall clock reads `local_timestamp` (local
    /// seconds, as if UTC), earliest first: none in a spring gap, two across an
    /// autumn fold.
    pub fn local_to_utc(&self, local_timestamp: i64) -> LocalTimeResult<(i64, ZoneOffset)> {
        let found = local_to_utc_by(local_timestamp, 86_400, |u| {
            self.offset_at(u).map(|z| z.utc_offset as i64)
        });
        let with_offset = |u: i64| match self.offset_at(u) {
            Ok(z) => Some((u, z)),
            Err(_) => None,
        };
        match found.as_slice() {
            [] => LocalTimeResult::None,
            [a] => match with_offset(*a) {
                Some(x) => LocalTimeResult::Single(x),
                None => LocalTimeResult::None,
            },
            [a, b, ..] => match (with_offset(*a), with_offset(*b)) {
                (Some(x), Some(y)) => LocalTimeResult::Ambiguous(x, y),
                _ => LocalTimeResult::None,
            },
        }
    }

    pub fn get_offset_at_utc(&self, utc_timestamp: i64) -> Outcome<i32> {
        Ok(res!(self.offset_at(utc_timestamp)).utc_offset)
    }

    pub fn is_dst_at_utc(&self, utc_timestamp: i64) -> Outcome<bool> {
        Ok(res!(self.offset_at(utc_timestamp)).is_dst)
    }
}

/// The UTC instants whose wall clock reads `local`, earliest first, given how
/// to find the offset at a UTC instant. All three quantities share one unit,
/// and `day` is a day in it. A local time stands within 15 hours of the same
/// number read as UTC, so the offsets sampled over two days either side are
/// the only ones that can apply; each is kept when it is the offset actually
/// in force at the instant it implies. Two survivors are the autumn fold and
/// none is the spring gap.
pub(crate) fn local_to_utc_by<F>(local: i64, day: i64, offset_at: F) -> Vec<i64>
where
    F: Fn(i64) -> Outcome<i64>,
{
    let mut tried: Vec<i64> = Vec::new();
    let mut found: Vec<i64> = Vec::new();
    for k in -2..=2 {
        let off = match offset_at(local + k * day) {
            Ok(off) => off,
            Err(_) => continue,
        };
        if tried.contains(&off) {
            continue;
        }
        tried.push(off);
        let utc = local - off;
        if let Ok(back) = offset_at(utc) {
            if back == off {
                found.push(utc);
            }
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

/// The rule a TZif footer gives for the years beyond its table: a POSIX TZ
/// string such as `EST5EDT,M3.2.0,M11.1.0` or `<+1030>-10:30<+11>-11,M10.1.0,M4.1.0`.
#[derive(Clone, Debug, PartialEq)]
struct PosixZone {
    std:      i32, // seconds east of UTC
    std_name: String,
    dst:      Option<PosixDst>,
}

#[derive(Clone, Debug, PartialEq)]
struct PosixDst {
    name:    String,
    off:     i32, // seconds east of UTC
    start:   PosixDay,
    start_t: i32, // seconds after local midnight, which may be negative or exceed a day
    end:     PosixDay,
    end_t:   i32,
}

#[derive(Clone, Debug, PartialEq)]
enum PosixDay {
    Julian1(i64), // Jn: 1 to 365, February 29th never counted
    Julian0(i64), // n: 0 to 365, February 29th counted
    Month(u8, u8, u8), // Mm.w.d: month, week 1 to 5 (5 is the last), day 0 to 6 from Sunday
}

impl PosixZone {
    fn parse(text: &str) -> Outcome<Self> {
        let b = text.as_bytes();
        let mut i = 0;
        let std_name = res!(Self::name(b, &mut i, text));
        let std = -res!(Self::seconds(b, &mut i, text));
        if i == b.len() {
            return Ok(Self { std, std_name, dst: None });
        }
        let dst_name = res!(Self::name(b, &mut i, text));
        let off = if i < b.len() && b[i] != b',' {
            -res!(Self::seconds(b, &mut i, text))
        } else {
            std + 3600
        };
        let (start, start_t, end, end_t) = if i == b.len() {
            // No rule given: the United States default, as the C library has it.
            (PosixDay::Month(3, 2, 0), 7200, PosixDay::Month(11, 1, 0), 7200)
        } else {
            let (start, start_t) = res!(Self::rule(b, &mut i, text));
            let (end, end_t) = res!(Self::rule(b, &mut i, text));
            (start, start_t, end, end_t)
        };
        if i != b.len() {
            return Err(err!(
                "Unread text '{}' after the rule in the TZif footer '{}'.", &text[i..], text;
                Invalid, Input));
        }
        Ok(Self { std, std_name, dst: Some(PosixDst { name: dst_name, off, start, start_t, end, end_t }) })
    }

    // A zone name, letters or angle-bracketed, three characters at least, as the
    // abbreviation it spells (the brackets are not part of it).
    fn name(b: &[u8], i: &mut usize, text: &str) -> Outcome<String> {
        let from = *i;
        if *i < b.len() && b[*i] == b'<' {
            while *i < b.len() && b[*i] != b'>' {
                *i += 1;
            }
            if *i >= b.len() {
                return Err(err!(
                    "A '<' in the TZif footer '{}' has no closing '>'.", text; Invalid, Input));
            }
            *i += 1;
            if *i - from < 5 {
                return Err(err!(
                    "A zone name in the TZif footer '{}' is shorter than three characters.", text;
                    Invalid, Input));
            }
        } else {
            while *i < b.len() && b[*i].is_ascii_alphabetic() {
                *i += 1;
            }
            if *i - from < 3 {
                return Err(err!(
                    "A zone name in the TZif footer '{}' is shorter than three characters.", text;
                    Invalid, Input));
            }
        }
        let spelt = &text[from..*i];
        Ok(spelt.trim_start_matches('<').trim_end_matches('>').to_string())
    }

    // [+-]h[h[h]][:mm[:ss]], in seconds, the sign kept as written.
    fn seconds(b: &[u8], i: &mut usize, text: &str) -> Outcome<i32> {
        let mut sign = 1;
        if *i < b.len() && (b[*i] == b'+' || b[*i] == b'-') {
            if b[*i] == b'-' {
                sign = -1;
            }
            *i += 1;
        }
        let mut total = 0i32;
        let mut scale = 3600;
        loop {
            let from = *i;
            let mut n = 0i32;
            while *i < b.len() && b[*i].is_ascii_digit() && *i - from < 3 {
                n = n * 10 + (b[*i] - b'0') as i32;
                *i += 1;
            }
            if *i == from {
                return Err(err!(
                    "Expected a number at byte {} of the TZif footer '{}'.", from, text;
                    Invalid, Input));
            }
            total += n * scale;
            if scale > 1 && *i < b.len() && b[*i] == b':' {
                *i += 1;
                scale /= 60;
            } else {
                break;
            }
        }
        Ok(sign * total)
    }

    // ,date[/time]
    fn rule(b: &[u8], i: &mut usize, text: &str) -> Outcome<(PosixDay, i32)> {
        if *i >= b.len() || b[*i] != b',' {
            return Err(err!(
                "Expected ',' at byte {} of the TZif footer '{}'.", *i, text; Invalid, Input));
        }
        *i += 1;
        let day = res!(Self::day(b, i, text));
        let t = if *i < b.len() && b[*i] == b'/' {
            *i += 1;
            res!(Self::seconds(b, i, text))
        } else {
            7200
        };
        Ok((day, t))
    }

    fn day(b: &[u8], i: &mut usize, text: &str) -> Outcome<PosixDay> {
        let number = |i: &mut usize| -> Outcome<i64> {
            let from = *i;
            let mut n = 0i64;
            while *i < b.len() && b[*i].is_ascii_digit() && *i - from < 4 {
                n = n * 10 + (b[*i] - b'0') as i64;
                *i += 1;
            }
            if *i == from {
                return Err(err!(
                    "Expected a number at byte {} of the TZif footer '{}'.", from, text;
                    Invalid, Input));
            }
            Ok(n)
        };
        if *i < b.len() && b[*i] == b'M' {
            *i += 1;
            let m = res!(number(i));
            if *i >= b.len() || b[*i] != b'.' {
                return Err(err!("Expected '.' in the TZif footer '{}'.", text; Invalid, Input));
            }
            *i += 1;
            let w = res!(number(i));
            if *i >= b.len() || b[*i] != b'.' {
                return Err(err!("Expected '.' in the TZif footer '{}'.", text; Invalid, Input));
            }
            *i += 1;
            let d = res!(number(i));
            if !(1..=12).contains(&m) || !(1..=5).contains(&w) || !(0..=6).contains(&d) {
                return Err(err!(
                    "The month, week or day in the TZif footer '{}' is out of range.", text;
                    Invalid, Input, Range));
            }
            Ok(PosixDay::Month(m as u8, w as u8, d as u8))
        } else if *i < b.len() && b[*i] == b'J' {
            *i += 1;
            let n = res!(number(i));
            if !(1..=365).contains(&n) {
                return Err(err!(
                    "The Julian day in the TZif footer '{}' is out of range.", text;
                    Invalid, Input, Range));
            }
            Ok(PosixDay::Julian1(n))
        } else {
            let n = res!(number(i));
            if !(0..=365).contains(&n) {
                return Err(err!(
                    "The day of the year in the TZif footer '{}' is out of range.", text;
                    Invalid, Input, Range));
            }
            Ok(PosixDay::Julian0(n))
        }
    }

    fn offset_at(&self, utc: i64) -> Outcome<ZoneOffset> {
        let dst = match &self.dst {
            Some(dst) => dst,
            None => return Ok(ZoneOffset { utc_offset: self.std, is_dst: false }),
        };
        // The year by the standard-time calendar. Each change falls on a date
        // of that year, read in the clock in force just before it.
        let year = res!(CalendarDate::from_days_since_epoch(
            (utc + self.std as i64).div_euclid(86_400),
            CalClockZone::utc(),
        )).year();
        let start = res!(dst.start.day_of(year)) * 86_400 + dst.start_t as i64 - self.std as i64;
        let end = res!(dst.end.day_of(year)) * 86_400 + dst.end_t as i64 - dst.off as i64;
        // The southern hemisphere has its summer across the new year.
        let summer = if start < end {
            utc >= start && utc < end
        } else {
            utc >= start || utc < end
        };
        if summer {
            Ok(ZoneOffset { utc_offset: dst.off, is_dst: true })
        } else {
            Ok(ZoneOffset { utc_offset: self.std, is_dst: false })
        }
    }
}

impl PosixDay {
    // Days since the Unix epoch of the date this rule names in `year`.
    fn day_of(&self, year: i32) -> Outcome<i64> {
        let utc = CalClockZone::utc();
        let jan1 = res!(res!(CalendarDate::new(year, 1, 1, utc.clone())).days_since_epoch());
        match self {
            Self::Julian1(n) => {
                let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                Ok(jan1 + n - 1 + if leap && *n >= 60 { 1 } else { 0 })
            },
            Self::Julian0(n) => Ok(jan1 + n),
            Self::Month(m, w, d) => {
                let first_date = res!(CalendarDate::new(year, *m, 1, utc));
                let first = res!(first_date.days_since_epoch());
                let dim = res!(first_date.days_in_month()) as i64;
                // 1970-01-01 fell on a Thursday, which is day 4 counting from Sunday.
                let dow = (first + 4).rem_euclid(7);
                let mut day = first + (*d as i64 - dow).rem_euclid(7) + (*w as i64 - 1) * 7;
                if day >= first + dim {
                    day -= 7; // week 5 is the last, which may be the fourth
                }
                Ok(day)
            },
        }
    }
}

// Helper functions for reading big-endian values

fn read_u32_be(cursor: &mut Cursor<&Vec<u8>>) -> Outcome<u32> {
    let mut bytes = [0u8; 4];
    res!(cursor.read_exact(&mut bytes).map_err(|e| 
        err!("Failed to read u32: {}", e; IO)));
    Ok(u32::from_be_bytes(bytes))
}

fn read_i32_be(cursor: &mut Cursor<&Vec<u8>>) -> Outcome<i32> {
    let mut bytes = [0u8; 4];
    res!(cursor.read_exact(&mut bytes).map_err(|e| 
        err!("Failed to read i32: {}", e; IO)));
    Ok(i32::from_be_bytes(bytes))
}

fn read_i64_be(cursor: &mut Cursor<&Vec<u8>>) -> Outcome<i64> {
    let mut bytes = [0u8; 8];
    res!(cursor.read_exact(&mut bytes).map_err(|e| 
        err!("Failed to read i64: {}", e; IO)));
    Ok(i64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tzif_header_parsing() {
        // Create minimal valid TZif header
        let mut data = Vec::new();
        // A version 2 file is two headers, the second with its own data
        // block, and the footer's lines; each header has no counts here.
        for _ in 0..2 {
            data.extend_from_slice(b"TZif");  // Magic
            data.push(b'2');                 // Version 2
            data.extend_from_slice(&[0u8; 15]); // Reserved
            data.extend_from_slice(&[0u8; 24]); // 6 * 4 bytes of counts
        }
        data.extend_from_slice(b"\n\n"); // Empty footer
        
        let mut parser = TZifParser::new();
        assert!(parser.load_from_bytes(&data).is_ok());
    }

    #[test]
    fn test_local_time_type() {
        let ltt = LocalTimeType {
            utc_offset: -18000, // EST: -5 hours
            is_dst: false,
            abbreviation_index: 0,
        };
        
        assert_eq!(ltt.utc_offset, -18000);
        assert!(!ltt.is_dst);
    }

    #[test]
    fn test_leap_second() {
        let leap = LeapSecond {
            transition_time: 78796800, // 1972-07-01
            correction: 1,
        };
        
        assert_eq!(leap.transition_time, 78796800);
        assert_eq!(leap.correction, 1);
    }
}