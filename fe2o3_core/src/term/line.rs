//! Lines typed at a terminal, read beside other work, with output printed above the prompt.
//!
//! The terminal stays in its ordinary cooked mode, so its own line editing (erase, word erase,
//! kill, reprint) works and nothing need be restored if the program dies. The cost is that the
//! text of a line is the terminal's until Enter is pressed: [`LineReader::print`] clears the
//! prompt line, writes its text and draws the prompt again, but cannot draw again what was
//! partly typed. That text is still held by the terminal, Enter still sends it, and the
//! terminal's reprint key (Ctrl-R, where `stty` has `rprnt` set) shows it once more.
//!
//! The lines are read by a thread of their own, which blocks on its input, and handed over a
//! flume channel, so an asynchronous caller awaits [`LineReader::next`] under any executor, in
//! `select!` beside its other work, and a runtime that ends need not wait for the read. Ctrl-C is
//! heard, when asked for, through tokio's `signal` module on a second thread with a runtime of its
//! own, as `stop.rs` hears it, so the caller's runtime needs no signal driver and this crate keeps
//! `#![forbid(unsafe_code)]`.

use crate::{
    prelude::*,
    channels::{
        unbounded,
        Receiver,
        Sender,
    },
};

use std::{
    io::{
        BufRead,
        IsTerminal,
        Stdout,
        Write,
    },
    sync::mpsc,
    thread,
};

const CLEAR_LINE: &str = "\r\x1b[K"; // fixed: ECMA-48 carriage return, then erase in line

/// What the person at the terminal did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LineEvent {
    Line(String),   // an entry, its continuation lines joined by '\n'
    Interrupt,      // Ctrl-C, which the terminal answers by discarding what was typed
    End,            // end of input, Ctrl-D on an empty line at a terminal
}

/// What the reading threads hand over.
enum Heard {
    Line(String),
    End,
    Interrupt,
    Failed(Error<ErrTag>),
}

/// Reads entries from a terminal, or any line input, without holding up an asynchronous caller.
///
/// A line ending in a backslash continues the entry on the next line; the backslash is dropped
/// and the lines are joined by '\n'. Input ending part way through an entry gives the entry as
/// it stands, then the end.
pub struct LineReader<W: Write> {
    rx:         Receiver<Heard>,
    tx:         Sender<Heard>,  // for the interrupt listener
    out:        W,
    prompt:     String,
    cont:       String,         // the prompt for a continuation line
    prompting:  bool,           // are prompts drawn at all?
    up:         bool,           // the caller has the prompt up
    drawn:      bool,           // the current line holds the prompt, or the terminal's ^C
    midline:    bool,           // printed output has not ended its line
    partial:    String,         // an entry continued with a trailing backslash
    hearing:    bool,           // interrupts reach the channel
    ended:      bool,           // the input has ended
    told:       bool,           // the end has been given to the caller
}

impl LineReader<Stdout> {

    /// Reads standard input and writes to standard output, hearing Ctrl-C as
    /// [`LineEvent::Interrupt`] rather than as the end of the process. Prompts are drawn only
    /// when both are a terminal, so piped input reads the same way unprompted. The reading thread
    /// holds the lock on standard input for the life of the process.
    pub fn stdio(prompt: &str) -> Outcome<Self> {
        let (tx, rx) = unbounded();
        let tx2 = tx.clone();
        let _reader = res!(thread::Builder::new()
            .name("fe2o3-line".to_string())
            .spawn(move || read(std::io::stdin().lock(), tx2)), Thread, Init);
        let prompting = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        let r = Self::with_channel(rx, tx, std::io::stdout(), prompt, prompting);
        r.hear_interrupts()
    }
}

impl<W: Write> LineReader<W> {

    /// Reads `input` on a thread of its own and draws prompts and output on `out`.
    pub fn new<R: BufRead + Send + 'static>(input: R, out: W, prompt: &str) -> Outcome<Self> {
        let (tx, rx) = unbounded();
        let tx2 = tx.clone();
        let _reader = res!(thread::Builder::new()
            .name("fe2o3-line".to_string())
            .spawn(move || read(input, tx2)), Thread, Init);
        Ok(Self::with_channel(rx, tx, out, prompt, true))
    }

    fn with_channel(rx: Receiver<Heard>, tx: Sender<Heard>, out: W, prompt: &str, prompting: bool)
        -> Self
    {
        Self {
            rx,
            tx,
            out,
            prompt:     prompt.to_string(),
            cont:       String::new(),
            prompting,
            up:         false,
            drawn:      false,
            midline:    false,
            partial:    String::new(),
            hearing:    false,
            ended:      false,
            told:       false,
        }
    }

    /// Sets the prompt drawn for a continuation line, by default none.
    pub fn continuation(mut self, prompt: &str) -> Self {
        self.cont = prompt.to_string();
        self
    }

    /// Hears Ctrl-C (`SIGINT`) as [`LineEvent::Interrupt`] from now until the process ends. The
    /// listener is in place when this returns. Once heard, Ctrl-C no longer ends the process,
    /// even after the reader is dropped, since a signal handler cannot be taken back.
    pub fn hear_interrupts(mut self) -> Outcome<Self> {
        if self.hearing {
            return Ok(self);
        }
        let tx = self.tx.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let _listener = res!(thread::Builder::new()
            .name("fe2o3-line-int".to_string())
            .spawn(move || listen(tx, ready_tx)), Thread, Init);
        match ready_rx.recv() {
            Ok(Ok(()))  => (),
            Ok(Err(e))  => return Err(e),
            Err(e)      => return Err(err!(e,
                "The interrupt listener ended before it was in place."; Thread, Init)),
        }
        self.hearing = true;
        Ok(self)
    }

    /// Waits for the next entry, interrupt or end of input. Cancel-safe: a call dropped part way,
    /// as a `select!` arm not taken is, loses nothing. After the end, a further call gives the
    /// end again, or waits for an interrupt while interrupts are heard.
    pub async fn next(&mut self) -> Outcome<LineEvent> {
        loop {
            if self.ended && (!self.told || !self.hearing) {
                self.told = true;
                return Ok(LineEvent::End);
            }
            let heard = match self.rx.recv_async().await {
                Ok(h)   => h,
                Err(_)  => {
                    // Every sender has gone, so nothing more can arrive.
                    self.hearing = false;
                    Heard::End
                },
            };
            match heard {
                Heard::Line(line) => {
                    if self.prompting {
                        // Enter has taken the terminal to a fresh line.
                        self.drawn = false;
                        self.midline = false;
                    }
                    match line.strip_suffix('\\') {
                        Some(head) => {
                            self.partial.push_str(head);
                            self.partial.push('\n');
                            if self.up {
                                res!(self.draw());
                            }
                        },
                        None => {
                            self.up = false;
                            let mut entry = std::mem::take(&mut self.partial);
                            entry.push_str(&line);
                            return Ok(LineEvent::Line(entry));
                        },
                    }
                },
                Heard::Interrupt => {
                    self.partial.clear();
                    self.up = false;
                    // The terminal has written ^C where the cursor was.
                    self.drawn = self.prompting && !self.midline;
                    return Ok(LineEvent::Interrupt);
                },
                Heard::End => {
                    self.ended = true;
                    self.up = false;
                    if !self.partial.is_empty() {
                        let mut entry = std::mem::take(&mut self.partial);
                        entry.pop();
                        return Ok(LineEvent::Line(entry));
                    }
                },
                Heard::Failed(e) => return Err(e),
            }
        }
    }

    /// Puts the prompt up, on a line of its own, until an entry is made or [`Self::hide`].
    pub fn prompt(&mut self) -> Outcome<()> {
        if self.up && self.drawn {
            return Ok(());
        }
        self.up = true;
        self.draw()
    }

    /// Takes the prompt down, clearing its line.
    pub fn hide(&mut self) -> Outcome<()> {
        self.up = false;
        if self.drawn {
            self.drawn = false;
            res!(self.out.write_all(CLEAR_LINE.as_bytes()), IO, Write);
            res!(self.out.flush(), IO, Write);
        }
        Ok(())
    }

    /// Writes `text` above the prompt, which is drawn again below it once the text ends its
    /// line. Text that does not end its line, as streamed output may not, is continued by the
    /// next call.
    pub fn print(&mut self, text: &str) -> Outcome<()> {
        if text.is_empty() {
            return Ok(());
        }
        if self.drawn {
            self.drawn = false;
            res!(self.out.write_all(CLEAR_LINE.as_bytes()), IO, Write);
        }
        res!(self.out.write_all(text.as_bytes()), IO, Write);
        self.midline = !text.ends_with('\n');
        if self.up && !self.midline {
            return self.draw();
        }
        res!(self.out.flush(), IO, Write);
        Ok(())
    }

    /// Draws the prompt, or the continuation prompt part way through an entry.
    fn draw(&mut self) -> Outcome<()> {
        if !self.prompting {
            return Ok(());
        }
        if self.drawn {
            res!(self.out.write_all(CLEAR_LINE.as_bytes()), IO, Write);
        }
        if self.midline {
            res!(self.out.write_all(b"\n"), IO, Write);
            self.midline = false;
        }
        let p = if self.partial.is_empty() { &self.prompt } else { &self.cont };
        res!(self.out.write_all(p.as_bytes()), IO, Write);
        res!(self.out.flush(), IO, Write);
        self.drawn = true;
        Ok(())
    }
}

/// The reading thread's whole life: a line at a time until the input ends or nobody listens.
fn read<R: BufRead>(mut input: R, tx: Sender<Heard>) {
    loop {
        let heard = match read_line(&mut input) {
            Ok(Some(line))  => Heard::Line(line),
            Ok(None)        => Heard::End,
            Err(e)          => Heard::Failed(e),
        };
        let last = !matches!(heard, Heard::Line(_));
        if tx.send(heard).is_err() || last {
            return;
        }
    }
}

/// One line without its ending, or `None` at the end of input. Bytes that are not UTF-8 are
/// replaced rather than refused, so one stray byte does not end the input.
fn read_line<R: BufRead>(input: &mut R) -> Outcome<Option<String>> {
    let mut buf = Vec::new();
    if res!(input.read_until(b'\n', &mut buf), IO, Read) == 0 {
        return Ok(None);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// The interrupt listener's whole life, on a runtime of its own, saying when it is in place.
fn listen(tx: Sender<Heard>, ready: mpsc::Sender<Outcome<()>>) {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt)  => rt,
        Err(e)  => {
            let _ = ready.send(Err(err!(e,
                "Building the interrupt listener's runtime."; Init, System)));
            return;
        },
    };
    rt.block_on(interrupts(tx, ready));
}

#[cfg(unix)]
async fn interrupts(tx: Sender<Heard>, ready: mpsc::Sender<Outcome<()>>) {
    use tokio::signal::unix::{
        signal,
        SignalKind,
    };
    let mut int = match signal(SignalKind::interrupt()) {
        Ok(s)   => s,
        Err(e)  => {
            let _ = ready.send(Err(err!(e, "Listening for SIGINT."; Init, System)));
            return;
        },
    };
    let _ = ready.send(Ok(()));
    while int.recv().await.is_some() {
        if tx.send(Heard::Interrupt).is_err() {
            return;
        }
    }
}

#[cfg(windows)]
async fn interrupts(tx: Sender<Heard>, ready: mpsc::Sender<Outcome<()>>) {
    let mut int = match tokio::signal::windows::ctrl_c() {
        Ok(s)   => s,
        Err(e)  => {
            let _ = ready.send(Err(err!(e, "Listening for Ctrl-C."; Init, System)));
            return;
        },
    };
    let _ = ready.send(Ok(()));
    while int.recv().await.is_some() {
        if tx.send(Heard::Interrupt).is_err() {
            return;
        }
    }
}

#[cfg(not(any(unix, windows)))]
async fn interrupts(tx: Sender<Heard>, ready: mpsc::Sender<Outcome<()>>) {
    let _ = ready.send(Ok(()));
    while tokio::signal::ctrl_c().await.is_ok() {
        if tx.send(Heard::Interrupt).is_err() {
            return;
        }
    }
}
