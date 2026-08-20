//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedyne_fe2o3_datime::{
    calendar::CalendarDate,
    clock::ClockTime,
    core::Duration,
    time::{
        CalClock,
        CalClockConverter,
        CalClockDuration,
        CalClockZone,
    },
};

pub fn test_time(filter: &str) -> Outcome<()> {
    
    res!(test_it(filter, &["calclock_creation", "all", "time", "calclock"], || {
        let zone = res!(CalClockZone::new("UTC"));
        
        // Create from components
        let cc1 = res!(CalClock::new(2024, 3, 15, 14, 30, 0, 0, zone.clone()));
        assert_eq!(cc1.date().year(), 2024);
        assert_eq!(cc1.date().month(), 3);
        assert_eq!(cc1.date().day(), 15);
        assert_eq!(cc1.time().hour().of(), 14);
        assert_eq!(cc1.time().minute().of(), 30);
        
        // Create from date and time
        let date = res!(CalendarDate::new(2024, 3, 15, zone.clone()));
        let time = res!(ClockTime::new(14, 30, 0, 0, zone.clone()));
        let cc2 = res!(CalClock::from_date_time(date, time));
        assert_eq!(cc1, cc2);
        Ok(())
    }));
    
    res!(test_it(filter, &["timezone_support", "all", "time", "timezone"], || {
        // Test UTC
        let utc = res!(CalClockZone::new("UTC"));
        assert_eq!(utc.id(), "UTC");
        assert_eq!(res!(utc.offset_millis_at_time(0)), 0);
        
        // Test fixed offset (without colon - parser expects HHMM format)
        let plus5 = res!(CalClockZone::new("+0500"));
        assert_eq!(res!(plus5.offset_millis_at_time(0)), 5 * 60 * 60 * 1000);
        
        let minus8 = res!(CalClockZone::new("-0800"));
        assert_eq!(res!(minus8.offset_millis_at_time(0)), -8 * 60 * 60 * 1000);
        
        // Test named timezones
        let nyc = res!(CalClockZone::new("America/New_York"));
        assert_eq!(nyc.id(), "America/New_York");
        // Offset varies with DST
        Ok(())
    }));
    
    res!(test_it(filter, &["calclock_arithmetic", "all", "time", "calclock", "arithmetic"], || {
        let zone = res!(CalClockZone::new("UTC"));
        let cc1 = res!(CalClock::new(2024, 3, 15, 14, 30, 0, 0, zone));
        
        // Add duration - 1 hour only to avoid day rollover complexity
        let dur = CalClockDuration::from_hours(1);
        let cc2 = res!(cc1.add_duration(&dur));
        assert_eq!(cc2.date().day(), 15); // Same day
        assert_eq!(cc2.time().hour().of(), 15); // 14 + 1 = 15
        
        // Add days
        let cc3 = res!(cc1.add_days(10));
        assert_eq!(cc3.date().day(), 25);
        
        // Add months
        let cc4 = res!(cc1.add_months(1));
        assert_eq!(cc4.date().month(), 4);
        
        // Duration between
        let duration = res!(cc1.duration_until(&cc2));
        assert_eq!(res!(duration.to_hours()), 1);
        Ok(())
    }));
    
    res!(test_it(filter, &["converter_basic", "all", "time", "converter"], || {
        let zone = res!(CalClockZone::new("UTC"));
        let converter = CalClockConverter::new(zone.clone());
        
        // Just test that conversion produces a number
        let cc = res!(CalClock::new(2024, 3, 15, 14, 30, 0, 0, zone.clone()));
        let millis = res!(converter.calclock_to_unix(&cc));
        
        // Should be a reasonable Unix timestamp (after year 2000)
        assert!(millis > 946684800000); // Jan 1, 2000 in millis
        assert!(millis < 4102444800000); // Jan 1, 2100 in millis
        Ok(())
    }));
    
    res!(test_it(filter, &["converter_optimization", "all", "time", "converter", "optimization"], || {
        let zone = res!(CalClockZone::new("UTC"));
        let mut converter = CalClockConverter::new(zone.clone());
        converter.set_max_reference_deviation(24 * 60 * 60 * 1000); // 1 day
        
        // Just test that we can convert multiple times
        let cc1 = res!(CalClock::new(2024, 3, 15, 10, 0, 0, 0, zone.clone()));
        let cc2 = res!(CalClock::new(2024, 3, 15, 11, 0, 0, 0, zone.clone()));
        
        let millis1 = res!(converter.calclock_to_unix(&cc1));
        let millis2 = res!(converter.calclock_to_unix(&cc2));
        
        // Second time should be 1 hour later
        assert_eq!(millis2 - millis1, 60 * 60 * 1000); // 1 hour in millis
        Ok(())
    }));
    
    res!(test_it(filter, &["calclock_comparison", "all", "time", "calclock", "comparison"], || {
        let zone = res!(CalClockZone::new("UTC"));
        
        let cc1 = res!(CalClock::new(2024, 3, 15, 14, 30, 0, 0, zone.clone()));
        let cc2 = res!(CalClock::new(2024, 3, 15, 14, 30, 0, 1, zone.clone()));
        let cc3 = res!(CalClock::new(2024, 3, 15, 14, 30, 0, 0, zone.clone()));
        
        assert!(cc1 < cc2);
        assert!(cc2 > cc1);
        assert_eq!(cc1, cc3);
        
        assert!(cc1.is_before(&cc2));
        assert!(cc2.is_after(&cc1));
        assert!(!cc1.is_before(&cc3));
        Ok(())
    }));
    
    res!(test_it(filter, &["calclock_formatting", "all", "time", "calclock", "format"], || {
        let zone = res!(CalClockZone::new("UTC"));
        let cc = res!(CalClock::new(2024, 3, 15, 14, 30, 45, 123_456_789, zone));
        
        // ISO format
        let iso = res!(cc.to_iso8601());
        assert!(iso.contains("2024-03-15"));
        assert!(iso.contains("14:30:45"));
        
        // String representation
        let s = cc.to_string();
        assert!(s.contains("2024"));
        assert!(s.contains("14:30"));
        Ok(())
    }));
    
    res!(test_it(filter, &["duration_operations", "all", "time", "duration"], || {
        // Test various duration creations
        let d1 = CalClockDuration::from_seconds(90);
        assert_eq!(res!(d1.to_seconds()), 90);
        
        let d2 = CalClockDuration::from_minutes(5);
        assert_eq!(res!(d2.to_minutes()), 5);
        assert_eq!(res!(d2.to_seconds()), 300);
        
        let d3 = CalClockDuration::from_hours(2);
        assert_eq!(res!(d3.to_hours()), 2);
        assert_eq!(res!(d3.to_minutes()), 120);
        
        // Test arithmetic
        let sum = res!(d1.add(&d2));
        assert_eq!(res!(sum.to_seconds()), 390);
        
        let diff = res!(d3.subtract(&d2));
        assert_eq!(res!(diff.to_minutes()), 115);
        Ok(())
    }));
    
    res!(test_it(filter, &["local_zone_oracle", "all", "time", "zone"], || {
        // The system's own date command is the oracle. Only run where a
        // zoneinfo tree exists; a container without one detects nothing and
        // that is the documented fallback, not a fault.
        if !std::path::Path::new("/usr/share/zoneinfo").is_dir() {
            return Ok(());
        }
        let now_ms = res!(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| err!("{}", e; System))).as_millis() as i64;

        // local() must agree with `date +%z` on the current offset.
        let out = res!(std::process::Command::new("date").arg("+%z").output(),
            System);
        let text = String::from_utf8_lossy(&out.stdout);
        let want = res!(parse_offset_minutes(text.trim()));
        let zone = CalClockZone::local();
        let got = i64::from(res!(zone.offset_millis_at_time(now_ms))) / 60_000;
        assert_eq!(got, want,
            "local() answers {} minutes, the date command says {}.", got, want);

        // A named zone absent from the embedded table must resolve through
        // the system tree rather than silently answering zero.
        for name in ["Australia/Perth", "Australia/Sydney"] {
            if !std::path::Path::new("/usr/share/zoneinfo").join(name).is_file() {
                continue;
            }
            let out = res!(std::process::Command::new("date")
                .env("TZ", name)
                .arg("+%z")
                .output(), System);
            let text = String::from_utf8_lossy(&out.stdout);
            let want = res!(parse_offset_minutes(text.trim()));
            let zone = res!(CalClockZone::new(name));
            let got = i64::from(res!(zone.offset_millis_at_time(now_ms))) / 60_000;
            assert_eq!(got, want,
                "{} answers {} minutes, the date command says {}.",
                name, got, want);
        }
        Ok(())
    }));

    res!(test_it(filter, &["calclock_to_nanos_monotonic", "all", "time", "calclock"], || {
        // `to_millis` already truncates the sub-second field to whole
        // milliseconds, so the old `to_nanos` added those milliseconds a second
        // time.  The overcount grows with the sub-second field and resets when
        // the second rolls over, which makes the sequence run backwards at every
        // second boundary.  Ordering is built on this number, so `is_before`,
        // `is_after` and `PartialOrd` all answered wrongly there.
        let zone = res!(CalClockZone::new("UTC"));
        let steps = [
            (15u8, 999_999_998u32),
            (15,    999_999_999),
            (16,    0),
            (16,    1),
        ];
        let mut prev: Option<i64> = None;
        for (sec, nanos) in steps {
            let cc = res!(CalClock::new(2024, 3, 15, 14, 30, sec, nanos, zone.clone()));
            let now = res!(cc.to_nanos());
            if let Some(before) = prev {
                assert!(now > before,
                    "14:30:{:02}.{:09} answers {} ns, which is not after the {} ns \
                    of the instant before it.", sec, nanos, now, before);
            }
            prev = Some(now);
        }

        // The boundary itself: one nanosecond apart, and in that order.
        let last  = res!(CalClock::new(2024, 3, 15, 14, 30, 15, 999_999_999, zone.clone()));
        let first = res!(CalClock::new(2024, 3, 15, 14, 30, 16, 0, zone.clone()));
        assert_eq!(res!(first.to_nanos()) - res!(last.to_nanos()), 1,
            "The second boundary is not one nanosecond wide.");
        assert!(last.is_before(&first), "is_before misreads the second boundary.");
        assert!(first.is_after(&last), "is_after misreads the second boundary.");
        assert!(last < first, "PartialOrd misreads the second boundary.");
        let gap = res!(last.duration_until(&first));
        assert_eq!(res!(gap.to_nanos()), 1,
            "duration_until misreads the second boundary.");
        Ok(())
    }));

    res!(test_it(filter, &["calclock_to_nanos_known_value", "all", "time", "calclock"], || {
        // Derived by hand, not from the method under test.
        //
        // Days from 1970-01-01 to 2024-01-01: 54 years of 365 days, plus one
        // day for each of the 13 leap years 1972, 1976, ... 2020, so
        //   54 * 365 + 13 = 19710 + 13 = 19723 days.
        // 2024 is a leap year, so 2024-03-15 is a further 31 + 29 + 14 = 74
        // days on, giving 19797 days.  19797 * 86400 = 1_710_460_800 seconds,
        // which is the published Unix time of 2024-03-15T00:00:00Z.
        // 14:30:15 is 14*3600 + 30*60 + 15 = 52_215 seconds into the day, so
        // the instant is 1_710_513_015 seconds after the epoch, and
        //   1_710_513_015 * 1e9 + 123_456_789 nanoseconds.
        const WANT: i64 = 1_710_513_015_123_456_789;

        let zone = res!(CalClockZone::new("UTC"));
        let cc = res!(CalClock::new(2024, 3, 15, 14, 30, 15, 123_456_789, zone.clone()));
        let got = res!(cc.to_nanos());
        assert_eq!(got, WANT, "to_nanos is out by {} ns.", got - WANT);
        assert_eq!(res!(cc.to_nanos_since_epoch()), WANT,
            "to_nanos_since_epoch disagrees with to_nanos.");
        assert_eq!(res!(cc.to_millis()), 1_710_513_015_123,
            "to_millis disagrees with the hand-worked value.");

        // The last nanosecond before the epoch, where the whole thing is negative.
        let before = res!(CalClock::new(1969, 12, 31, 23, 59, 59, 999_999_999, zone.clone()));
        assert_eq!(res!(before.to_nanos()), -1,
            "The nanosecond before the epoch is not -1.");

        // `from_nanos` is the inverse.  It used to overwrite the whole sub-second
        // field with the sub-millisecond remainder, throwing the milliseconds away.
        for want in [WANT, -1i64, 0i64] {
            let back = res!(CalClock::from_nanos(want, zone.clone()));
            let round = res!(back.to_nanos());
            assert_eq!(round, want,
                "{} does not survive from_nanos then to_nanos; it came back as {}.",
                want, round);
        }
        let back = res!(CalClock::from_nanos_since_epoch(WANT, zone.clone()));
        assert_eq!(back.nanosecond(), 123_456_789,
            "from_nanos_since_epoch lost the millisecond part of the sub-second field.");
        assert_eq!(back.second(), 15, "from_nanos_since_epoch landed on the wrong second.");
        Ok(())
    }));

    Ok(())
}

/// Reads a `+0800`-style offset as minutes.
fn parse_offset_minutes(text: &str) -> Outcome<i64> {
    if text.len() < 5 {
        return Err(err!("'{}' is not a +hhmm offset.", text; Invalid, Input));
    }
    let sign = match &text[..1] {
        "+" => 1i64,
        "-" => -1i64,
        _ => return Err(err!("'{}' is not a +hhmm offset.", text; Invalid, Input)),
    };
    let hours: i64 = res!(text[1..3].parse().map_err(|_|
        err!("'{}' is not a +hhmm offset.", text; Invalid, Input)));
    let minutes: i64 = res!(text[3..5].parse().map_err(|_|
        err!("'{}' is not a +hhmm offset.", text; Invalid, Input)));
    Ok(sign * (hours * 60 + minutes))
}
