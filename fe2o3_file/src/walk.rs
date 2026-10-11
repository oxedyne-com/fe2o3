//! A directory walk that honours nested ignore files, over any listing source.
//!
//! The walk is a pull-based state machine, [`Walk`], with no I/O of its own. The caller asks
//! [`Walk::next`] what to do and answers each request: a [`Step::List`] with [`Walk::listed`], a
//! [`Step::Read`] with [`Walk::read`]. That keeps every await point in the caller, so a browser
//! engine listing the origin-private file system or a File System Access handle drives the same
//! walk as a disk source, awaiting its own listings between steps and yielding to its event loop
//! whenever it likes:
//!
//! ```ignore
//! let mut walk = Walk::new(20_000);
//! loop {
//!     match res!(walk.next()) {
//!         Step::List(dir)     => res!(walk.listed(list_dir(&dir).await)),
//!         Step::Read(path)    => res!(walk.read(read_file(&path).await)),
//!         Step::Entry(entry)  => found.push(entry),
//!         Step::Stopped(rep)  => { report = Some(rep); break; }
//!         Step::Done          => break,
//!     }
//! }
//! ```
//!
//! A synchronous source implements [`WalkSource`] and hands itself to [`Walk::run`]; [`Disk`] is
//! that source over `std::fs`.
//!
//! The rules are git's. Each directory's `.gitignore` and `.oreignore` (by default) speak for
//! paths beneath that directory, relative to it; a deeper file outranks a shallower one, a later
//! name in the list outranks an earlier one in the same directory, and within one file the last
//! matching pattern decides, so `!` re-includes. An ignored directory is never listed, which is
//! both git's rule that nothing inside it can be re-included and the point of the walk: the cost
//! of `.ore/` or `node_modules/` is one entry, not every entry beneath. Beneath every file lie
//! built-in defaults ([`WALK_DEFAULTS`]), which a repository's own `!node_modules/` overrides.
//!
//! Every listed entry, kept or ignored, spends one unit of the entry budget. When it runs out the
//! walk ends with a [`WalkBudget`] saying how far it got, so a caller can tell a search that found
//! nothing from one that stopped early.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::glob::IgnoreFile;

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::VecDeque,
    fs,
    io,
    path::PathBuf,
};


// Directories no walk wants by default: version-control stores, tool state, Syncthing's history and
// dependency or build output.  Laid beneath every ignore file, so a repository can re-include one.
pub const WALK_DEFAULTS: &[&str] = &[
	".git/",			// git
	".hg/",				// Mercurial
	".svn/",			// Subversion
	".ore/",			// Ore
	".claude/",			// agent state, including whole worktree copies
	".stversions/",		// Syncthing's old versions
	"node_modules/",	// npm dependencies
	"target/",			// cargo build output
];

// The ignore files read in each directory, lowest priority first.
pub const IGNORE_NAMES: &[&str] = &[
	".gitignore",
	".oreignore",
];

/// One name from a directory listing, as the caller's source gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub name:   Vec<u8>,    // one component, no slash
    pub is_dir: bool,
}

impl Listed {
    pub fn new<N: Into<Vec<u8>>>(name: N, is_dir: bool) -> Self {
        Self { name: name.into(), is_dir }
    }
}

/// One entry the walk kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkEntry {
    pub path:   Vec<u8>,    // relative to the walk's root, components joined by `/`
    pub is_dir: bool,
}

/// How far a walk got before its entry budget ran out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkBudget {
    pub budget:     usize,      // the entries allowed
    pub kept:       usize,      // entries yielded
    pub ignored:    usize,      // entries listed but ignored
    pub unlisted:   usize,      // kept directories never listed
    pub at:         Vec<u8>,    // the directory being listed when it ran out
}

/// What the caller is to do next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    List(Vec<u8>),          // list this directory, then call `listed`
    Read(Vec<u8>),          // read this ignore file, then call `read`
    Entry(WalkEntry),       // a kept file or directory
    Stopped(WalkBudget),    // the budget ran out; the walk is over
    Done,                   // everything not ignored has been yielded
}

/// How [`Walk::run`] ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalkEnd {
    Complete,
    Budget(WalkBudget),
    Halted,                 // the visitor asked to stop
}

/// A synchronous listing source for [`Walk::run`].
pub trait WalkSource {
    /// Lists one directory, given relative to the walk's root (empty for the root itself).
    fn list(&mut self, dir: &[u8]) -> Outcome<Vec<Listed>>;
    /// Reads one ignore file; `None` when it has gone or cannot be read.
    fn read(&mut self, path: &[u8]) -> Outcome<Option<Vec<u8>>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Want {
    Nothing,
    Listing,
    File,
}

// One ignore file, scoped to the directory it sits in, chained to the one that ranks below it.
#[derive(Debug)]
struct Layer {
    base:   Vec<u8>,
    file:   IgnoreFile,
    below:  Option<usize>,
}

/// An ignore-aware depth-first walk. See the module documentation.
#[derive(Debug)]
pub struct Walk {
    defaults:   IgnoreFile,
    names:      Vec<Vec<u8>>,
    layers:     Vec<Layer>,                     // arena of every ignore file read
    pending:    Vec<(Vec<u8>, Option<usize>)>,  // directories to list, with their top layer
    dir:        Vec<u8>,                        // the directory whose listing is being worked
    top:        Option<usize>,                  // its top layer
    entries:    VecDeque<Listed>,               // its listing, not yet judged
    reads:      VecDeque<Vec<u8>>,              // its ignore files, not yet read
    children:   Vec<Vec<u8>>,                   // its kept subdirectories
    want:       Want,
    budget:     usize,
    kept:       usize,
    ignored:    usize,
    over:       bool,
}

impl Walk {

    /// A walk from the root, reading [`IGNORE_NAMES`] above [`WALK_DEFAULTS`], allowed `budget`
    /// listed entries.
    pub fn new(budget: usize) -> Self {
        let defaults = IgnoreFile::parse(WALK_DEFAULTS.join("\n").as_bytes());
        Self {
            defaults,
            names:      IGNORE_NAMES.iter().map(|n| n.as_bytes().to_vec()).collect(),
            layers:     Vec::new(),
            pending:    vec![(Vec::new(), None)],
            dir:        Vec::new(),
            top:        None,
            entries:    VecDeque::new(),
            reads:      VecDeque::new(),
            children:   Vec::new(),
            want:       Want::Nothing,
            budget,
            kept:       0,
            ignored:    0,
            over:       false,
        }
    }

    /// Replaces the built-in defaults, e.g. with an empty [`IgnoreFile`] to walk everything.
    pub fn with_defaults(mut self, defaults: IgnoreFile) -> Self {
        self.defaults = defaults;
        self
    }

    /// Replaces the names of the ignore files read in each directory, lowest priority first.
    pub fn with_ignore_names<N: AsRef<[u8]>>(mut self, names: &[N]) -> Self {
        self.names = names.iter().map(|n| n.as_ref().to_vec()).collect();
        self
    }

    pub fn next(&mut self) -> Outcome<Step> {
        match self.want {
            Want::Nothing	=> (),
            Want::Listing	=> return Err(err!(
                "Walk::next called while the listing of '{}' is still owed.",
                String::from_utf8_lossy(&self.dir); Order)),
            Want::File		=> return Err(err!(
                "Walk::next called while the ignore file '{}' is still owed.",
                String::from_utf8_lossy(self.reads.front().map(|p| p.as_slice()).unwrap_or(b"")); Order)),
        }
        if self.over {
            return Ok(Step::Done);
        }
        if let Some(path) = self.reads.front() {
            self.want = Want::File;
            return Ok(Step::Read(path.clone()));
        }
        while let Some(item) = self.entries.pop_front() {
            if self.kept + self.ignored >= self.budget {
                self.over = true;
                let unlisted = self.pending.len()
                    + self.children.len()
                    + if item.is_dir { 1 } else { 0 }
                    + self.entries.iter().filter(|e| e.is_dir).count();
                return Ok(Step::Stopped(WalkBudget {
                    budget:     self.budget,
                    kept:       self.kept,
                    ignored:    self.ignored,
                    unlisted,
                    at:         self.dir.clone(),
                }));
            }
            let path = Self::join(&self.dir, &item.name);
            if self.is_ignored(&path, item.is_dir) {
                self.ignored += 1;
                continue;
            }
            self.kept += 1;
            if item.is_dir {
                self.children.push(path.clone());
            }
            return Ok(Step::Entry(WalkEntry { path, is_dir: item.is_dir }));
        }
        // Listing done: its subdirectories go on the stack so the first is listed next.
        let top = self.top;
        for child in self.children.drain(..).rev() {
            self.pending.push((child, top));
        }
        match self.pending.pop() {
            Some((dir, top)) => {
                self.dir = dir;
                self.top = top;
                self.want = Want::Listing;
                Ok(Step::List(self.dir.clone()))
            }
            None => {
                self.over = true;
                Ok(Step::Done)
            }
        }
    }

    /// Answers a [`Step::List`] with the directory's entries, in the order to walk them.
    pub fn listed(&mut self, entries: Vec<Listed>) -> Outcome<()> {
        if self.want != Want::Listing {
            return Err(err!("Walk::listed called when no listing was asked for."; Order));
        }
        self.want = Want::Nothing;
        for name in &self.names {
            if entries.iter().any(|e| !e.is_dir && &e.name == name) {
                self.reads.push_back(Self::join(&self.dir, name));
            }
        }
        self.entries = entries.into();
        Ok(())
    }

    /// Answers a [`Step::Read`] with the ignore file's bytes, or `None` if it could not be read.
    pub fn read(&mut self, bytes: Option<Vec<u8>>) -> Outcome<()> {
        if self.want != Want::File {
            return Err(err!("Walk::read called when no ignore file was asked for."; Order));
        }
        self.want = Want::Nothing;
        self.reads.pop_front();
        if let Some(bytes) = bytes {
            let file = IgnoreFile::parse(&bytes);
            if !file.is_empty() {
                self.layers.push(Layer { base: self.dir.clone(), file, below: self.top });
                self.top = Some(self.layers.len() - 1);
            }
        }
        Ok(())
    }

    /// Drives the walk to its end over a synchronous source, handing each kept entry to `visit`,
    /// which returns `false` to stop.
    pub fn run<S, F>(mut self, src: &mut S, mut visit: F) -> Outcome<WalkEnd>
    where
        S: WalkSource,
        F: FnMut(WalkEntry) -> Outcome<bool>,
    {
        loop {
            match res!(self.next()) {
                Step::List(dir)		=> res!(self.listed(res!(src.list(&dir)))),
                Step::Read(path)	=> res!(self.read(res!(src.read(&path)))),
                Step::Entry(entry)	=> if !res!(visit(entry)) { return Ok(WalkEnd::Halted); },
                Step::Stopped(rep)	=> return Ok(WalkEnd::Budget(rep)),
                Step::Done			=> return Ok(WalkEnd::Complete),
            }
        }
    }

    // The deepest ignore file that speaks decides; the defaults speak last.
    fn is_ignored(&self, path: &[u8], is_dir: bool) -> bool {
        let mut at = self.top;
        while let Some(i) = at {
            let layer = &self.layers[i];
            let rel = if layer.base.is_empty() { path } else { &path[layer.base.len() + 1..] };
            if let Some(ignored) = layer.file.decides(rel, is_dir) {
                return ignored;
            }
            at = layer.below;
        }
        self.defaults.ignores(path, is_dir)
    }

    fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
        if dir.is_empty() {
            return name.to_vec();
        }
        let mut path = Vec::with_capacity(dir.len() + 1 + name.len());
        path.extend_from_slice(dir);
        path.push(b'/');
        path.extend_from_slice(name);
        path
    }
}


/// A [`WalkSource`] over `std::fs`, rooted at a directory.
///
/// Symbolic links are listed as files and never followed. A directory that vanishes or cannot be
/// read mid-walk lists as empty rather than ending the walk; the root itself must be readable.
#[derive(Clone, Debug)]
pub struct Disk {
    root: PathBuf,
}

impl Disk {

    pub fn new<P: Into<PathBuf>>(root: P) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, rel: &[u8]) -> PathBuf {
        if rel.is_empty() {
            return self.root.clone();
        }
        self.root.join(Self::from_bytes(rel))
    }

    #[cfg(unix)]
    fn from_bytes(b: &[u8]) -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(b))
    }

    #[cfg(not(unix))]
    fn from_bytes(b: &[u8]) -> PathBuf {
        PathBuf::from(String::from_utf8_lossy(b).into_owned())
    }

    #[cfg(unix)]
    fn to_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
        use std::os::unix::ffi::OsStrExt;
        s.as_bytes().to_vec()
    }

    #[cfg(not(unix))]
    fn to_bytes(s: &std::ffi::OsStr) -> Vec<u8> {
        s.to_string_lossy().into_owned().into_bytes()
    }

    fn tolerable(e: &io::Error) -> bool {
        matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied)
    }
}

impl WalkSource for Disk {

    fn list(&mut self, dir: &[u8]) -> Outcome<Vec<Listed>> {
        let path = self.path(dir);
        let iter = match fs::read_dir(&path) {
            Ok(iter) => iter,
            Err(e) if !dir.is_empty() && Self::tolerable(&e) => return Ok(Vec::new()),
            Err(e) => return Err(err!(Error::from(e), "While listing {:?} for a walk.", path; IO, File)),
        };
        let mut out = Vec::new();
        for entry in iter {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) if Self::tolerable(&e) => continue,
                Err(e) => return Err(err!(Error::from(e), "While listing {:?} for a walk.", path; IO, File)),
            };
            let is_dir = match entry.file_type() {
                Ok(t) => t.is_dir(),
                Err(e) if Self::tolerable(&e) => continue,
                Err(e) => return Err(err!(Error::from(e), "While typing an entry of {:?} for a walk.", path; IO, File)),
            };
            out.push(Listed { name: Self::to_bytes(&entry.file_name()), is_dir });
        }
        // Name order, so a walk is the same on every run and every platform.
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn read(&mut self, path: &[u8]) -> Outcome<Option<Vec<u8>>> {
        let path = self.path(path);
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if Self::tolerable(&e) => Ok(None),
            Err(e) => Err(err!(Error::from(e), "While reading ignore file {:?}.", path; IO, File)),
        }
    }
}
