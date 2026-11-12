//! `term::line::LineReader` fed from a buffer and through a pipe, with what it draws read back.
//!
//! The terminal's own part (echo, line editing, Ctrl-D, the ^C it writes) cannot be had without a
//! terminal; what is tested is everything the reader does with the lines and the signal it is
//! given. The interrupt is a real `SIGINT` sent to this process by `kill`, once the listener is in
//! place, which the reader guarantees before `hear_interrupts` returns.

#![cfg(not(target_arch = "wasm32"))]

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_core::term::line::{
    LineEvent,
    LineReader,
};

use std::{
    io::{
        BufReader,
        Cursor,
        Write,
    },
    future::Future,
    process::Command,
    sync::{
        Arc,
        Mutex,
    },
    task::{
        Context,
        Poll,
        Waker,
    },
    time::Duration,
};

/// Output the test can read back while the reader holds it.
#[derive(Clone, Default)]
struct Screen(Arc<Mutex<Vec<u8>>>);

impl Screen {
    fn take(&self) -> Outcome<String> {
        let mut v = lock_mutex!(self.0);
        Ok(String::from_utf8_lossy(&std::mem::take(&mut *v)).into_owned())
    }
}

impl Write for Screen {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        match self.0.lock() {
            Ok(mut v)   => { v.extend_from_slice(b); Ok(b.len()) },
            Err(_)      => Err(std::io::Error::other("screen lock poisoned")),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

/// Polls `r.next()` until `screen` shows `want`, then drops the call part way through, as a
/// `select!` arm not taken is. The reader's thread delivers in its own time, so this waits a
/// bounded while for it.
fn poll_until(r: &mut LineReader<Screen>, screen: &Screen, want: &str) -> Outcome<()> {
    let mut cx = Context::from_waker(Waker::noop());
    let mut f = std::pin::pin!(r.next());
    let mut seen = String::new();
    for _ in 0..500 {
        if let Poll::Ready(e) = f.as_mut().poll(&mut cx) {
            return Err(err!("The reader gave {:?} where it should have waited.", e; Test, Unexpected));
        }
        seen.push_str(&res!(screen.take()));
        if seen == want {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(err!("The screen showed {:?}, not {:?}.", seen, want; Test, Timeout))
}

fn reader(input: &'static str) -> Outcome<LineReader<Screen>> {
    LineReader::new(Cursor::new(input.as_bytes()), Screen::default(), "> ")
}

#[tokio::test]
async fn backslash_lines_join_into_one_entry() -> Outcome<()> {
    let mut r = res!(reader("one \\\ntwo\\\nthree\nfour\r\n"));
    req!(res!(r.next().await), LineEvent::Line("one \ntwo\nthree".to_string()));
    req!(res!(r.next().await), LineEvent::Line("four".to_string()));
    req!(res!(r.next().await), LineEvent::End);
    req!(res!(r.next().await), LineEvent::End, "The end is given again.");
    Ok(())
}

#[tokio::test]
async fn input_ending_mid_entry_gives_the_entry_then_the_end() -> Outcome<()> {
    let mut r = res!(reader("a\\\nb\\\n"));
    req!(res!(r.next().await), LineEvent::Line("a\nb".to_string()));
    req!(res!(r.next().await), LineEvent::End);
    let mut r = res!(reader("last"));
    req!(res!(r.next().await), LineEvent::Line("last".to_string()), "A last line without its newline.");
    req!(res!(r.next().await), LineEvent::End);
    let mut r = res!(reader(""));
    req!(res!(r.next().await), LineEvent::End);
    Ok(())
}

#[tokio::test]
async fn lines_through_a_pipe_arrive_as_written_and_output_goes_above_the_prompt() -> Outcome<()> {
    let (rd, mut wr) = res!(std::io::pipe());
    let screen = Screen::default();
    let mut r = res!(LineReader::new(BufReader::new(rd), screen.clone(), "> "))
        .continuation(". ");

    res!(r.prompt());
    res!(r.prompt());
    req!(res!(screen.take()), "> ".to_string(), "The prompt is drawn once.");
    res!(r.print("news\n"));
    req!(res!(screen.take()), "\r\x1b[Knews\n> ".to_string());
    res!(r.print("part"));
    res!(r.print("ly\n"));
    req!(res!(screen.take()), "\r\x1b[Kpartly\n> ".to_string(),
        "The prompt waits for the output's line to end.");

    res!(wr.write_all(b"first \\\n"));
    res!(poll_until(&mut r, &screen, ". "));
    res!(wr.write_all(b"second\n"));
    req!(res!(r.next().await), LineEvent::Line("first \nsecond".to_string()));

    res!(r.print("after"));
    res!(r.prompt());
    req!(res!(screen.take()), "after\n> ".to_string(),
        "Once an entry is made the prompt is down until asked for.");
    res!(r.hide());
    req!(res!(screen.take()), "\r\x1b[K".to_string());

    drop(wr);
    req!(res!(r.next().await), LineEvent::End);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn ctrl_c_is_an_event_and_discards_the_entry_being_made() -> Outcome<()> {
    let (rd, mut wr) = res!(std::io::pipe());
    let screen = Screen::default();
    let mut r = res!(res!(LineReader::new(BufReader::new(rd), screen.clone(), "> "))
        .continuation(". ")
        .hear_interrupts());
    res!(r.prompt());
    req!(res!(screen.take()), "> ".to_string());
    res!(wr.write_all(b"begun \\\n"));
    res!(poll_until(&mut r, &screen, ". "));
    let out = res!(Command::new("kill")
        .arg("-INT")
        .arg(fmt!("{}", std::process::id()))
        .output());
    req!(out.status.success(), true);
    req!(res!(r.next().await), LineEvent::Interrupt);
    res!(wr.write_all(b"fresh\n"));
    req!(res!(r.next().await), LineEvent::Line("fresh".to_string()),
        "The entry begun before Ctrl-C is gone.");
    drop(wr);
    req!(res!(r.next().await), LineEvent::End);
    Ok(())
}
