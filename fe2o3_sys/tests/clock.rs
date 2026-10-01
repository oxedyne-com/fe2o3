//! Tests for `fe2o3_sys::clock` against the kernel and against readings a suspend would leave.
//!
//! The kernel is the oracle for the two things it can attest on any machine: the first
//! field of `/proc/uptime` is `CLOCK_BOOTTIME`, and a process stopped by a real `SIGSTOP`
//! (whose state the kernel reports in `/proc/<pid>/status`) is not a suspend, since both
//! clocks run through it. A suspend itself cannot be produced here, and this machine has
//! never had one (its boot and monotonic clocks agree to the microsecond), so what a
//! suspend leaves is given to `Asleep` as readings, 3 s apart, and the arithmetic is
//! checked on those.

#![cfg(all(target_os = "linux", feature = "clock"))]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::clock::{
    since_boot,
    slept,
    Asleep,
    LEAST,
};

use std::{
    fs,
    io::{
        BufRead,
        BufReader,
    },
    process::{
        Child,
        Command,
        Stdio,
    },
    time::{
        Duration,
        Instant,
    },
};

const CHILD:    &str        = "FE2O3_SYS_CLOCK_CHILD";
const SECOND:   Duration    = Duration::from_secs(1);

fn io<T>(r: std::io::Result<T>, what: &str) -> Outcome<T> {
    match r {
        Ok(v)   => Ok(v),
        Err(e)  => Err(err!("{}: {}.", what, e; IO, File)),
    }
}

// The first field of /proc/uptime, in seconds, as the kernel prints it (two decimals).
fn uptime() -> Outcome<f64> {
    let text = res!(io(fs::read_to_string("/proc/uptime"), "read /proc/uptime"));
    let field = res!(text.split_whitespace().next()
        .ok_or_else(|| err!("/proc/uptime is empty."; Missing)));
    match field.parse::<f64>() {
        Ok(v)   => Ok(v),
        Err(e)  => Err(err!("/proc/uptime says {:?}: {}.", field, e; Input, Invalid)),
    }
}

#[test]
fn since_boot_agrees_with_the_kernels_own_uptime() -> Outcome<()> {
    // Read either side of the call. The two reads of /proc/uptime must themselves be
    // close (a loaded machine can stretch them), or the bracket proves nothing.
    for _ in 0..20 {
        let before  = res!(uptime());
        let boot    = res!(since_boot()).as_secs_f64();
        let after   = res!(uptime());
        if after - before > 0.05 {
            continue;
        }
        println!("uptime {} <= since_boot {:.3} <= uptime {}", before, boot, after);
        // /proc/uptime prints two decimals, so each end may be a hundredth short.
        assert!(boot >= before - 0.02,
            "since_boot {} is behind the kernel's uptime {} read before it", boot, before);
        assert!(boot <= after + 0.02,
            "since_boot {} is ahead of the kernel's uptime {} read after it", boot, after);
        return Ok(());
    }
    Err(err!("Twenty tries and /proc/uptime was never read twice within 50 ms."; Test, Timeout))
}

#[test]
fn since_boot_keeps_the_kernels_rate() -> Outcome<()> {
    let up0     = res!(uptime());
    let boot0   = res!(since_boot());
    std::thread::sleep(Duration::from_millis(400));
    let boot1   = res!(since_boot());
    let up1     = res!(uptime());
    let moved   = (boot1 - boot0).as_secs_f64();
    let kernel  = up1 - up0;
    println!("since_boot moved {:.3} s while /proc/uptime moved {:.2} s", moved, kernel);
    assert!((moved - kernel).abs() < 0.03,
        "since_boot moved {} s where /proc/uptime moved {} s", moved, kernel);
    assert!(moved >= 0.4, "since_boot moved {} s over a 400 ms sleep", moved);
    Ok(())
}

#[test]
fn a_stretch_the_boot_clock_counted_and_the_monotonic_clock_did_not_is_a_suspend() -> Outcome<()> {
    // The boot clock ran 5 s and the monotonic clock 2 s: the machine slept for 3.
    let base = Instant::now();
    let mut a = Asleep::at(Duration::from_secs(1_000), base, LEAST);
    assert_eq!(
        a.check_at(Duration::from_secs(1_005), base + Duration::from_secs(2)),
        Some(Duration::from_secs(3)));
    assert_eq!(
        slept(Duration::from_secs(5), Duration::from_secs(2), LEAST),
        Some(Duration::from_secs(3)));
    Ok(())
}

#[test]
fn a_stretch_both_clocks_counted_is_not_a_suspend() -> Outcome<()> {
    // A stalled process, a debugger, a long swap-out: both clocks go on, and agree.
    let base = Instant::now();
    for n in [1, 3, 600, 86_400] {
        let mut a = Asleep::at(Duration::from_secs(1_000), base, LEAST);
        let found = a.check_at(Duration::from_secs(1_000 + n), base + Duration::from_secs(n));
        assert_eq!(found, None, "a {} s stretch on both clocks", n);
    }
    Ok(())
}

#[test]
fn a_gap_is_reported_from_the_threshold_up() -> Outcome<()> {
    let ms = Duration::from_millis;
    assert_eq!(slept(ms(7_999), ms(6_000), LEAST), None, "1.999 s asleep");
    assert_eq!(slept(ms(8_000), ms(6_000), LEAST), Some(ms(2_000)), "2 s asleep");
    assert_eq!(slept(ms(8_001), ms(6_000), LEAST), Some(ms(2_001)), "2.001 s asleep");
    // Another threshold, and a threshold of nothing reports nothing for no gap.
    assert_eq!(slept(ms(6_500), ms(6_000), ms(500)), Some(ms(500)));
    assert_eq!(slept(ms(6_499), ms(6_000), ms(500)), None);
    assert_eq!(slept(ms(6_000), ms(6_000), Duration::ZERO), None);
    Ok(())
}

#[test]
fn a_suspend_is_reported_once_and_the_next_stretch_starts_afresh() -> Outcome<()> {
    let s = Duration::from_secs;
    let base = Instant::now();
    let mut a = Asleep::at(s(100), base, LEAST);
    // Asleep for 10 s inside a 12 s turn.
    assert_eq!(a.check_at(s(112), base + s(2)), Some(s(10)));
    // The turn after is ordinary: 5 s on both clocks.
    assert_eq!(a.check_at(s(117), base + s(7)), None);
    // And a second suspend is reported at its own length, not the sum.
    assert_eq!(a.check_at(s(130), base + s(9)), Some(s(11)));
    Ok(())
}

#[test]
fn a_monotonic_clock_ahead_of_the_boot_clock_reports_nothing() -> Outcome<()> {
    // It cannot be, but two reads a moment apart can say it; it must not underflow.
    let s = Duration::from_secs;
    let base = Instant::now();
    let mut a = Asleep::at(s(100), base, LEAST);
    assert_eq!(a.check_at(s(101), base + s(30)), None);
    // And a boot clock that went backwards is nothing either.
    assert_eq!(a.check_at(s(90), base + s(31)), None);
    Ok(())
}

#[test]
fn the_real_clocks_a_short_pause_apart_report_no_suspend() -> Outcome<()> {
    let mut a = res!(Asleep::new());
    for _ in 0..4 {
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(a.check(), None);
    }
    // A threshold that would catch a millisecond of difference still catches nothing here.
    let mut a = res!(Asleep::with_least(Duration::from_millis(50)));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(a.check(), None, "both real clocks ran through a 300 ms sleep");
    Ok(())
}

// The child of the SIGSTOP test: it checks every 50 ms for 6 s, says each suspend it is
// told of, and at the end says the longest gap between two of its turns by each clock. Each
// line starts with a newline, since the test harness has left its own words on the line.
fn watching_child() -> Outcome<()> {
    let mut a = res!(Asleep::new());
    let began = Instant::now();
    println!("\nCHILD ready");
    let mut top_mono = Duration::ZERO;
    let mut top_boot = Duration::ZERO;
    let mut last_mono = Instant::now();
    let mut last_boot = res!(since_boot());
    while began.elapsed() < Duration::from_secs(6) {
        std::thread::sleep(Duration::from_millis(50));
        if let Some(d) = a.check() {
            println!("\nCHILD slept {}", d.as_millis());
        }
        let m = Instant::now();
        let b = res!(since_boot());
        top_mono = top_mono.max(m.duration_since(last_mono));
        top_boot = top_boot.max(b.saturating_sub(last_boot));
        last_mono = m;
        last_boot = b;
    }
    println!("\nCHILD gaps mono {} boot {}", top_mono.as_millis(), top_boot.as_millis());
    Ok(())
}

// Whatever happens to the test, a stopped child must not be left stopped.
struct Reap(Child);

impl Drop for Reap {
    fn drop(&mut self) {
        let _ = Command::new("kill").arg("-CONT").arg(self.0.id().to_string()).output();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn signal(sig: &str, child: &Child) -> Outcome<()> {
    let out = res!(io(Command::new("kill").arg(fmt!("-{}", sig)).arg(fmt!("{}", child.id()))
        .output(), "run kill"));
    if !out.status.success() {
        return Err(err!("kill -{} {} answered {:?}.", sig, child.id(), out.status; Test, System));
    }
    Ok(())
}

// The kernel's own word for the process: the letter after "State:".
fn state(child: &Child) -> Outcome<char> {
    let text = res!(io(fs::read_to_string(fmt!("/proc/{}/status", child.id())), "read the child's status"));
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("State:") {
            if let Some(c) = rest.trim().chars().next() {
                return Ok(c);
            }
        }
    }
    Err(err!("No State line in the child's /proc status."; Test, Missing))
}

#[test]
fn a_process_stopped_by_sigstop_for_three_seconds_is_not_a_suspend() -> Outcome<()> {
    if std::env::var(CHILD).is_ok() {
        return watching_child();
    }
    let exe = res!(io(std::env::current_exe(), "find this test binary"));
    let child = res!(io(Command::new(&exe)
        .args(["--exact", "a_process_stopped_by_sigstop_for_three_seconds_is_not_a_suspend",
            "--test-threads=1", "--nocapture"])
        .env(CHILD, "1")
        .stdout(Stdio::piped())
        .spawn(), "start the child"));
    let mut child = Reap(child);
    let out = match child.0.stdout.take() {
        Some(o) => o,
        None => return Err(err!("The child has no stdout."; Test, Missing)),
    };
    let mut lines = BufReader::new(out).lines();
    loop {
        match lines.next() {
            Some(Ok(l)) if l == "CHILD ready" => break,
            Some(Ok(_)) => continue,
            _ => return Err(err!("The child never said ready."; Test, Missing)),
        }
    }
    std::thread::sleep(SECOND);
    // A real stop, and the kernel's confirmation that the child is stopped.
    let up0 = res!(uptime());
    let t0 = Instant::now();
    res!(signal("STOP", &child.0));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(res!(state(&child.0)), 'T', "the kernel does not call the child stopped");
    std::thread::sleep(Duration::from_millis(2_800));
    assert_eq!(res!(state(&child.0)), 'T', "the child ran while it was meant to be stopped");
    res!(signal("CONT", &child.0));
    let stopped = t0.elapsed();
    let up1 = res!(uptime());
    println!("stopped for {:?} by the monotonic clock, {:.2} s by /proc/uptime", stopped, up1 - up0);
    assert!(stopped >= Duration::from_secs(3));
    assert!(up1 - up0 >= 3.0 - 0.02, "the kernel's boot clock moved {} s", up1 - up0);

    let mut said = Vec::new();
    for l in lines {
        match l {
            Ok(l) => said.push(l),
            Err(_) => break,
        }
    }
    let _ = child.0.wait();
    println!("the child said: {:?}", said);
    for l in &said {
        assert!(!l.starts_with("CHILD slept"), "a stop was read as a suspend: {:?}", said);
    }
    // Each clock saw the stop as one gap of about three seconds in the child's own turns,
    // so a stop that only one clock saw would be told apart from this one.
    let gaps = match said.iter().find(|l| l.starts_with("CHILD gaps")) {
        Some(g) => g.clone(),
        None => return Err(err!("The child never reported its gaps: {:?}.", said; Test, Missing)),
    };
    let words: Vec<&str> = gaps.split_whitespace().collect();
    assert_eq!(words.len(), 6, "{:?}", gaps);
    let mono: u64 = res!(words[3].parse::<u64>().map_err(|e| err!("{}: {:?}", e, gaps; Test, Input)));
    let boot: u64 = res!(words[5].parse::<u64>().map_err(|e| err!("{}: {:?}", e, gaps; Test, Input)));
    assert!(mono >= 2_900, "the monotonic clock saw a {} ms gap across the stop", mono);
    assert!(boot >= 2_900, "the boot clock saw a {} ms gap across the stop", boot);
    assert!(boot.abs_diff(mono) < 50, "the clocks disagreed over the stop: {} ms and {} ms", mono, boot);
    Ok(())
}
