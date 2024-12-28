use crate::{
    prelude::*,
    bot,
    channels::Simplex,
    log::{
        base::{
            LogLevel,
            Source,
        },
    },
};

use oxedize_fe2o3_stds::chars::Term;

use std::{
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

/// Different messages can be sent to the console or file.
#[derive(Clone, Debug)]
pub enum Msg<ETAG: GenTag> where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error {
    Base(bot::BaseMsg<ETAG>),
    Console(String),
    Finish(Source),
    Level((Source, LogLevel)),
    Log {
        level:  LogLevel,
        src:    Source,
        erropt: Option<Error<ETAG>>,
        msg:    String,
    },
    Update(Source),
}

impl<
    ETAG: GenTag,
>
    bot::CtrlMsg for Msg<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn finish() -> Self { Msg::Base(bot::BaseMsg::Finish) }
    fn ready() -> Self { Msg::Base(bot::BaseMsg::Ready) }
}

#[derive(Clone, Debug, Default)]
pub struct Config<ETAG: GenTag>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub file:       Option<FileConfig>,
    pub console:    Option<Simplex<Msg<ETAG>>>,
    pub level:      LogLevel,
}

impl<ETAG: GenTag> Config<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
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
    pub dir:    PathBuf,        // Directory for numbered log files.
    pub name:   String,         // Log file base name.
    pub ext:    String,         // Log file extension.
    pub num:    u16,            // Current log file number.
    pub max:    Option<u64>,    // Maximum file size.
    pub path:   PathBuf,        // Current log file path.
}

impl FileConfig {

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
            path: PathBuf::new(),
        };
        result.path = result.path();
        trace!("Log file path = {:?}", result.path);
        result
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
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    file:   Option<File>,   
    cfg:    Arc<RwLock<Config<ETAG>>>,
    chan:   Simplex<Msg<ETAG>>,
}

impl<ETAG: GenTag> LogBot<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
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

    pub fn format_msg(
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
        let t = time::SystemTime::now();
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

    fn check_for_archiving(&mut self) -> Outcome<()> {
        if let Some(f) = &self.file {
            res!(f.sync_all());
            let size = res!(f.metadata()).len();
            if let Some(max_size) = res!(self.file_max_size()) {
                if size > max_size {
                    res!(self.archive());
                }
            }
        }
        Ok(())
    }

    fn archive(&mut self) -> Outcome<()> {
        let mut unlocked_cfg = lock_write!(self.cfg);
        if let Some(file_cfg) = unlocked_cfg.file.as_mut() {
            let mut log_path = file_cfg.dir.clone();
            let mut name = file_cfg.name.clone();
            name.push('.');
            name.push_str(&file_cfg.ext);
            name.push('.');
            name.push_str(&fmt!("{:02}.gz", file_cfg.num));
            log_path.push(name);
            let mut new = res!(File::create(log_path));
            let old = self.file.as_mut().unwrap();
            res!(old.seek(SeekFrom::Start(0)));
            let old_buf = BufReader::new(old);
            let mut gz = GzEncoder::new(old_buf, Compression::fast());
            res!(io::copy(&mut gz, &mut new));
            // Archive to new compressed file.
            res!(self.file.as_mut().unwrap().set_len(0)); // Return to start.
            file_cfg.num = file_cfg.num.wrapping_add(1); 
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
                    _ => (),
                }
            }
        }
    }

    fn write(
        &mut self,
        level:  LogLevel,
        src:    &Source,
        res:    Outcome<String>,
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
                level,
                &src,
                res,
                console_chan.is_some(),
                self.file.is_some(),
            );
            if let Some(console_chan) = console_chan {
                if let Some(msg) = msg_console_opt {
                    match console_chan.send(Msg::Console(msg.clone())) {
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
                        _ => (),
                    }
                }
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

    fn listen(&mut self) -> bool {
        match self.chan.recv() {
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
                self.write(level, &src, Ok(msg));
                return true;
            }
            Ok(Msg::Base(bot::BaseMsg::Ready)) => msg!("LogBot now ready to receive messages."),
            Ok(Msg::Log { level, src, erropt, msg }) => {
                let msg = if let Some(e) = erropt {
                    fmt!("{} {}", msg, e)
                } else {
                    fmt!("{}", msg)
                };
                self.write(level, &src, Ok(msg));
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
                self.write(LogLevel::Error, &src, Err(e));
            },
            _ => (),
        }
        false
    }
}
