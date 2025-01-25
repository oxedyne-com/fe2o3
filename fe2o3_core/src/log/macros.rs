#[macro_export]
/// Log a message if the provided log level is less than or equal to the current log level.  Note
/// that an error message can be logged this way but you won't be able to provide an actual error
/// object.  Instead, user `error!` directly.
macro_rules! log {
    ($level:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: $level,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::new(),
        });
    };
    ($level:expr, $stream:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: $level,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::($stream),
        });
    };
}

#[macro_export]
/// Log an error message by sending it to the `LogBot` instance. This macro has a different
/// structure to the others, accepting either an `oxedize_fe2o3_core::error::Error` or an `Error` with
/// parameters for string formatting.  In other words, it requires the caller to pass an `Error` or
/// create one.
macro_rules! error {
    ($e:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Error,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: Some($e),
            msg: fmt!($($arg)*),
        });
    };
    ($e:expr, $stream:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Error,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: Some($e),
            msg: fmt!($($arg)*),
            stream: String::($stream),
        });
    };
    ($e:expr) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Error,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: Some($e),
            msg: fmt!(""),
            stream: String::new(),
        });
    };
}

#[macro_export]
/// Log a fault message by sending it to the `LogBot` instance. This is just a workaround because
/// getting the `error!` macro to also recognise a message without an error object is hard.  A
/// `fault!` displays just like an `error!`.
macro_rules! fault {
    ($($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Error,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::new(),
        });
    };
    ($stream:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Error,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::($stream),
        });
    };
}

#[macro_export]
/// Log a warning message by sending it to the `LogBot` instance.
macro_rules! warn {
    ($($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Warn,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::new(),
        });
    };
    ($stream:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Warn,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::($stream),
        });
    };
}

#[macro_export]
/// Log an info message by sending it to the `LogBot` instance.
macro_rules! info {
    ($($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Info,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::new(),
        });
    };
    ($stream:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Info,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::($stream),
        });
    };
}

#[macro_export]
/// Log a test message by sending it to the `LogBot` instance.
macro_rules! test {
    ($($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Test,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::new(),
        });
    };
    ($stream:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Test,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::($stream),
        });
    };
}

#[macro_export]
/// Log a debug message by sending it to the `LogBot` instance.
macro_rules! debug {
    ($($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Debug,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::new(),
        });
    };
    ($stream:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Debug,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::($stream),
        });
    };
}

#[macro_export]
/// Log a trace message by sending it to the `LogBot` instance.
macro_rules! trace {
    ($($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Trace,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::new(),
        });
    };
    ($stream:expr, $($arg:tt)*) => {
        LOG.send_in(bot_log::Msg::Log {
            level: LogLevel::Trace,
            src: oxedize_fe2o3_core::log::base::Source {
                tid: std::thread::current().id(),
                file: file!(),
                line: line!(),
            },
            erropt: None,
            msg: fmt!($($arg)*),
            stream: String::($stream),
        });
    };
}

#[macro_export]
/// Send a finish message to the `LogBot` instance.
macro_rules! log_finish {
    () => {
        LOG.send_in(bot_log::Msg::Finish(oxedize_fe2o3_core::log::base::Source {
            tid: std::thread::current().id(),
            file: file!(),
            line: line!(),
        }));
    }
}

#[macro_export]
/// Wait for the `Logger` singleton instance thread to terminate.  The caller needs to accomodate
/// several possible failure modes in the `Outcome`.  But `std::sync::MutexGuard` is explicitly
/// `!Send` so we need to handle errors with special care using `LogWaitError` wrapping.
macro_rules! log_in_finish_wait {
    () => {
        // Send finish message to the incoming channel of the logger, and wait for the logbot
        // thread to finish.
        LOG.send_in(bot_log::Msg::Finish(oxedize_fe2o3_core::log::base::Source {
            tid: std::thread::current().id(),
            file: file!(),
            line: line!(),
        }));
        if let Some(handle) = {
            match LOG.chan_in.hopt.lock() {
                Ok(mut inner) => inner.take(),
                Err(e) => {
                    let err = oxedize_fe2o3_core::log::base::LogWaitError::LockError(fmt!("{}", e));
                    return Err(Error::Local(ErrMsg {
                        tags: &[ErrTag::Lock],
                        msg: fmt!("{}", err),
                    }));
                }
            }
        } {
            if let Err(e) = handle.join() {
                let err = oxedize_fe2o3_core::log::base::LogWaitError::JoinError(fmt!("{:?}", e));
                return Err(Error::Local(ErrMsg {
                    tags: &[ErrTag::Thread],
                    msg: fmt!("{}", err),
                }));
            }
        }
    }
}

#[macro_export]
/// Wait for the current `LoggerConsole` implementation thread to terminate.  The caller needs to
/// accomodate several possible failure modes in the `Outcome`.  But `std::sync::MutexGuard` is
/// explicitly `!Send` so we need to handle errors with special care using `LogWaitError` wrapping.
macro_rules! log_out_finish_wait {
    () => {
        // Send finish message to the outgoing channel of the logger, and wait for its thread to
        // finish.
        res!(LOG.send_out(bot_log::Msg::Finish(oxedize_fe2o3_core::log::base::Source {
            tid: std::thread::current().id(),
            file: file!(),
            line: line!(),
        })));
        {   // Important to enclose the locking to ensure its release.
            let unlocked_chan_out = lock_write!(LOG.chan_out);
            if let Some(handle) = {
                let x = match unlocked_chan_out.hopt.lock() {
                    Ok(mut inner) => inner.take(),
                    Err(e) => {
                        let err = oxedize_fe2o3_core::log::base::LogWaitError::LockError(fmt!("{}", e));
                        return Err(Error::Local(ErrMsg {
                            tags: &[ErrTag::Lock],
                            msg: fmt!("{}", err),
                        }));
                    }
                };
                x // KTCH Keeping the Compiler Happy (TM)
            } {
                if let Err(e) = handle.join() {
                    let err = oxedize_fe2o3_core::log::base::LogWaitError::JoinError(fmt!("{:?}", e));
                    return Err(Error::Local(ErrMsg {
                        tags: &[ErrTag::Thread],
                        msg: fmt!("{}", err),
                    }));
                }
            }
        }
    }
}

#[macro_export]
/// Wait for the `Logger` singleton instance thread to terminate.  The caller needs to accomodate
/// several possible failure modes in the `Outcome`.  But `std::sync::MutexGuard` is explicitly
/// `!Send` so we need to handle errors with special care using `LogWaitError` wrapping.
macro_rules! log_finish_wait {
    () => {
        log_in_finish_wait!();
        log_out_finish_wait!();
    }
}

#[macro_export]
/// Set a new log level by accessing the global `LOG` instance configuration.  Because this is done
/// via its write lock, this macro can return an error.
macro_rules! set_log_level {
    ($level:literal) => {
        {
            let mut unlocked_cfg = lock_write!(LOG.cfg);
            unlocked_cfg.level = res!(LogLevel::from_str($level));
        }
    }
}

#[macro_export]
/// Get the current log level by accessing the global `LOG` instance configuration.  Because this is done
/// via its read lock, this macro can return an error.
macro_rules! get_log_level {
    () => {
        {
            let unlocked_cfg = lock_read!(LOG.cfg);
            unlocked_cfg.level
        }
    }
}

#[macro_export]
/// Set a new log `oxedize_fe2o3_core::log::bot::Config`.  Because this is done via its write lock, this
/// macro can return an error.
macro_rules! set_log_config {
    ($cfg:expr) => {
        {
            let mut unlocked_cfg = lock_write!(LOG.cfg);
            *unlocked_cfg = $cfg;
        }
        LOG.send_in(bot_log::Msg::Update(oxedize_fe2o3_core::log::base::Source {
            tid: std::thread::current().id(),
            file: file!(),
            line: line!(),
        }));
    }
}

#[macro_export]
/// Get the current log `oxedize_fe2o3_core::log::bot::Config`.  Because this is done via its write lock,
/// this macro can return an error.
macro_rules! get_log_config {
    () => {
        {
            let unlocked_cfg = lock_read!(LOG.cfg);
            unlocked_cfg.clone()
        }
    }
}

#[macro_export]
/// Set a new `SimplexThread` to handle console messages sent out by the `Logger`.  Because this is
/// done via its write lock, this macro can return an error.
macro_rules! set_log_out {
    ($simthread:expr) => {
        log_out_finish_wait!();
        let chan_clone = $simthread.chan.clone();
        {
            let mut unlocked_chan_out = lock_write!(LOG.chan_out);
            *unlocked_chan_out = $simthread;
        }
        {
            let mut unlocked_cfg = lock_write!(LOG.cfg);
            (*unlocked_cfg).console = Some(chan_clone);
        }
    }
}

#[macro_export]
/// Get the current log file path.  Because this is done via its write lock, this macro can return
/// an error.
macro_rules! get_log_file_path {
    () => {
        {
            let unlocked_cfg = lock_read!(LOG.cfg);
            match &unlocked_cfg.file {
                Some(fcfg) => {
                    Some(fcfg.path())
                }
                None => None,
            }
        }
    }
}
