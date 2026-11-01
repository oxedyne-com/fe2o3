use crate::email::msg::EmailMessage;

use oxedyne_fe2o3_core::prelude::*;

use std::path::Path;

use tokio::{
    fs::File,
    io::{AsyncBufReadExt, BufReader},
};


pub struct MboxEmailIterator {
    reader: BufReader<File>,
    buffer: Vec<u8>,
}

impl MboxEmailIterator {
    pub async fn new<P: AsRef<Path>>(path: P) -> Outcome<Self> {
        let file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(e) => {
                return Err(err!(e, "While opening mbox file."; IO, File, Read));
            }
        };
        let reader = BufReader::new(file);
        Ok(Self {
            reader,
            buffer: Vec::new(),
        })
    }

    async fn read_line(&mut self) -> Outcome<Option<Vec<u8>>> {
        self.buffer.clear();
        let result = self.reader.read_until(b'\n', &mut self.buffer).await;
        let bytes_read = res!(result, IO, File, Read);
        if bytes_read == 0 {
            Ok(None)
        } else {
            if self.buffer.ends_with(&[b'\r', b'\n']) {
                self.buffer.truncate(self.buffer.len() - 2);
            } else if self.buffer.ends_with(&[b'\n']) {
                self.buffer.truncate(self.buffer.len() - 1);
            }
            Ok(Some(self.buffer.clone()))
        }
    }

    /// Reads the next message's lines, without its "From " separator line or the blank
    /// line mbox puts before the next one.
    async fn read_until_next_from(&mut self) -> Outcome<Option<Vec<Vec<u8>>>> {
        let mut lines = Vec::new();
        let mut line_num: usize = 0;
        loop {
            match self.read_line().await {
                Ok(Some(line)) => {
                    debug!("{:05} line={}", line_num, String::from_utf8_lossy(&line));
                    line_num += 1;
                    if line.starts_with(b"From ") {
                        if lines.is_empty() {
                            continue; // This message's own separator.
                        }
                        break; // The next message's separator.
                    }
                    lines.push(line);
                }
                Ok(None) => break,
                Err(e) => return Err(e),
            }
        }
        if lines.last().map_or(false, |l| l.is_empty()) {
            lines.pop();
        }
        if lines.is_empty() {
            Ok(None)
        } else {
            Ok(Some(lines))
        }
    }

    /// Renders mbox lines as an SMTP DATA body, which `EmailMessage::read` parses: the
    /// mboxrd quoting of a "From " line is undone (one '>' off a `>+From ` line), each
    /// line ends CRLF, a leading '.' is doubled (RFC 5321 4.5.2) and ".\r\n" ends it.
    fn to_wire(lines: Vec<Vec<u8>>) -> Vec<u8> {
        let mut wire = Vec::new();
        for line in lines {
            let quotes = line.iter().take_while(|b| **b == b'>').count();
            let line = if quotes > 0 && line[quotes..].starts_with(b"From ") {
                &line[1..]
            } else {
                &line[..]
            };
            if line.starts_with(b".") {
                wire.push(b'.');
            }
            wire.extend_from_slice(line);
            wire.extend_from_slice(b"\r\n");
        }
        wire.extend_from_slice(b".\r\n");
        wire
    }
}

impl Iterator for MboxEmailIterator {
    type Item = Outcome<EmailMessage>;

    fn next(&mut self) -> Option<Self::Item> {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(_) => return None,
        };

        runtime.block_on(async {
            match self.read_until_next_from().await {
                Ok(Some(lines)) => {
                    let cursor = std::io::Cursor::new(Self::to_wire(lines));
                    let mut reader = tokio::io::BufReader::new(cursor);
                    match EmailMessage::read(&mut reader).await {
                        Ok(email) => Some(Ok(email)),
                        Err(e) => Some(Err(e)),
                    }
                }
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            }
        })
    }
}
