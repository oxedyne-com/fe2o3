use oxedyne_fe2o3_core::{
    prelude::*,
    channels::{
        simplex,
        simplex_bounded,
    },
    log::{
        base::{
            LogLevel,
            Source,
        },
        bot::{
            Config,
            FileConfig,
            LogBot,
            Msg,
            stamp,
        },
    },
};

use std::{
    path::PathBuf,
    sync::{
        Arc,
        RwLock,
        atomic::Ordering,
    },
    time::{
        Duration,
        Instant,
    },
};

/// Lines queued ahead of the finish message. At one fsync a line this took
/// the Daimond gateway's LogBot about 35 s on NVMe (21 Oct 2026, ~600 lines/s).
const BACKLOG: usize = 20_000;

fn src() -> Source {
    Source { tid: std::thread::current().id(), file: file!(), line: line!() }
}

/// A stop under a backlog must not wait on one fsync per queued line: a
/// service manager gives a stopping process a few seconds, then kills it.
#[test]
fn test_a_backlog_is_written_and_finished_within_five_seconds() -> Outcome<()> {
    // On disk, not tmpfs, where an fsync costs nothing and proves nothing.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("log_backlog");
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = Arc::new(RwLock::new(Config::<ErrTag> {
        file:    Some(FileConfig::new(dir.clone(), "backlog".to_string(), "log".to_string(),
                    0, None)),
        console: None,
        level:   LogLevel::Debug,
    }));
    let chan = simplex::<Msg<ErrTag>>();
    let mut bot = LogBot::new();
    res!(bot.init(cfg, chan.clone()));

    for i in 0..BACKLOG {
        res!(chan.send(Msg::Log {
            at:     stamp(),
            level:  LogLevel::Debug,
            src:    src(),
            erropt: None,
            msg:    fmt!("backlog line {} of a request that came and went", i),
            stream: String::new(),
        }));
    }
    res!(chan.send(Msg::Finish(src())));

    let start = Instant::now();
    let handle = std::thread::spawn(move || bot.go());
    if handle.join().is_err() {
        return Err(err!("The LogBot thread panicked."; Thread));
    }
    let took = start.elapsed();

    let text = res!(std::fs::read_to_string(dir.join("backlog.log")));
    // Every queued line, plus the finish notice, is on disk when go() returns.
    assert_eq!(text.lines().count(), BACKLOG + 1);
    assert!(took < Duration::from_secs(5), "finishing a backlog of {} took {:?}", BACKLOG, took);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Lines a sender dropped on a full channel are reported in the file once, even at Finish.
#[test]
fn test_dropped_lines_are_reported_once_by_the_bot() -> Outcome<()> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("log_dropped");
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = Arc::new(RwLock::new(Config::<ErrTag> {
        file:    Some(FileConfig::new(dir.clone(), "dropped".to_string(), "log".to_string(),
                    0, None)),
        console: None,
        level:   LogLevel::Info,
    }));
    let chan = simplex::<Msg<ErrTag>>();
    let mut bot = LogBot::new();
    res!(bot.init(cfg, chan.clone()));
    bot.drop_counter().store(3, Ordering::Relaxed);
    res!(chan.send(Msg::Finish(src())));
    bot.go();

    let text = res!(std::fs::read_to_string(dir.join("dropped.log")));
    assert_eq!(text.matches("3 log line(s) at info or below were dropped").count(), 1, "{}", text);
    assert_eq!(bot.drop_counter().load(Ordering::Relaxed), 0);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// A full bounded channel refuses a `try_send` rather than waiting.
#[test]
fn test_try_send_on_a_full_bounded_channel_answers_false() -> Outcome<()> {
    let chan = simplex_bounded::<u32>(2);
    assert!(res!(chan.try_send(1)));
    assert!(res!(chan.try_send(2)));
    assert!(!res!(chan.try_send(3)));
    assert_eq!(res!(chan.recv()), 1);
    assert!(res!(chan.try_send(4)));
    Ok(())
}
