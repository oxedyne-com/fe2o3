use crate::{
    prelude::*,
    bot,
    channels::{
        Recv,
        Simplex,
    },
    log::{
        base::{
            LogLevel,
            Source,
        },
    },
};

use crate::term::Term;

use std::{
    collections::HashMap,
    fs::{
        File,
        OpenOptions,
    },
    io::{
        self,
        BufReader,
        Seek,
        SeekFrom,
        Write,
    },
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        RwLock,
    },
    time,
};

use flate2::{
    Compression,
    bufread::GzEncoder,
};
pub use humantime::format_rfc3339_seconds as timefmt;

/// The time a log line is stamped with, read where the log call is made and carried to the
/// sink in [`Msg::Log`], so a sink held up by a slow disk still shows each line's true time.
#[cfg(not(target_arch = "wasm32"))]
pub fn stamp() -> time::SystemTime {
    time::SystemTime::now()
}

/// No system clock exists on wasm; reconstruct the instant from the `Date.now()` millisecond
/// reading, which is pure arithmetic on `UNIX_EPOCH` and never panics.
#[cfg(target_arch = "wasm32")]
pub fn stamp() -> time::SystemTime {
    time::UNIX_EPOCH + time::Duration::from_millis(crate::wasm::now_ms() as u64)
}

/// Different messages can be sent to the console or file.
#[derive(Clone, Debug)]
pub enum Msg<ETAG: GenTag> where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error {
    AddStream(String, Simplex<String>),
    Base(bot::BaseMsg<ETAG>),
    Console(String, String), // Stream key, Message
    Finish(Source),
    GetStreams(Simplex<Arc<RwLock<HashMap<String, Simplex<String>>>>>),
    Level(Source, LogLevel),
    Log {
        at:     time::SystemTime, // when the log call was made, from `stamp()`
        level:  LogLevel,
        src:    Source,
        erropt: Option<Error<ETAG>>,
        msg:    String,
        stream: String,
    },
    Update(Source),
}

impl<
    ETAG: GenTag,
>
    bot::CtrlMsg for Msg<ETAG>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn finish() -> Self { Msg::Base(bot::BaseMsg::Finish) }
    fn ready() -> Self { Msg::Base(bot::BaseMsg::Ready) }
}

#[derive(Clone, Debug, Default)]
pub struct Config<ETAG: GenTag>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub file:       Option<FileConfig>,
    pub console:    Option<Simplex<Msg<ETAG>>>,
    pub level:      LogLevel,
}

impl<ETAG: GenTag> Config<ETAG>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{

    pub fn path(&self) -> Option<PathBuf> {
        if let Some(fcfg) = self.file.as_ref() {
            Some(fcfg.path())
        } else {
            None
        }
    }

    pub fn update_file(&mut self) -> Outcome<Option<File>> {
        Ok(match self.file {
            Some(ref mut fcfg) => Some(res!(fcfg.update())),
            None => None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct FileConfig {
    pub dir:        PathBuf,                // Directory for numbered log files.
    pub name:       String,                 // Log file base name.
    pub ext:        String,                 // Log file extension.
    pub num:        u16,                    // Next archive number.
    pub max:        Option<u64>,            // Maximum file size.
    pub path:       PathBuf,                // Current log file path.
    pub keep:       Option<usize>,          // archives kept, newest first; `None` keeps all
    pub keep_age:   Option<time::Duration>, // archives older than this go; `None` keeps all
    pub flush:      time::Duration,         // longest a routine line waits for a sync
    pub durable:    bool,                   // sync after every line
}

impl FileConfig {

    pub const FLUSH: time::Duration = time::Duration::from_secs(1);

    pub fn new(
        dir:    PathBuf,
        name:   String,
        ext:    String,
        num:    u16,            // Starting log file number.
        max:    Option<u64>,
    )
        -> Self
    {
        let mut result = Self {
            dir,
            name,
            ext,
            num,
            max,
            path:       PathBuf::new(),
            keep:       None,
            keep_age:   None,
            flush:      Self::FLUSH,
            durable:    false,
        };
        result.path = result.path();
        trace!("Log file path = {:?}", result.path);
        result
    }

    /// Bounds the archives a size limit (`max`) makes: at most `keep` of them, the newest,
    /// and none older than `age`. Either bound alone may be given, and the smaller wins.
    /// They are applied after each archive, when the file is opened, and hourly.
    pub fn with_retention(mut self, keep: Option<usize>, age: Option<time::Duration>) -> Self {
        self.keep = keep;
        self.keep_age = age;
        self
    }

    /// Sets how often written lines are synced to disk. Routine lines are synced at most
    /// `flush` after they are written, so a machine crash loses at most that much of them; an
    /// error line, an archive and a shutdown are synced at once. `durable` syncs after every
    /// line instead, which costs a disk flush per line. A zero `flush` means the default, 1 s.
    pub fn with_flush(mut self, flush: time::Duration, durable: bool) -> Self {
        self.flush = if flush.is_zero() { Self::FLUSH } else { flush };
        self.durable = durable;
        self
    }

    /// Every archive of this log in its directory, `<name>.<ext>.<number>.gz`, newest first by
    /// modification time, with its number.
    pub fn archives(&self) -> Outcome<Vec<(PathBuf, time::SystemTime, u16)>> {
        let stem = fmt!("{}.{}.", self.name, self.ext);
        let list = match std::fs::read_dir(&self.dir) {
            Ok(l) => l,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(err!(e,
                "Listing the log directory {:?} for archives.", self.dir;
            IO, File, Read)),
        };
        let mut out = Vec::new();
        for entry in list {
            let entry = res!(entry);
            let fname = entry.file_name();
            let num = match fname.to_str()
                .and_then(|f| f.strip_prefix(&stem))
                .and_then(|f| f.strip_suffix(".gz"))
            {
                Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) =>
                    match n.parse::<u16>() {
                        Ok(n) => n,
                        Err(_) => continue,
                    },
                _ => continue,
            };
            let meta = res!(entry.metadata());
            if !meta.is_file() {
                continue;
            }
            let at = meta.modified().unwrap_or(time::UNIX_EPOCH);
            out.push((entry.path(), at, num));
        }
        out.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        Ok(out)
    }

    /// Removes the archives past `keep` or older than `keep_age` at `now`, and gives how many
    /// went.
    pub fn prune(&self, now: time::SystemTime) -> Outcome<usize> {
        if self.keep.is_none() && self.keep_age.is_none() {
            return Ok(0);
        }
        let mut n = 0;
        for (i, (path, at, _)) in res!(self.archives()).into_iter().enumerate() {
            let too_many = self.keep.map(|k| i >= k).unwrap_or(false);
            let too_old = match self.keep_age {
                Some(age) => now.duration_since(at).map(|d| d > age).unwrap_or(false),
                None => false,
            };
            if too_many || too_old {
                match std::fs::remove_file(&path) {
                    Ok(()) => n += 1,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(err!(e,
                        "Removing the log archive {:?}.", path;
                    IO, File, Write)),
                }
            }
        }
        Ok(n)
    }

    pub fn path(&self) -> PathBuf {
        let mut log_path = self.dir.clone();
        log_path.push(&self.name);
        log_path.set_extension(&self.ext);
        log_path
    }

    fn make_dir(&self) -> Outcome<()> {
        res!(std::fs::create_dir_all(&self.dir));
        Ok(())
    }

    fn update(&mut self) -> Outcome<File> {
        res!(self.make_dir());
        let path = self.path();
        self.path = path.clone();
        // Number on from the archives already there, so a restart never writes over one.
        if let Some(last) = res!(self.archives()).iter().map(|(_, _, n)| *n).max() {
            if last >= self.num {
                self.num = last.wrapping_add(1);
            }
        }
        res!(self.prune(time::SystemTime::now()));
        let file = res!(OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path));
        Ok(file)
    }

}

#[derive(Default)]
pub struct LogBot<ETAG: GenTag>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    file:       Option<File>,
    cfg:        Arc<RwLock<Config<ETAG>>>,
    chan:       Simplex<Msg<ETAG>>,
    pruned:     Option<time::Instant>,  // the last age prune
    unsynced:   Option<time::Instant>,  // the first line written since the last sync
    #[cfg(test)]
    syncs:      usize,                  // live file syncs, counted for the tests
}

impl<ETAG: GenTag> LogBot<ETAG>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{

    pub fn new() -> Self {
        Self::default()
    }

    pub fn level(&self) -> Outcome<LogLevel> {
        let unlocked_cfg = lock_read!(self.cfg);
        Ok(unlocked_cfg.level)
    }

    pub fn len(&self) -> usize {
        self.chan.len()
    }

    pub fn init(
        &mut self,
        cfg:    Arc<RwLock<Config<ETAG>>>,
        chan:   Simplex<Msg<ETAG>>,
    )
        -> Outcome<()>
    {
        self.cfg = cfg;
        res!(self.update_file());
        self.chan = chan;
        Ok(())
    }

    pub fn update_file(&mut self) -> Outcome<()> {
        res!(self.sync()); // the file being replaced
        let mut unlocked_cfg = lock_write!(self.cfg);
        self.file = res!(unlocked_cfg.update_file());
        if unlocked_cfg.console.is_none() && self.file.is_none() {
            return Err(err!(
                "LogBot must output to either a file or a console channel, but \
                neither has been specified.";
            Init, Invalid));
        }
        Ok(())
    }

    /// Formats a line stamped `at`, the time of the log call rather than of this write.
    pub fn format_msg(
        at:             time::SystemTime,
        level:          LogLevel,
        src:            &Source,
        res:            Outcome<String>,
        for_console:    bool,
        for_file:       bool,
    )
        -> (Option<String>, Option<String>) // (console, file)
    {
        let msg = match res {
            Ok(msg) => msg,
            Err(e) => fmt!("{}", e),
        };
        let mut path_str = src.file.to_string();
        if level == LogLevel::Info {
            let path = Path::new(src.file);
            path_str = match path.file_name() {
                Some(s) => match s.to_os_string().into_string() {
                    Ok(s) => s,
                    Err(_) => fmt!("{}", path.display()),
                }
                None => fmt!("{}", path.display()),
            };
        }
        let t = at;
        let console_result = if for_console {
            let prefix = fmt!(
                "{}: {:5?} {} {}:{}",
                timefmt(t),
                src.tid,
                level,
                path_str,
                src.line,
            );
            Some(fmt!("{} {}{}", prefix, Term::RESET, msg))
        } else {
            None
        };
        let file_result = if for_file {
            let prefix = fmt!(
                "{}: {:5?} {:?} {}:{}",
                timefmt(t),
                src.tid,
                level,
                path_str,
                src.line,
            );
            Some(fmt!("{} {}", prefix, msg))
        } else {
            None
        };
        (
            console_result,
            file_result,
        )
    }

    fn file_max_size(&self) -> Outcome<Option<u64>> {
        let unlocked_cfg = lock_read!(self.cfg);
        Ok(if let Some(file_cfg) = unlocked_cfg.file.as_ref() {
            file_cfg.max
        } else {
            None
        })
    }

    /// The live file's flush interval and whether every line is synced.
    fn flush_cfg(&self) -> Outcome<(time::Duration, bool)> {
        let unlocked_cfg = lock_read!(self.cfg);
        Ok(match unlocked_cfg.file.as_ref() {
            Some(file_cfg) => (file_cfg.flush, file_cfg.durable),
            None => (FileConfig::FLUSH, false),
        })
    }

    fn wrote(&mut self) {
        if self.unsynced.is_none() {
            self.unsynced = Some(time::Instant::now());
        }
    }

    /// Syncs the live file to disk, if anything has been written to it since the last sync.
    fn sync(&mut self) -> Outcome<()> {
        if self.unsynced.take().is_some() {
            if let Some(f) = &self.file {
                res!(f.sync_data());
                #[cfg(test)]
                { self.syncs += 1; }
            }
        }
        Ok(())
    }

    /// Syncs now when the line just written must not wait, an error line or any line when
    /// durable, or when the oldest unsynced line has waited the flush interval.
    fn sync_after(&mut self, urgent: bool) -> Outcome<()> {
        if let Some(t) = self.unsynced {
            let (flush, durable) = res!(self.flush_cfg());
            if urgent || durable || t.elapsed() >= flush {
                res!(self.sync());
            }
        }
        Ok(())
    }

    fn check_for_archiving(&mut self) -> Outcome<()> {
        if let Some(f) = &self.file {
            let size = res!(f.metadata()).len();
            if let Some(max_size) = res!(self.file_max_size()) {
                if size > max_size {
                    res!(self.archive());
                }
            }
        }
        // An age bound must hold on a quiet log too, which may archive once a month.
        const PRUNE_EVERY: time::Duration = time::Duration::from_secs(3600);
        if self.pruned.map(|t| t.elapsed() >= PRUNE_EVERY).unwrap_or(true) {
            self.pruned = Some(time::Instant::now());
            let unlocked_cfg = lock_read!(self.cfg);
            if let Some(file_cfg) = unlocked_cfg.file.as_ref() {
                res!(file_cfg.prune(time::SystemTime::now()));
            }
        }
        Ok(())
    }

    fn archive(&mut self) -> Outcome<()> {
        res!(self.sync());
        let mut unlocked_cfg = lock_write!(self.cfg);
        if let (Some(file_cfg), Some(old)) = (unlocked_cfg.file.as_mut(), self.file.as_mut()) {
            let mut log_path = file_cfg.dir.clone();
            log_path.push(fmt!("{}.{}.{:02}.gz", file_cfg.name, file_cfg.ext, file_cfg.num));
            let mut new = res!(File::create(log_path));
            res!(old.seek(SeekFrom::Start(0)));
            {
                let old_buf = BufReader::new(&mut *old);
                let mut gz = GzEncoder::new(old_buf, Compression::fast());
                res!(io::copy(&mut gz, &mut new));
            }
            res!(new.sync_all()); // the archive is whole before the live file is cut
            res!(old.set_len(0)); // Return to start.
            file_cfg.num = file_cfg.num.wrapping_add(1);
            res!(file_cfg.prune(time::SystemTime::now()));
        }
        Ok(())
    }

    fn get_file_path(&self) -> Outcome<Option<PathBuf>> {
        let unlocked_cfg = lock_read!(self.cfg);
        Ok(match &unlocked_cfg.file {
            Some(file_cfg) => Some(file_cfg.path()),
            None => None,
        })
    }

    fn write_err(
        &mut self,
        src:    &Source,
        e:      Error<ErrTag>,
    ) {
        let at = stamp(); // the bot's own error, found now
        let mut for_console = true;
        let mut for_file = false;
        {
            let unlocked_cfg = self.cfg.read();
            match unlocked_cfg {
                Ok(cfg) => {
                    for_console = cfg.console.is_some();
                    for_file = cfg.file.is_some();
                },
                Err(e) => {
                    let (msg_console_opt, _msg_file_opt) = Self::format_msg(
                        at,
                        LogLevel::Error,
                        src,
                        Err(err!(
                            "Could access log configuration: {}", e;
                        Poisoned, Configuration)),
                        for_console,
                        for_file,
                    );
                    if let Some(msg) = msg_console_opt {
                        println!("{}", msg);
                    }
                },
            }
        }
        let (msg_console_opt, msg_file_opt) = Self::format_msg(
            at,
            LogLevel::Error,
            src,
            Err(e),
            for_console,
            for_file,
        );
        if let Some(msg) = msg_console_opt {
            println!("{}", msg);
        }
        if let Some(f) = self.file.as_mut() {
            if let Some(msg) = msg_file_opt {
                match writeln!(f, "{}", msg) {
                    Err(e) => msg!("{}", err!(e,
                        "Error writing '{}' to the log file.", msg;
                    IO, File, Write)),
                    _ => self.wrote(),
                }
            }
        }
        if let Err(e) = self.sync_after(true) {
            msg!("{}", err!(e, "Error syncing the log file after an error line."; IO, File, Write));
        }
    }

    fn write(
        &mut self,
        at:     time::SystemTime,
        level:  LogLevel,
        src:    &Source,
        res:    Outcome<String>,
        stream: String,
    ) {
        let mut path_opt = None;
        let mut console_chan = None;
        let mut err_opt = None;
        let mut current_level = LogLevel::None;
        {
            let unlocked_cfg = self.cfg.write();
            match unlocked_cfg {
                Ok(cfg) => {
                    path_opt = match &cfg.file {
                        Some(file_cfg) => Some(file_cfg.path()),
                        None => None,
                    };
                    console_chan = cfg.console.clone();
                    current_level = cfg.level;
                },
                Err(e) => err_opt = Some(e.into()),
            }
        }
        if let Some(e) = err_opt {
            let src = Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            };
            self.write_err(&src, e);
            return;
        }
        // Send log entry to media, as necessary.
        if level <= current_level {
            let (msg_console_opt, msg_file_opt) = Self::format_msg(
                at,
                level,
                &src,
                res,
                console_chan.is_some(),
                self.file.is_some(),
            );
            if let Some(console_chan) = console_chan {
                if let Some(msg) = msg_console_opt {
                    match console_chan.send(Msg::Console(stream, msg.clone())) {
                        Err(e) => msg!("{}", err!(e,
                            "Error writing '{}' to the console channel.", msg;
                        IO, Channel, Write)),
                        _ => (),
                    }
                }
            }
            if let Some(f) = self.file.as_mut() {
                if let Some(msg) = msg_file_opt {
                    match writeln!(f, "{}", msg) {
                        Err(e) => msg!("{}", err!(e,
                            "Error writing '{}' to the log file {:?}.", msg, path_opt;
                        IO, File, Write)),
                        _ => self.wrote(),
                    }
                }
            }
            // The line that explains a crash must not wait for the interval.
            let urgent = level == LogLevel::Error;
            if let Err(e) = self.sync_after(urgent) {
                msg!("{}", err!(e,
                    "Error syncing the log file {:?} after a {:?} line.", path_opt, level;
                IO, File, Write));
            }
        }
    }

    /// Starts the lbot thread and the receiving loop.  Does not panic, instead sends error
    /// messages to `stdout` via `msg!`.
    pub fn go(&mut self) {
        loop {
            if self.listen() {
                break;
            }
        }
    }

    /// Waits for the next message; while unsynced lines wait, only until the flush interval
    /// is up, so a quiet log still syncs them on time.
    fn next_msg(&mut self) -> Option<Outcome<Msg<ETAG>>> {
        let wait = match (self.unsynced, self.flush_cfg()) {
            (None, _)               => return Some(self.chan.recv()),
            (Some(t), Ok((f, _)))   => f.saturating_sub(t.elapsed()),
            (Some(_), Err(_))       => time::Duration::ZERO,
        };
        match self.chan.recv_timeout(wait) {
            Recv::Result(r) => Some(r),
            Recv::Empty => None,
        }
    }

    fn listen(&mut self) -> bool {
        let next = match self.next_msg() {
            Some(next) => next,
            None => {
                if let Err(e) = self.sync() {
                    msg!("{}", err!(e, "Error syncing the log file on its interval."; IO, File, Write));
                }
                return false;
            }
        };
        match next {
            Err(e) => {
                let e = err!(e,
                    "Error while LogBot attempted to receive a message.";
                IO, Channel, Read);
                let src = Source {
                    tid: std::thread::current().id(),
                    file: file!(),
                    line: line!(),
                };
                self.write_err(&src, e);
            }
            Ok(Msg::Finish(src)) => {
                let msg = fmt!("Finish message received, LogBot finishing now.");
                let level = LogLevel::Warn;
                self.write(stamp(), level, &src, Ok(msg), String::new());
                if let Err(e) = self.sync() {
                    msg!("{}", err!(e, "Error syncing the log file at finish."; IO, File, Write));
                }
                return true;
            }
            Ok(Msg::Base(bot::BaseMsg::Ready)) => msg!("LogBot now ready to receive messages."),
            Ok(Msg::Log { at, level, src, erropt, msg, stream }) => {
                let msg = if let Some(e) = erropt {
                    fmt!("{} {}", msg, e)
                } else {
                    fmt!("{}", msg)
                };
                self.write(at, level, &src, Ok(msg), stream);
            }
            Ok(Msg::Update(src)) => match self.update_file() {
                Ok(()) => (),
                Err(e) => self.write_err(&src, e),
            }
            Ok(msg) => {
                let e = err!(
                    "Message handling for {:?} currently not implemented.", msg;
                IO, Channel, Read, Unimplemented);
                let src = Source {
                    tid: std::thread::current().id(),
                    file: file!(),
                    line: line!(),
                };
                self.write_err(&src, e);
            }
        }
        match self.check_for_archiving() {
            Err(e) => {
                let src = Source {
                    tid: std::thread::current().id(),
                    file: file!(),
                    line: line!(),
                };
                self.write(stamp(), LogLevel::Error, &src, Err(e), String::new());
            },
            _ => (),
        }
        false
    }
}

impl<ETAG: GenTag> Drop for LogBot<ETAG>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn drop(&mut self) {
        if let Err(e) = self.sync() {
            msg!("{}", err!(e, "Error syncing the log file as the log bot ends."; IO, File, Write));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(fmt!("fe2o3_log_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn touch_archive(cfg: &FileConfig, num: u16, age_secs: u64) -> Outcome<()> {
        let p = cfg.dir.join(fmt!("{}.{}.{:02}.gz", cfg.name, cfg.ext, num));
        let f = res!(File::create(&p));
        let at = time::SystemTime::now() - time::Duration::from_secs(age_secs);
        res!(f.set_modified(at));
        Ok(())
    }

    fn bot_for(tag: &str, fcfg: impl FnOnce(FileConfig) -> FileConfig)
        -> Outcome<(LogBot<ErrTag>, Simplex<Msg<ErrTag>>, PathBuf)>
    {
        let dir = scratch(tag);
        let file = fcfg(FileConfig::new(dir.clone(), fmt!("app"), fmt!("log"), 0, None));
        let cfg = Config { file: Some(file), console: None, level: LogLevel::Trace };
        let chan = crate::channels::simplex();
        let mut bot = LogBot::new();
        res!(bot.init(Arc::new(RwLock::new(cfg)), chan.clone()));
        Ok((bot, chan, dir))
    }

    fn line(chan: &Simplex<Msg<ErrTag>>, level: LogLevel, n: usize) -> Outcome<()> {
        let src = Source { tid: std::thread::current().id(), file: file!(), line: line!() };
        res!(chan.send(Msg::Log { at: stamp(), level, src, erropt: None, msg: fmt!("line {}", n), stream: String::new() }));
        Ok(())
    }

    #[test]
    fn test_routine_lines_are_not_synced_one_by_one() -> Outcome<()> {
        const N: usize = 200;
        let (mut bot, chan, dir) = res!(bot_for("routine", |f| f));
        for n in 0..N {
            res!(line(&chan, LogLevel::Info, n));
            bot.listen();
        }
        assert!(bot.syncs <= 3, "{} routine lines took {} syncs.", N, bot.syncs);
        let text = res!(std::fs::read_to_string(dir.join("app.log")));
        assert_eq!(text.lines().count(), N);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_an_error_line_is_synced_at_once() -> Outcome<()> {
        let hour = time::Duration::from_secs(3600);
        let (mut bot, chan, dir) = res!(bot_for("errline", |f| f.with_flush(hour, false)));
        for n in 0..5 {
            res!(line(&chan, LogLevel::Info, n));
            bot.listen();
        }
        assert_eq!(bot.syncs, 0, "Routine lines were synced within the interval.");
        res!(line(&chan, LogLevel::Error, 5));
        bot.listen();
        assert_eq!(bot.syncs, 1, "The error line was not synced at once.");
        assert!(bot.unsynced.is_none());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_durable_syncs_every_line() -> Outcome<()> {
        const N: usize = 20;
        let hour = time::Duration::from_secs(3600);
        let (mut bot, chan, dir) = res!(bot_for("durable", |f| f.with_flush(hour, true)));
        for n in 0..N {
            res!(line(&chan, LogLevel::Info, n));
            bot.listen();
        }
        assert_eq!(bot.syncs, N);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_a_quiet_log_syncs_on_its_interval() -> Outcome<()> {
        let flush = time::Duration::from_millis(200);
        let (mut bot, chan, dir) = res!(bot_for("quiet", |f| f.with_flush(flush, false)));
        res!(line(&chan, LogLevel::Info, 0));
        bot.listen();
        assert_eq!(bot.syncs, 0);
        // Nothing more arrives, so the wait times out and the line is synced.
        let t = time::Instant::now();
        bot.listen();
        assert_eq!(bot.syncs, 1);
        assert!(t.elapsed() < time::Duration::from_secs(2));
        // A line still waiting is synced at finish.
        res!(line(&chan, LogLevel::Info, 1));
        bot.listen();
        let src = Source { tid: std::thread::current().id(), file: file!(), line: line!() };
        res!(chan.send(Msg::Finish(src)));
        assert!(bot.listen(), "Finish did not end the bot.");
        assert_eq!(bot.syncs, 2, "Finish did not sync the lines left.");
        drop(bot);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_prune_keeps_the_newest_and_drops_the_old() -> Outcome<()> {
        let dir = scratch("prune");
        res!(std::fs::create_dir_all(&dir));
        let cfg = FileConfig::new(dir.clone(), fmt!("app"), fmt!("log"), 0, Some(10))
            .with_retention(Some(3), Some(time::Duration::from_secs(30 * 86_400)));
        // Five archives a day apart, and one past the age bound.
        for n in 0..5u16 {
            res!(touch_archive(&cfg, n, (5 - n as u64) * 86_400));
        }
        res!(touch_archive(&cfg, 7, 31 * 86_400));
        // Neither a stranger nor the live file is an archive.
        res!(File::create(dir.join("app.log")));
        res!(File::create(dir.join("other.log.01.gz")));
        assert_eq!(res!(cfg.archives()).len(), 6);
        let gone = res!(cfg.prune(time::SystemTime::now()));
        assert_eq!(gone, 3);
        let left: Vec<u16> = res!(cfg.archives()).into_iter().map(|(_, _, n)| n).collect();
        assert_eq!(left, vec![4, 3, 2]);
        assert!(dir.join("app.log").exists() && dir.join("other.log.01.gz").exists());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn test_reopening_numbers_on_from_the_last_archive() -> Outcome<()> {
        let dir = scratch("renum");
        res!(std::fs::create_dir_all(&dir));
        let mut cfg = FileConfig::new(dir.clone(), fmt!("app"), fmt!("log"), 0, Some(10));
        res!(touch_archive(&cfg, 0, 60));
        res!(touch_archive(&cfg, 1, 30));
        let _f = res!(cfg.update());
        assert_eq!(cfg.num, 2, "A restart would have written over archive 00.");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// A sink that stalls must not move a line's time: the line carries the time of the
    /// call, not of the write.  The 16 Oct gateway log showed a 57 s silence and wrong
    /// times on every queued line because the stamp was taken at the write.
    #[test]
    fn test_a_stalled_sink_keeps_the_time_of_the_call() -> Outcome<()> {
        let (mut bot, chan, dir) = res!(bot_for("stamp", |f| f));
        res!(line(&chan, LogLevel::Info, 0));
        bot.listen();
        // The second line is logged now, but the sink is held for 2 s before it writes.
        let called = time::SystemTime::now();
        res!(line(&chan, LogLevel::Info, 1));
        std::thread::sleep(time::Duration::from_secs(2));
        bot.listen();
        drop(bot);
        let text = res!(std::fs::read_to_string(dir.join("app.log")));
        let second = match text.lines().nth(1) {
            Some(l) => l.to_string(),
            None => return Err(err!("The second line was not written: {:?}", text; Test, Missing)),
        };
        let stamp = match second.split(": ").next() {
            Some(s) => s,
            None => return Err(err!("No stamp on {:?}.", second; Test, Invalid)),
        };
        let at = res!(humantime::parse_rfc3339(stamp));
        // The stamp is whole seconds, so it lies within the second before the call.
        let early = called - time::Duration::from_secs(1);
        assert!(at <= called && at > early,
            "The line was stamped {} but called at {}: the stamp is the write time.",
            timefmt(at), humantime::format_rfc3339_millis(called));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
