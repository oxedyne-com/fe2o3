//!
//! A simple logging library.
//! 
//! Log a message by sending it to a static `LogBot` (lbot) thread using a macro.  An approximate
//! maximum file size can be set for the lbot file, which results in the creation of sequential
//! gzipped archive files over time.  See `tests/db.rs` for an example of how to boot and use the
//! lbot, run the test using `cargo test --test lbot -- --nocapture`.
//!
use crate::{
    prelude::*,
    channels::simplex,
    log::{
        bot::{
            Config,
            LogBot,
            Msg,
        },
        console::{
            LoggerConsole,
            StdoutLoggerConsole,
        },
    },
    thread::{
        thread_channel,
        SimplexThread,
    },
};

use oxedize_fe2o3_stds::chars::Term;

use std::{
    fmt,
    str::FromStr,
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
    thread,
};

use once_cell::sync::Lazy;


pub struct Logger<ETAG: GenTag>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub chan_in:    SimplexThread<Msg<ETAG>>,
    pub chan_out:   Arc<RwLock<SimplexThread<Msg<ETAG>>>>,
    pub cfg:        Arc<RwLock<Config<ETAG>>>,
}

impl<ETAG: GenTag> Logger<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    /// Unfortunately there is no obvious way to retain a printable version of `msg` up our sleeve to
    /// show on the off chance that `send` fails, without cloning.
    pub fn send_in(&self, msg: Msg<ETAG>) {
        match self.chan_in.chan.send(msg.clone()) {
            Err(e) => msg!("Error!: {} for message: {:?}", e, msg),
            Ok(()) => (),
        }
    }

    pub fn send_out(&self, msg: Msg<ETAG>) -> Outcome<()> {
        let unlocked_chan_out = lock_read!(self.chan_out);
        match unlocked_chan_out.chan.send(msg.clone()) {
            Ok(()) => Ok(()),
            Err(e) => Err(err!(e, errmsg!(
                "While trying to send message: {:?}", msg,
            ), IO, Channel, Write)),
        }
    }

    pub fn recv_in(&self) -> Outcome<Msg<ETAG>> {
        self.chan_in.chan.recv()
    }
}
    
pub static LOG: Lazy<Logger<ErrTag>> = Lazy::new(|| {
    let mut logbot = LogBot::new();
    let mut logger_console = StdoutLoggerConsole::new();
    let chan_out = logger_console.chan.clone();
    let cfg = Arc::new(RwLock::new(Config{
        file:       None,
        level:      LogLevel::Trace,
        console:    Some(chan_out),
    }));
    let chan_in = simplex::<Msg<ErrTag>>();
    let (semaphore, _sentinel) = thread_channel();
    match logbot.init(cfg.clone(), chan_in.clone()) {
        Ok(()) => (),
        // One of the few panics in Hematite, done as early as possible.
        Err(e) => panic!("{}", e),
    }
    let chan_out = logger_console.go();
    let semaphore_clone = semaphore.clone();
    let handle = thread::spawn(move || {
        // We just touch the semaphore to ensure its Drop function gets activated when the closure
        // ends.  Normally we could just hide the semaphore inside the bot but LOG is never
        // dropped, being static.  This also means we have only one mechanism to stop the thread,
        // by sending a finish message.  Normally bots can also use the sentinel as a second
        // mechanism.  The sentinel can, however, detect when the thread has ended, and how.
        semaphore.touch();
        logbot.go();
    });
    Logger {
        chan_in: SimplexThread::new(
            chan_in,
            Arc::new(Mutex::new(Some(handle))),
            semaphore_clone,
        ),
        chan_out: Arc::new(RwLock::new(chan_out)),
        cfg, 
    }
});

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub enum LogLevel {
    None,
    Error,
    Warn,
    Info,
    Test,
    Debug,
    Trace,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::None
    }
}

/// `Display` is for console use.
impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => Ok(()),
            Self::Error => write!(
                f,
                "{}{}ERR!{}{}",
                Term::SET_BRIGHT_FORE_RED,
                Term::BOLD,
                Term::RESET,
                Term::FORE_RED,
            ),
            Self::Warn => write!(
                f,
                "{}{}WARN{}{}",
                Term::SET_BRIGHT_FORE_YELLOW,
                Term::BOLD,
                Term::RESET,
                Term::FORE_YELLOW,
            ),
            Self::Info => write!(
                f,
                "{}{}INFO{}{}",
                Term::SET_BRIGHT_FORE_GREEN,
                Term::BOLD,
                Term::RESET,
                Term::FORE_GREEN,
            ),
            Self::Test => write!(
                f,
                "{}{}TEST{}{}",
                Term::SET_BRIGHT_FORE_CYAN,
                Term::BOLD,
                Term::RESET,
                Term::FORE_CYAN,
            ),
            Self::Debug => write!(
                f,
                "{}{}DBUG{}{}",
                Term::SET_BRIGHT_FORE_BLUE,
                Term::BOLD,
                Term::RESET,
                Term::FORE_BLUE,
            ),
            Self::Trace => write!(
                f,
                "{}{}TRCE{}{}",
                Term::SET_BRIGHT_FORE_MAGENTA,
                Term::BOLD,
                Term::RESET,
                Term::FORE_MAGENTA,
            ),
        }
    }
}

/// `Debug` is for file use.
impl fmt::Debug for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None  => Ok(()),
            Self::Error => write!(f, "ERR!"),
            Self::Warn  => write!(f, "WARN"),
            Self::Info  => write!(f, "INFO"),
            Self::Test  => write!(f, "TEST"),
            Self::Debug => write!(f, "DBUG"),
            Self::Trace => write!(f, "TRCE"),
        }
    }
}

impl FromStr for LogLevel {
    type Err = Error<ErrTag>;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "error" => Ok(LogLevel::Error),
            "warn"  => Ok(LogLevel::Warn),
            "info"  => Ok(LogLevel::Info),
            "test"  => Ok(LogLevel::Test),
            "debug" => Ok(LogLevel::Debug),
            "trace" => Ok(LogLevel::Trace),
            _ => Err(err!(errmsg!(
                "The LogLevel '{}' is not recognised, use 'trace', 'debug', \
                'info', 'warn', or 'error'.", s,
            ), Invalid, Input)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Source {
    pub tid:    std::thread::ThreadId,
    pub file:   &'static str,
    pub line:   u32,
}

/// `std::sync::MutexGuard` is explicitly `!Send` so we need to handle errors in `log_finish_wait!` with
/// special care.
#[derive(Debug)]
pub enum LogWaitError {
    LockError(String),
    JoinError(String),
}

impl fmt::Display for LogWaitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LockError(msg) => write!(f, "Failed to acquire lock on logger handle: {}", msg),
            Self::JoinError(msg) => write!(f, "Failed to join logging thread: {}", msg),
        }
    }
}

impl std::error::Error for LogWaitError {}
