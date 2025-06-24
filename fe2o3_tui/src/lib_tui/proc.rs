use crate::lib_tui::{
    draw::text::TextLines,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    channels::Simplex,
};

use std::{
    fmt::Debug,
    marker::PhantomData,
    io::{
        BufRead,
        Write,
    },
    sync::{
        Arc,
        RwLock,
    },
    thread,
};


#[derive(Clone, Debug)]
pub struct Process<
    R: BufRead + Debug + Send + Sync,
    W: Write + Debug + Send + Sync,
> {
    label:          String,
    stream_in:      Arc<RwLock<W>>,
    stream_out:     Arc<RwLock<R>>,
    output:         Arc<RwLock<TextLines>>,
    _phantom:       PhantomData<(R, W)>,
}

impl<
    R: BufRead + Debug + Send + Sync + 'static,
    W: Write + Debug + Send + Sync + 'static,
>
    Process<R, W>
{
    fn new(
        label:      String,
        stream_in:  W,
        stream_out: R,
    )
        -> Self
    {
        Self {
            label,
            stream_in:  Arc::new(RwLock::new(stream_in)),
            stream_out: Arc::new(RwLock::new(stream_out)),
            output:     Arc::new(RwLock::new(TextLines::default())),
            _phantom:   PhantomData,
        }
    }

    fn write(&self, byts: &[u8]) -> Outcome<()> {
        let mut stream = lock_write!(self.stream_in);
        res!(stream.write_all(byts));
        res!(stream.flush());
        Ok(())
    }

    fn read(&self, buf: &mut String) -> Outcome<usize> {
        let mut stream = lock_write!(self.stream_out);
        let byts = res!(stream.read_line(buf));
        Ok(byts)
    }
}

#[derive(Clone, Debug)]
pub enum Msg<
    R: BufRead + Debug + Send + Sync + 'static,
    W: Write + Debug + Send + Sync + 'static,
> {
    AddProcess(Process<R, W>),
    Input(usize, String),
}

pub struct ProcessManager<
    R: BufRead + Debug + Send + Sync + 'static,
    W: Write + Debug + Send + Sync + 'static,
> {
    procs:      Vec<Process<R, W>>,
    chan_in:    Simplex<Msg<R, W>>,
}

impl<
    R: BufRead + Debug + Send + Sync + 'static,
    W: Write + Debug + Send + Sync + 'static,
>
    ProcessManager<R, W>
{
    fn new(chan_in: Simplex<Msg<R, W>>) -> Self {
        Self {
            procs: Vec::new(),
            chan_in,
        }
    }

    fn add_process(&mut self, proc: Process<R, W>) -> Outcome<()> {
        self.chan_in.send(Msg::AddProcess(proc))
    }

    fn write(&self, ind: usize, input: String) -> Outcome<()> {
        self.chan_in.send(Msg::Input(ind, input))
    }

    //fn listen(&mut self) {
    //    let procs = self.procs.clone();
    //    thread::spawn(move || {
    //        loop {
    //            for proc in &procs {
    //                let mut buf = String::new();
    //                let byts = res!(proc.read_output(&mut buf));
    //                if bytes_read > 0 {
    //                    let mut output = lock_write!(proc.output);
    //                    output.add_text(buf.trim());
    //                }
    //                buf.clear();
    //            }
    //        }
    //    });
    //}
}
