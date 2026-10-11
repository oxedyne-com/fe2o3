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
//! Two more layers sit where git puts them. A directory holding a `.git` directory has its
//! `.git/info/exclude` read first and laid at that directory beneath its own ignore files, so it
//! ranks below every in-tree file of the repository. A `.git` FILE (a linked worktree or a
//! submodule) is read as git reads it: its `gitdir:` names the repository's directory, and that
//! directory's `commondir`, when there is one, names the shared one whose `info/exclude` is laid
//! in the same place. A pointer that leaves the root, absolute or by climbing past it, cannot be
//! followed through a source that sees only the root, so the walk reports it as
//! [`Step::Beyond`] and leaves the caller to judge a rule it cannot know. Global excludes, git's
//! `core.excludesFile`, are given with [`Walk::with_excludes`] and lie just above the defaults; on
//! a native target [`git_global_excludes`] finds and reads them. A browser cannot see the user's
//! git configuration, so a walk there has no global layer unless the caller supplies one.
//!
//! [`Walk::beneath`] starts the walk at a directory inside the root while keeping entry paths
//! relative to the root, reading the ignore files of every directory above the start without
//! listing any of them. [`Walk::rules`] hands back every file the walk read as [`Rules`], which
//! answers for any path the same way the walk did, and names the file that decided.
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

pub const REPO_EXCLUDE:		&str = ".git/info/exclude";	// a repository's own excludes, below its files
pub const EXCLUDES_NAME:	&str = "core.excludesFile";	// how Rules::why names the global excludes

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
    Beyond(Vec<u8>),        // this `.git` file's repository lies outside the root; its excludes are unread
    Stopped(WalkBudget),    // the budget ran out; the walk is over
    Done,                   // everything not ignored has been yielded
}

/// How [`Walk::run`] ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalkEnd {
    Complete,               // a `.git` file pointing outside the root is passed over, see Walk::run
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
#[derive(Clone, Debug)]
struct Layer {
    base:   Vec<u8>,        // the directory it speaks for
    path:   Vec<u8>,        // the file itself, relative to the root
    rank:   usize,          // its order among one directory's files, lowest first
    file:   IgnoreFile,
    below:  Option<usize>,
}

// What an owed read is for.
#[derive(Clone, Debug)]
enum Owe {
    Rules,                                          // an ignore file, laid as a layer
    Link,                                           // a `.git` file, perhaps a `gitdir:` pointer
    Common { link: Vec<u8>, gitdir: Vec<u8> },      // that repository's `commondir`, perhaps absent
}

// A file still to be read.
#[derive(Clone, Debug)]
struct Owed {
    path:   Vec<u8>,
    rank:   usize,
    owe:    Owe,
}

impl Owed {
    fn rules(path: Vec<u8>, rank: usize) -> Self {
        Self { path, rank, owe: Owe::Rules }
    }
}

/// An ignore-aware depth-first walk. See the module documentation.
#[derive(Debug)]
pub struct Walk {
    defaults:   IgnoreFile,
    excludes:   IgnoreFile,                     // global excludes, just above the defaults
    names:      Vec<Vec<u8>>,
    layers:     Vec<Layer>,                     // arena of every ignore file read
    pending:    Vec<(Vec<u8>, Option<usize>)>,  // directories to list, with their top layer
    climb:      VecDeque<Vec<u8>>,              // root down to the start, not yet opened
    start_ign:  bool,                           // the start or a directory above it is ignored
    dir:        Vec<u8>,                        // the directory whose listing is being worked
    top:        Option<usize>,                  // its top layer
    entries:    VecDeque<Listed>,               // its listing, not yet judged
    reads:      VecDeque<Owed>,                 // its ignore files, not yet read
    children:   Vec<Vec<u8>>,                   // its kept subdirectories
    beyond:     VecDeque<Vec<u8>>,              // `.git` files pointing outside the root, unreported
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
            excludes:   IgnoreFile::default(),
            names:      IGNORE_NAMES.iter().map(|n| n.as_bytes().to_vec()).collect(),
            layers:     Vec::new(),
            pending:    vec![(Vec::new(), None)],
            climb:      VecDeque::new(),
            start_ign:  false,
            dir:        Vec::new(),
            top:        None,
            entries:    VecDeque::new(),
            reads:      VecDeque::new(),
            children:   Vec::new(),
            beyond:     VecDeque::new(),
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

    /// Lays a global excludes file, as git's `core.excludesFile`, below every file in the tree
    /// and above the defaults. See [`git_global_excludes`].
    pub fn with_excludes<B: AsRef<[u8]>>(mut self, bytes: B) -> Self {
        self.excludes = IgnoreFile::parse(bytes.as_ref());
        self
    }

    /// Starts the walk at `start`, a directory relative to the root, keeping entry paths relative
    /// to the root.
    ///
    /// Before `start` is listed, the walk asks to read [`REPO_EXCLUDE`] and every ignore file in
    /// each directory above it, the root first, without listing them; a source answers `None`
    /// for a file that is not there. If `start` or a directory above it is ignored, the walk ends
    /// with [`Step::Done`] having yielded nothing, and [`Walk::start_ignored`] says so.
    pub fn beneath<P: AsRef<[u8]>>(mut self, start: P) -> Self {
        let mut start = start.as_ref();
        while start.first() == Some(&b'/') {
            start = &start[1..];
        }
        while start.last() == Some(&b'/') {
            start = &start[..start.len() - 1];
        }
        if start.is_empty() {
            return self;
        }
        self.pending.clear();
        self.climb.push_back(Vec::new());
        for (i, b) in start.iter().enumerate() {
            if *b == b'/' {
                self.climb.push_back(start[..i].to_vec());
            }
        }
        self.climb.push_back(start.to_vec());
        self
    }

    /// Did the walk end because its start, or a directory above it, is ignored?
    pub fn start_ignored(&self) -> bool {
        self.start_ign
    }

    /// The rules the walk has read so far, to ask about paths it did not list.
    pub fn rules(&self) -> Rules {
        Rules {
            defaults:   self.defaults.clone(),
            excludes:   self.excludes.clone(),
            files:      self.layers.iter().map(|l| RuleFile {
                base:   l.base.clone(),
                path:   l.path.clone(),
                rank:   l.rank,
                file:   l.file.clone(),
            }).collect(),
        }
    }

    pub fn next(&mut self) -> Outcome<Step> {
        match self.want {
            Want::Nothing	=> (),
            Want::Listing	=> return Err(err!(
                "Walk::next called while the listing of '{}' is still owed.",
                String::from_utf8_lossy(&self.dir); Order)),
            Want::File		=> return Err(err!(
                "Walk::next called while the ignore file '{}' is still owed.",
                String::from_utf8_lossy(self.reads.front().map(|o| o.path.as_slice()).unwrap_or(b"")); Order)),
        }
        loop {
            if let Some(link) = self.beyond.pop_front() {
                return Ok(Step::Beyond(link));
            }
            if self.over {
                return Ok(Step::Done);
            }
            if let Some(owed) = self.reads.front() {
                self.want = Want::File;
                return Ok(Step::Read(owed.path.clone()));
            }
            // Climbing to the start: open the next directory down, unless it is ignored.
            let dir = match self.climb.pop_front() {
                Some(dir)   => dir,
                None        => break,
            };
            if !dir.is_empty() && self.is_ignored(&dir, true) {
                self.start_ign = true;
                self.over = true;
                return Ok(Step::Done);
            }
            if self.climb.is_empty() {
                self.pending.push((dir, self.top)); // the start itself, listed as usual
            } else {
                // Blind reads: the directory is never listed, so a source answers None for a gap.
                // At most one of `.git` as a file and `.git/info/exclude` answers.
                self.dir = dir;
                self.reads.push_back(Owed { path: Self::join(&self.dir, b".git"), rank: 0, owe: Owe::Link });
                self.reads.push_back(Owed::rules(Self::join(&self.dir, REPO_EXCLUDE.as_bytes()), 0));
                for (i, name) in self.names.iter().enumerate() {
                    self.reads.push_back(Owed::rules(Self::join(&self.dir, name), i + 1));
                }
            }
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
        // A repository's own excludes rank below its ignore files, so they are laid first.
        match entries.iter().find(|e| e.name == b".git") {
            Some(e) if e.is_dir => self.reads.push_back(Owed::rules(Self::join(&self.dir, REPO_EXCLUDE.as_bytes()), 0)),
            Some(_)             => self.reads.push_back(Owed { path: Self::join(&self.dir, b".git"), rank: 0, owe: Owe::Link }),
            None                => (),
        }
        for (i, name) in self.names.iter().enumerate() {
            if entries.iter().any(|e| !e.is_dir && &e.name == name) {
                self.reads.push_back(Owed::rules(Self::join(&self.dir, name), i + 1));
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
        let owed = match self.reads.pop_front() {
            Some(owed)  => owed,
            None        => return Err(err!("Walk::read found no owed ignore file to record."; Bug)),
        };
        let bytes = match (owed.owe, bytes) {
            (Owe::Rules, Some(bytes))   => bytes,
            // A `.git` file that is no pointer names no repository; git refuses it too.
            (Owe::Link, Some(bytes))    => {
                if let Some(to) = Self::gitdir(&bytes) {
                    match Self::resolve(&self.dir, to) {
                        Some(gitdir) => self.reads.push_front(Owed {
                            path:   Self::join(&gitdir, b"commondir"),
                            rank:   0,
                            owe:    Owe::Common { link: owed.path, gitdir },
                        }),
                        None => self.beyond.push_back(owed.path),
                    }
                }
                return Ok(());
            },
            // Present or not, the shared directory's info/exclude is the one git reads.
            (Owe::Common { link, gitdir }, bytes) => {
                let common = match bytes {
                    Some(b) => Self::resolve(&gitdir, trim(&b)),
                    None    => Some(gitdir),
                };
                match common {
                    Some(common) => self.reads.push_front(Owed::rules(Self::join(&common, b"info/exclude"), 0)),
                    None         => self.beyond.push_back(link),
                }
                return Ok(());
            },
            (_, None)                   => return Ok(()),
        };
        {
            let file = IgnoreFile::parse(&bytes);
            if !file.is_empty() {
                self.layers.push(Layer {
                    base:   self.dir.clone(),
                    path:   owed.path,
                    rank:   owed.rank,
                    file,
                    below:  self.top,
                });
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
                // A source that sees only the root cannot read beyond it; that repository's
                // excludes go unheard, as they did before `.git` files were followed.
                Step::Beyond(_)		=> (),
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
        if let Some(ignored) = self.excludes.decides(path, is_dir) {
            return ignored;
        }
        self.defaults.ignores(path, is_dir)
    }

    // The target of a `gitdir: <path>` pointer, as git's read_gitfile takes it.
    fn gitdir(bytes: &[u8]) -> Option<&[u8]> {
        let rest = match trim(bytes).strip_prefix(b"gitdir:") {
            Some(rest)  => trim(rest),
            None        => return None,
        };
        if rest.is_empty() { None } else { Some(rest) }
    }

    // `to` read from beside `base`, both relative to the root, or None when it leaves the root.
    fn resolve(base: &[u8], to: &[u8]) -> Option<Vec<u8>> {
        let drive = to.len() > 1 && to[1] == b':';
        if to.first() == Some(&b'/') || to.first() == Some(&b'\\') || drive {
            return None;
        }
        let mut parts: Vec<&[u8]> = base.split(|b| *b == b'/').filter(|c| !c.is_empty()).collect();
        for c in to.split(|b| *b == b'/') {
            match c {
                b"" | b"."  => (),
                b".."       => if parts.pop().is_none() { return None; },
                _           => parts.push(c),
            }
        }
        Some(parts.join(&b'/'))
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


#[derive(Clone, Debug)]
struct RuleFile {
    base:   Vec<u8>,
    path:   Vec<u8>,
    rank:   usize,
    file:   IgnoreFile,
}

/// Every ignore file a walk read, with its defaults and global excludes, as one matcher.
///
/// It answers for any path beneath the walk's root as the walk would have, so a caller can judge
/// a path it learns of later -- one another device sent, say -- without walking again. A
/// directory the walk never listed contributes nothing, so its own ignore files go unheard.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    defaults:   IgnoreFile,
    excludes:   IgnoreFile,
    files:      Vec<RuleFile>,
}

// Which layer decided.
enum Origin<'a> {
    File(&'a [u8]),
    Excludes,
    Defaults,
}

impl Rules {

    /// Is the path ignored, by its own rules or because a directory above it is? Nothing inside
    /// an ignored directory can be re-included, as in git.
    pub fn ignored(&self, path: &[u8], is_dir: bool) -> bool {
        self.top_ignored(path, is_dir).is_some()
    }

    /// The path of the ignore file that ignores `path` or the directory above it that is
    /// ignored, e.g. `sub/.gitignore`: empty for the built-in defaults, [`EXCLUDES_NAME`] for the
    /// global excludes, `None` when the path is not ignored.
    pub fn why(&self, path: &[u8], is_dir: bool) -> Option<Vec<u8>> {
        let top = match self.top_ignored(path, is_dir) {
            Some(top)   => top,
            None        => return None,
        };
        let top_dir = if top.len() == path.len() { is_dir } else { true };
        match self.decide(&top, top_dir) {
            Some((true, Origin::File(p)))   => Some(p.to_vec()),
            Some((true, Origin::Excludes))  => Some(EXCLUDES_NAME.as_bytes().to_vec()),
            Some((true, Origin::Defaults))  => Some(Vec::new()),
            _                               => None,
        }
    }

    /// The shortest ignored path among `path` and the directories above it.
    pub fn top_ignored(&self, path: &[u8], is_dir: bool) -> Option<Vec<u8>> {
        for (i, b) in path.iter().enumerate() {
            if *b == b'/' && i > 0 && Self::says_ignored(&self.decide(&path[..i], true)) {
                return Some(path[..i].to_vec());
            }
        }
        if !path.is_empty() && Self::says_ignored(&self.decide(path, is_dir)) {
            return Some(path.to_vec());
        }
        None
    }

    /// Adds the files of another walk's rules not already held, by file path, so walks of
    /// several starts under one root answer as one. The defaults and excludes of `self` stand.
    pub fn absorb(&mut self, other: Rules) {
        for f in other.files {
            if !self.files.iter().any(|g| g.path == f.path) {
                self.files.push(f);
            }
        }
    }

    fn says_ignored(d: &Option<(bool, Origin)>) -> bool {
        matches!(d, Some((true, _)))
    }

    // The deepest file that speaks decides, and in one directory the later file; then the global
    // excludes, then the defaults.  The same order as the walk's chain of layers.
    fn decide(&self, path: &[u8], is_dir: bool) -> Option<(bool, Origin<'_>)> {
        let mut best: Option<(&RuleFile, bool)> = None;
        for f in &self.files {
            let rel = if f.base.is_empty() {
                path
            } else if path.len() > f.base.len()
                && path.starts_with(&f.base)
                && path[f.base.len()] == b'/'
            {
                &path[f.base.len() + 1..]
            } else {
                continue;
            };
            if let Some((b, _)) = best {
                if (f.base.len(), f.rank) < (b.base.len(), b.rank) {
                    continue;
                }
            }
            if let Some(ignored) = f.file.decides(rel, is_dir) {
                best = Some((f, ignored));
            }
        }
        if let Some((f, ignored)) = best {
            return Some((ignored, Origin::File(&f.path)));
        }
        if let Some(ignored) = self.excludes.decides(path, is_dir) {
            return Some((ignored, Origin::Excludes));
        }
        self.defaults.decides(path, is_dir).map(|ignored| (ignored, Origin::Defaults))
    }
}


/// Finds and reads git's global excludes for this user, as git does.
///
/// `core.excludesFile` is taken from `$GIT_CONFIG_GLOBAL` when set, else from
/// `$XDG_CONFIG_HOME/git/config` and then `~/.gitconfig`, the later winning; unset, the file is
/// `$XDG_CONFIG_HOME/git/ignore` (`~/.config/git/ignore`). A leading `~/` is expanded and
/// `include` directives are not followed. `None` when there is no such file.
#[cfg(not(target_arch = "wasm32"))]
pub fn git_global_excludes() -> Outcome<Option<Vec<u8>>> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let home = var("HOME");
    let xdg = match var("XDG_CONFIG_HOME") {
        Some(x) => Some(x),
        None    => home.as_ref().map(|h| h.join(".config")),
    };
    let configs: Vec<PathBuf> = match var("GIT_CONFIG_GLOBAL") {
        Some(g) => vec![g],
        None    => {
            let mut v = Vec::new();
            if let Some(x) = &xdg {
                v.push(x.join("git").join("config"));
            }
            if let Some(h) = &home {
                v.push(h.join(".gitconfig"));
            }
            v
        }
    };
    let mut named: Option<Vec<u8>> = None;
    for config in &configs {
        if let Some(bytes) = res!(read_tolerant(config)) {
            if let Some(v) = git_config_excludes(&bytes) {
                named = Some(v);
            }
        }
    }
    let file = match named {
        Some(v) if v.starts_with(b"~/") => match &home {
            Some(h) => h.join(Disk::from_bytes(&v[2..])),
            None    => return Err(err!(
                "core.excludesFile '{}' starts with ~/ but HOME is not set.",
                String::from_utf8_lossy(&v); Missing, Configuration)),
        },
        Some(v) => Disk::from_bytes(&v),
        None    => match &xdg {
            Some(x) => x.join("git").join("ignore"),
            None    => return Ok(None),
        },
    };
    read_tolerant(&file)
}

/// The value of `core.excludesFile` in one git config file, if set; the last setting wins.
///
/// Section and key names match without regard to case, a quoted value is unquoted, a backslash
/// escapes the next byte, and `#` or `;` outside quotes begins a comment.
pub fn git_config_excludes(config: &[u8]) -> Option<Vec<u8>> {
    let mut in_core = false;
    let mut found = None;
    for raw in config.split(|b| *b == b'\n') {
        let mut line = trim(raw);
        if line.first() == Some(&b'[') {
            let end = match line.iter().position(|b| *b == b']') {
                Some(end)   => end,
                None        => continue,
            };
            in_core = trim(&line[1..end]).eq_ignore_ascii_case(b"core");
            line = trim(&line[end + 1..]); // a key may follow the header on its line
        }
        if !in_core || line.is_empty() || line[0] == b'#' || line[0] == b';' {
            continue;
        }
        let eq = match line.iter().position(|b| *b == b'=') {
            Some(eq)    => eq,
            None        => continue,
        };
        if !trim(&line[..eq]).eq_ignore_ascii_case(b"excludesfile") {
            continue;
        }
        let mut val = Vec::new();
        let mut quoted = false;
        let mut bytes = trim(&line[eq + 1..]).iter();
        while let Some(b) = bytes.next() {
            match *b {
                b'"'                    => quoted = !quoted,
                b'\\'                   => if let Some(n) = bytes.next() { val.push(*n); },
                b'#' | b';' if !quoted  => break,
                other                   => val.push(other),
            }
        }
        let val = trim(&val);
        found = if val.is_empty() { None } else { Some(val.to_vec()) };
    }
    found
}

fn trim(b: &[u8]) -> &[u8] {
    let start = match b.iter().position(|c| !c.is_ascii_whitespace()) {
        Some(i) => i,
        None    => return &b[b.len()..],
    };
    let end = match b.iter().rposition(|c| !c.is_ascii_whitespace()) {
        Some(i) => i + 1,
        None    => b.len(),
    };
    &b[start..end]
}

#[cfg(not(target_arch = "wasm32"))]
fn read_tolerant(path: &std::path::Path) -> Outcome<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes)                       => Ok(Some(bytes)),
        Err(e) if Disk::tolerable(&e)   => Ok(None),
        Err(e)                          => Err(err!(Error::from(e),
            "While reading git's global excludes from {:?}.", path; IO, File)),
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

    // A blind read above a walk's start may meet a `.git` file where a directory was expected,
    // or a directory named like an ignore file.
    fn tolerable(e: &io::Error) -> bool {
        matches!(e.kind(),
            io::ErrorKind::NotFound
            | io::ErrorKind::PermissionDenied
            | io::ErrorKind::NotADirectory
            | io::ErrorKind::IsADirectory)
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
