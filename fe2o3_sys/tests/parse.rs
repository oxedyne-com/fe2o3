//! Integration tests for the `/proc` parsers. Each test uses a
//! fixed sample body copied from a real machine so the parsers
//! are exercised deterministically, regardless of the host the
//! test happens to run on.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_sys::{
    cpu::CpuTimes,
    disk::DiskStats,
    load::LoadAvg,
    mem::MemInfo,
    net::NetStats,
    proc_self::ProcSelf,
    uptime::Uptime,
};

#[test]
fn cpu_aggregate_line() -> Outcome<()> {
    let sample =
        "cpu  100 20 40 500 10 0 5 0 0 0\n\
         cpu0 50 10 20 250 5 0 2 0 0 0\n\
         cpu1 50 10 20 250 5 0 3 0 0 0\n\
         intr 123\n";
    let c = res!(CpuTimes::from_stat(sample));
    assert_eq!(c.user, 100);
    assert_eq!(c.nice, 20);
    assert_eq!(c.system, 40);
    assert_eq!(c.idle, 500);
    assert_eq!(c.iowait, 10);
    assert_eq!(c.irq, 0);
    assert_eq!(c.softirq, 5);
    assert_eq!(c.total(), 675);
    assert_eq!(c.idle_total(), 510);
    Ok(())
}

#[test]
fn cpu_busy_fraction() -> Outcome<()> {
    let prev = res!(CpuTimes::from_line("cpu 100 0 50 850 0 0 0 0 0 0"));
    let now  = res!(CpuTimes::from_line("cpu 150 0 100 900 0 0 0 0 0 0"));
    // total prev = 1000, now = 1150 → delta 150
    // idle prev = 850,  now = 900  → delta 50
    // busy / total = 100 / 150 ≈ 0.6667
    let frac = now.busy_fraction(&prev);
    assert!((frac - (100.0 / 150.0)).abs() < 1e-9);
    Ok(())
}

#[test]
fn meminfo_parses_total_and_available() -> Outcome<()> {
    let sample =
        "MemTotal:        8000000 kB\n\
         MemFree:          500000 kB\n\
         MemAvailable:    2000000 kB\n\
         Buffers:          100000 kB\n\
         Cached:          1500000 kB\n\
         SwapTotal:       1000000 kB\n\
         SwapFree:         900000 kB\n";
    let m = res!(MemInfo::from_meminfo(sample));
    assert_eq!(m.total,     8_000_000 * 1024);
    assert_eq!(m.available, 2_000_000 * 1024);
    assert_eq!(m.swap_total,1_000_000 * 1024);
    assert_eq!(m.swap_free,   900_000 * 1024);
    assert_eq!(m.swap_used(), 100_000 * 1024);
    // used = total - available = 6M kB.
    assert_eq!(m.used(),     6_000_000 * 1024);
    Ok(())
}

#[test]
fn loadavg_parses() -> Outcome<()> {
    let l = res!(LoadAvg::from_line("0.42 0.51 0.63 3/612 12345"));
    assert!((l.one - 0.42).abs() < 1e-9);
    assert!((l.five - 0.51).abs() < 1e-9);
    assert!((l.fifteen - 0.63).abs() < 1e-9);
    assert_eq!(l.runnable, 3);
    assert_eq!(l.total, 612);
    Ok(())
}

#[test]
fn uptime_parses() -> Outcome<()> {
    let u = res!(Uptime::from_line("12345.67 98765.43"));
    assert!((u.seconds - 12345.67).abs() < 1e-3);
    assert!((u.idle_total - 98765.43).abs() < 1e-3);
    Ok(())
}

#[test]
fn diskstats_parses_minimum_column_count() -> Outcome<()> {
    // 14-column diskstats line (kernel 4.18+).
    let sample =
        "   8       0 sda 100 0 2000 50 200 0 4000 80 0 120 130\n\
         254       0 dm-0 10 0 200 5 20 0 400 8 0 12 13\n";
    let d = res!(DiskStats::from_diskstats(sample));
    assert_eq!(d.devices.len(), 2);
    assert_eq!(d.devices[0].name, "sda");
    assert_eq!(d.devices[0].reads, 100);
    assert_eq!(d.devices[0].read_sectors, 2000);
    assert_eq!(d.devices[0].writes, 200);
    assert_eq!(d.devices[0].write_sectors, 4000);
    assert_eq!(d.devices[1].name, "dm-0");
    Ok(())
}

#[test]
fn net_dev_parses() -> Outcome<()> {
    let sample =
        "Inter-|   Receive                                                |  Transmit\n\
         face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n\
         lo:  1000     10    0    0    0     0          0         0     1000      10    0    0    0     0       0          0\n\
         eth0: 5000    50    1    2    0     0          0         0     6000      60    0    0    0     0       0          0\n";
    let n = res!(NetStats::from_net_dev(sample));
    assert_eq!(n.interfaces.len(), 2);
    assert_eq!(n.interfaces[0].name, "lo");
    assert_eq!(n.interfaces[0].rx_bytes, 1000);
    assert_eq!(n.interfaces[0].tx_bytes, 1000);
    assert_eq!(n.interfaces[1].name, "eth0");
    assert_eq!(n.interfaces[1].rx_errors, 1);
    assert_eq!(n.interfaces[1].rx_drops, 2);
    assert_eq!(n.interfaces[1].tx_bytes, 6000);
    Ok(())
}

#[test]
fn proc_self_status_parses() -> Outcome<()> {
    let sample =
        "Name:\tsteel\n\
         State:\tR (running)\n\
         VmPeak:\t   12345 kB\n\
         VmSize:\t   10000 kB\n\
         VmRSS:\t    4000 kB\n\
         Threads:\t8\n\
         voluntary_ctxt_switches:\t1234\n\
         nonvoluntary_ctxt_switches:\t56\n";
    let p = res!(ProcSelf::from_status(sample));
    assert_eq!(p.rss_bytes,   4_000 * 1024);
    assert_eq!(p.vsize_bytes, 10_000 * 1024);
    assert_eq!(p.peak_rss,    12_345 * 1024);
    assert_eq!(p.threads,     8);
    assert_eq!(p.voluntary_ctxt,   1234);
    assert_eq!(p.involuntary_ctxt, 56);
    Ok(())
}

#[test]
fn proc_self_stat_cpu_ticks_parses() -> Outcome<()> {
    // A synthetic /proc/self/stat line whose comm field deliberately
    // contains spaces and brackets to exercise the "parse after the
    // final ')'" logic.  utime is field 14 (value 30) and stime is
    // field 15 (value 12), so the sum is 42.
    let sample = "1234 (odd )name) ) R 1 1234 1234 0 -1 4194560 100 0 0 0 30 12 0 0 20 0 8 0 999";
    let ticks = res!(ProcSelf::cpu_ticks_from_stat(sample));
    assert_eq!(ticks, 42);
    Ok(())
}
