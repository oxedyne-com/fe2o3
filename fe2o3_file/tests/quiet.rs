//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use oxedyne_fe2o3_file::quiet::{
    Quiet,
    Stillness,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};

use std::time::{
    Duration,
    Instant,
};


pub fn test_quiet(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["The first poll is always a move 000", "all", "quiet"], || {
        let mut q: Quiet<u64> = Quiet::new(Duration::from_secs(15));
        let t0 = Instant::now();
        assert_eq!(q.poll(1, t0, Duration::from_secs(5)), Stillness::Moved);
        assert_eq!(q.reading(), Some(&1));
        Ok(())
    }));

    res!(test_it(filter, &["An unchanged reading settles then reports still 000", "all", "quiet"], || {
        let quiet    = Duration::from_secs(15);
        let poll     = Duration::from_secs(5);
        let mut q: Quiet<u64> = Quiet::new(quiet);
        let t0 = Instant::now();
        assert_eq!(q.poll(7, t0, poll), Stillness::Moved);
        // Two more polls, unchanged, still short of the 15s window.
        match q.poll(7, t0 + Duration::from_secs(5), poll) {
            Stillness::Settling(left) => assert_eq!(left, Duration::from_secs(10)),
            other => return Err(err!("Expected Settling(10s), got {:?}.", other; Test)),
        }
        match q.poll(7, t0 + Duration::from_secs(10), poll) {
            Stillness::Settling(left) => assert_eq!(left, Duration::from_secs(5)),
            other => return Err(err!("Expected Settling(5s), got {:?}.", other; Test)),
        }
        // Exactly at the window: still.
        assert_eq!(q.poll(7, t0 + Duration::from_secs(15), poll), Stillness::Still);
        // And it stays still while nothing changes, polled at the ordinary cadence.
        assert_eq!(q.poll(7, t0 + Duration::from_secs(20), poll), Stillness::Still);
        Ok(())
    }));

    res!(test_it(filter, &["A change resets the window 000", "all", "quiet"], || {
        let quiet = Duration::from_secs(15);
        let poll  = Duration::from_secs(5);
        let mut q: Quiet<u64> = Quiet::new(quiet);
        let t0 = Instant::now();
        assert_eq!(q.poll(1, t0, poll), Stillness::Moved);
        assert_eq!(q.poll(1, t0 + Duration::from_secs(15), poll), Stillness::Still);
        // The reading changes: back to Moved, and the window starts over.
        assert_eq!(q.poll(2, t0 + Duration::from_secs(16), poll), Stillness::Moved);
        assert_eq!(q.poll(2, t0 + Duration::from_secs(20), poll), Stillness::Settling(Duration::from_secs(11)));
        assert_eq!(q.poll(2, t0 + Duration::from_secs(31), poll), Stillness::Still);
        Ok(())
    }));

    // The control for the bug `ore-quiet-mark`'s bash loop had: "last seen" was stamped on only
    // some code paths, so ten of the loop's own regular polls could look like a suspend gap and
    // restart the window. `Quiet::poll` stamps it unconditionally, on every call.
    res!(test_it(filter, &["Ten regular polls never trip the gap guard 000", "all", "quiet"], || {
        let quiet = Duration::from_secs(15);
        let poll  = Duration::from_secs(5);
        let mut q: Quiet<u64> = Quiet::new(quiet); // default gap_factor = 3, so the guard is 15s.
        let t0 = Instant::now();
        assert_eq!(q.poll(9, t0, poll), Stillness::Moved);
        for i in 1..=10u64 {
            let now = t0 + poll * (i as u32);
            let got = q.poll(9, now, poll);
            assert!(
                !matches!(got, Stillness::Restarted),
                "poll {} restarted on an ordinary 5s cadence: {:?}", i, got,
            );
        }
        // 10 polls at 5s = 50s, well past the 15s window: it must have gone, and stayed, still.
        assert_eq!(q.poll(9, t0 + poll * 11, poll), Stillness::Still);
        Ok(())
    }));

    res!(test_it(filter, &["A gap past the guard restarts the window 000", "all", "quiet"], || {
        let quiet = Duration::from_secs(15);
        let poll  = Duration::from_secs(5);
        let mut q: Quiet<u64> = Quiet::new(quiet); // guard = 3 * 5s = 15s.
        let t0 = Instant::now();
        assert_eq!(q.poll(3, t0, poll), Stillness::Moved);
        // A gap of exactly the guard is not "more than" it: no restart, reading unchanged.
        match q.poll(3, t0 + Duration::from_secs(15), poll) {
            Stillness::Still => {},
            other => return Err(err!("A gap equal to the guard must not restart, got {:?}.", other; Test)),
        }
        // Now a real suspend: the machine sleeps for a minute between two polls. Even though the
        // reading never changed, waking is not an hour of stillness.
        assert_eq!(
            q.poll(3, t0 + Duration::from_secs(15) + Duration::from_secs(61), poll),
            Stillness::Restarted,
        );
        // Immediately after the restart the window has begun again, not finished.
        match q.poll(3, t0 + Duration::from_secs(15) + Duration::from_secs(62), poll) {
            Stillness::Settling(_) => {},
            other => return Err(err!("Expected Settling just after a restart, got {:?}.", other; Test)),
        }
        Ok(())
    }));

    res!(test_it(filter, &["The gap factor is configurable 000", "all", "quiet"], || {
        let mut q: Quiet<u64> = Quiet::with_gap_factor(Duration::from_secs(15), 10);
        let poll = Duration::from_secs(1);
        let t0 = Instant::now();
        assert_eq!(q.poll(1, t0, poll), Stillness::Moved);
        // 9s gap: under a 10x guard on a 1s poll, so no restart.
        match q.poll(1, t0 + Duration::from_secs(9), poll) {
            Stillness::Restarted => return Err(err!("A 9s gap under a 10s guard restarted."; Test)),
            _ => {},
        }
        // 11s gap: over the guard.
        assert_eq!(q.poll(1, t0 + Duration::from_secs(20), poll), Stillness::Restarted);
        Ok(())
    }));

    Ok(())
}
