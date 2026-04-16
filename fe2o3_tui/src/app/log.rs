use crate::lib_tui::{
    text::typ::{
        TextType,
        HighlightType,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    channels::{
        simplex,
        Simplex,
    },
    log::{
        bot::{
            LogBot,
            Msg,
        },
    },
    thread::{
        thread_channel,
        ThreadController,
    },
};
use oxedyne_fe2o3_text::lines::TextLines;

use std::{
    fmt,
    marker::{
        Send,
        Sync,
    },
    sync::{
        Arc,
        Mutex,
        RwLock,
    },
    thread,
};


#[derive(Clone, Debug)]
pub struct AppLoggerConsole<ETAG: GenTag>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub log_chan:   Simplex<Msg<ETAG>>,
    pub app_log:    Arc<RwLock<TextLines<TextType, HighlightType>>>,
}

impl<ETAG: GenTag + fmt::Debug + Send + Sync + 'static> AppLoggerConsole<ETAG>
    where oxedyne_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    /// Creates a new app logger console sharing the supplied app log buffer.
    pub fn new(
        app_log: Arc<RwLock<TextLines<TextType, HighlightType>>>,
    )
        -> Self
    {
        Self {
            log_chan: simplex(),
            app_log,
        }
    }

    /// Spawns the listener thread and returns a controller that owns its channel and handle.
    pub fn go(&mut self) -> ThreadController<Msg<ETAG>> {
        // Logger console thread.  Listens for messages from the LogBot.
        let (semaphore, _sentinel) = thread_channel();
        let semaphore_clone = semaphore.clone();
        let logger_clone = self.clone();
        let handle = thread::spawn(move || {
            semaphore.touch();
            logger_clone.listen();
        });
        ThreadController::new(
            self.log_chan.clone(),
            Arc::new(Mutex::new(Some(handle))),
            semaphore_clone,
        )
    }

    /// Drains the log channel, appending each message to the shared app log.
    pub fn listen(&self) {
        while let Ok(msg) = self.log_chan.recv() {
            match msg {
                Msg::Finish(src) => {
                    let msg = fmt!("Finish message received, logger console thread finishing now.");
                    if let (Some(msg), _) = LogBot::format_msg(
                        LogLevel::Warn,
                        &src,
                        Ok(msg),
                        true,
                        false,
                    ) {
                        self.append_to_log(msg);
                    }
                    break;
                }
                Msg::Console(_stream, msg) => {
                    // Stream key currently unused; the TUI surfaces a single stream.
                    self.append_to_log(msg);
                }
                _ => {
                    let msg = err!(
                        "Unexpected message type: {:?}", msg;
                        Bug, Unexpected, Input).to_string();
                    self.append_to_log(msg);
                }
            }
        }
    }

    /// Appends a pre-formatted line to the shared app log, or reports a lock failure via `msg!`.
    fn append_to_log(&self, line: String) {
        match self.app_log.write() {
            Ok(mut unlocked) => {
                let ref mut log = *unlocked;
                log.append_string(line);
            }
            Err(e) => {
                let msg2 = errmsg!(
                    "{}: Could not access app log to write message: {}", e, line,
                );
                msg!("{}", msg2);
            }
        }
    }
}
