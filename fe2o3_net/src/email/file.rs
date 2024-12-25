use crate::email::msg::EmailMessage;

use oxedize_fe2o3_core::prelude::*;

use tokio::{
    fs::File,
    io::{AsyncBufReadExt, BufReader},
};

pub struct MboxEmailIterator {
    reader: BufReader<File>,
    buffer: Vec<u8>,
}

impl MboxEmailIterator {
    pub async fn new(file_path: &str) -> Outcome<Self> {
        let file = match tokio::fs::File::open(file_path).await {
            Ok(file) => file,
            Err(e) => {
                return Err(err!(
                    e,
                    errmsg!("While opening mbox file.",),
                    IO,
                    File,
                    Read
                ))
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

    async fn read_until_next_from(&mut self) -> Outcome<Option<Vec<u8>>> {
        let mut email_content = Vec::new();
        let mut line_num: usize = 0;
        loop {
            match self.read_line().await {
                Ok(Some(line)) => {
                    debug!("{:05} line={}", line_num, String::from_utf8_lossy(&line));
                    line_num += 1;
                    if line.starts_with(b"From ") {
                        if !email_content.is_empty() {
                            break;
                        }
                    }
                    email_content.extend_from_slice(&line);
                    email_content.extend_from_slice(&[b'\n']);
                }
                Ok(None) => {
                    if email_content.is_empty() {
                        return Ok(None);
                    } else {
                        break;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(Some(email_content))
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
                Ok(Some(content)) => {
                    let cursor = std::io::Cursor::new(content);
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
