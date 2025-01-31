use crate::lib_tui::{
    text::typ::{
        TextType,
        HighlightType,
    },
};

use oxedize_fe2o3_core::{
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
        console::{
            LoggerConsole,
        },
    },
    thread::{
        thread_channel,
        ThreadController,
    },
};
use oxedize_fe2o3_text::lines::TextLines;

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
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    pub log_chan:   Simplex<Msg<ETAG>>,
    pub app_log:    Arc<RwLock<TextLines<TextType, HighlightType>>>,
}

impl<ETAG: GenTag> LoggerConsole<ETAG> for AppLoggerConsole<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    fn go(&mut self) -> ThreadController<Msg<ETAG>> {
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

    fn listen(&self) {
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
                        match self.app_log.write() {
                            Ok(mut unlocked) => {
                                let ref mut log = *unlocked;
                                log.append_string(msg);
                            }
                            Err(e) => {
                                let msg2 = errmsg!(
                                    "{}: Could not access app log to write message: {}", e, msg,
                                );
                                msg!("{}", msg2);
                            }
                        }
                    }
                    break;
                }
                Msg::Console(msg) => {
                    match self.app_log.write() {
                        Ok(mut unlocked) => {
                            let ref mut log = *unlocked;
                            log.append_string(msg);
                        }
                        Err(e) => {
                            let msg2 = errmsg!(
                                "{}: Could not access app log to write message: {}", e, msg,
                            );
                            msg!("{}", msg2);
                        }
                    }
                }
                _ => {
                    let msg = err!(
                        "Unexpected message type: {:?}", msg;
                        Bug, Unexpected, Input).to_string();
                    match self.app_log.write() {
                        Ok(mut unlocked) => {
                            let ref mut log = *unlocked;
                            log.append_string(msg);
                        }
                        Err(e) => {
                            let msg2 = errmsg!(
                                "{}: Could not access app log to write message: {}", e, msg,
                            );
                            msg!("{}", msg2);
                        }
                    }
                }
            }
        }
    }
}

impl<ETAG: GenTag + fmt::Debug + Send + Sync> AppLoggerConsole<ETAG>
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
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
}
