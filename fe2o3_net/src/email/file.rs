use crate::email::msg::{
    EmailMessage,
    EmailParser,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    fs::File,
    io::{
        BufRead,
        BufReader,
    },
    path::Path,
};


/// Iterates the messages of an mboxrd file. Each message starts at a "From "
/// envelope line, which is dropped, as is the blank line that separates it from
/// the next message; a body line escaped as ">From " (any number of '>') loses
/// one '>'.
pub struct MboxEmailIterator {
    reader:     BufReader<File>,
    started:    bool, // Has the first envelope line been passed?
    done:       bool,
}

impl MboxEmailIterator {

    pub fn new<P: AsRef<Path>>(path: P) -> Outcome<Self> {
        let path = path.as_ref();
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) => return Err(err!(e,
                "While opening mbox file {:?}.", path; IO, File, Read)),
        };
        Ok(Self {
            reader:     BufReader::new(file),
            started:    false,
            done:       false,
        })
    }

    /// Reads one line without its terminator, or `None` at the end of the file.
    fn read_line(&mut self) -> Outcome<Option<String>> {
        let mut bfr = Vec::new();
        let byts_read = res!(self.reader.read_until(b'\n', &mut bfr), IO, File, Read);
        if byts_read == 0 {
            return Ok(None);
        }
        if bfr.ends_with(b"\n") {
            bfr.pop();
        }
        if bfr.ends_with(b"\r") {
            bfr.pop();
        }
        Ok(Some(String::from_utf8_lossy(&bfr).into_owned()))
    }

    fn read_message(&mut self) -> Outcome<Option<EmailMessage>> {
        let mut lines: Vec<String> = Vec::new();
        loop {
            match res!(self.read_line()) {
                None => {
                    self.done = true;
                    break;
                }
                Some(line) if line.starts_with("From ") => {
                    if self.started {
                        break;
                    }
                    self.started = true;
                }
                Some(line) => {
                    if !self.started {
                        return Err(err!(
                            "Mbox file does not start with a 'From ' envelope line: {:?}.",
                            line; Invalid, Input, File));
                    }
                    lines.push(line);
                }
            }
        }
        if !self.started {
            return Ok(None); // Empty file.
        }
        // The blank line before the next envelope separates, it is not body.
        if lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.pop();
        }
        let mut parser = EmailParser::default();
        for line in &lines {
            let unescaped = if line.starts_with('>') && line.trim_start_matches('>').starts_with("From ") {
                &line[1..]
            } else {
                line.as_str()
            };
            res!(parser.push_line(unescaped));
        }
        parser.finish().map(Some)
    }
}

impl Iterator for MboxEmailIterator {
    type Item = Outcome<EmailMessage>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.read_message() {
            Ok(Some(email)) => Some(Ok(email)),
            Ok(None)        => None,
            Err(e)          => {
                self.done = true; // A fault ends the iteration rather than repeating.
                Some(Err(e))
            }
        }
    }
}
