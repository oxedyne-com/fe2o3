use crate::prelude::*;

use std::{
    ops::Deref,
    path::{
        Component,
        Path,
        PathBuf,
    },
};


/// Check that the string contains no path components and is just a filename.
pub fn is_filename(name: &str) -> bool {
    !name.contains(std::path::MAIN_SEPARATOR) &&
    cfg!(not(windows)) || !name.contains('/')
}

/// Canonicalise a path that need not exist yet.
///
/// The longest ancestor that exists is canonicalised by the file system, so a symlink in it is
/// followed to where it really points; the components below it, which cannot be symlinks because
/// they do not exist, are appended lexically, with `.` dropped and `..` taking one step up. A
/// relative path is taken from the current directory. This is what a containment check needs for
/// a file about to be read or written: `std::fs::canonicalize` refuses a path that is not there,
/// and lexical normalisation alone is fooled by a symlink.
pub fn canonicalise_allow_missing(path: &Path) -> Outcome<PathBuf> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(d)  => d.join(path),
            Err(e) => return Err(err!(e,
                "Cannot read the current directory to place {:?}.", path; IO, File)),
        }
    };
    let comps: Vec<Component> = abs.components().collect();
    // Find the longest existing prefix, walking back from the whole path.
    let mut n = comps.len();
    let mut base = loop {
        let prefix: PathBuf = comps[..n].iter().collect();
        match std::fs::canonicalize(&prefix) {
            Ok(c) => break c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && n > 1 => n -= 1,
            Err(e) => return Err(err!(e,
                "Cannot canonicalise {:?}, the existing part of {:?}.", prefix, path;
                IO, File)),
        }
    };
    for comp in &comps[n..] {
        match comp {
            Component::CurDir       => (),
            Component::ParentDir    => { base.pop(); },
            Component::Normal(c)    => base.push(c),
            Component::RootDir | Component::Prefix(_) => return Err(err!(
                "Unexpected root component {:?} inside {:?}.", comp, path;
                Bug, Unexpected)),
        }
    }
    Ok(base)
}

/// Normalise a relative path without reference to the state of the file system by:
/// - Ensure path begins with "." or "..", e.g. /a/b -> ./a/b, ./../a -> ../a
/// - Eliminate redundant path components, e.g. a/b/../c -> ./a/c
/// - Eliminate trailing separator, e.g. a/b/ -> ./a/b
/// The result will always begin with either "." or a sequence of "..".  Neither will appear
/// elsewhere in the path.
pub trait NormalPath {
    fn normalise(&self) -> NormPathBuf;
}

new_type_priv!(NormPathBuf, PathBuf, Clone, Debug);

/// A `NormPathBuf` wraps a `PathBuf` with some extra methods.  Making the inner object private
/// ensures that a `NormPathBuf` can generally only be created by normalising a `Path`.
impl NormPathBuf {
    /// Test whether the normalised `PathBuf` escapes upwards out of the current directory.
    pub fn escapes(&self) -> bool {
        match self.components().next() {
            Some(std::path::Component::ParentDir) => true,
            _ => false,
        }
    }

    /// Remove leading "." and ".." components, and trailing separator.  We know that `self` is
    /// normalised, so all of these components will be at the front of the path.
    pub fn remove_relative(self) -> Self {
        let mut components = self.0.components();
        let mut pbuf = PathBuf::new();
        loop {
            let component = components.next();
            //debug!(" component = {:?}", component);
            match component {
                Some(Component::CurDir) | Some(Component::ParentDir) => continue,
                _ => {
                    pbuf.extend(component);
                    pbuf.extend(components);
                    break
                },
            }
        }
        Self(pbuf)
    }

    /// Ensure the path begins with "/". after removing relative components.
    pub fn absolute(self) -> Self {
        let path = self.remove_relative();
        let mut components = path.0.components();
        match components.next() {
            Some(Component::RootDir) => path,
            component => {
                let mut pbuf = PathBuf::from("/");
                pbuf.extend(component);
                pbuf.extend(components);
                Self(pbuf)
            },
        }
    }

    pub fn join(mut self, rel_path: Self) -> Self {
        self.0.push(rel_path.0);
        Self(self.0)
    }

    pub fn as_pathbuf(self) -> PathBuf { self.0 }
}

impl<T: ?Sized> AsRef<T> for NormPathBuf where <NormPathBuf as Deref>::Target: AsRef<T> {
    fn as_ref(&self) -> &T {
        self.deref().as_ref()
    }
}

impl NormalPath for Path {

    fn normalise(&self) -> NormPathBuf {
        let mut normalised = PathBuf::new();
    
        let mut first = true;
        for component in self.components() {
            //debug!(">> {:?} first={}", component, first);
            match component {
                Component::RootDir => {
                    normalised.push(".");
                },
                Component::ParentDir => {
                    if normalised.as_os_str().is_empty() || normalised == Path::new("..")
                    {
                        normalised.push(component.as_os_str());
                    } else if normalised == Path::new(".") {
                        // Replace "." with ".."
                        normalised.pop();
                        normalised.push(component.as_os_str());
                    } else {
                        normalised.pop();
                    }
                },
                Component::CurDir => {
                    if first {
                        normalised.push(component.as_os_str());
                    }
                },
                _ => { 
                    if first {
                        normalised.push(".");
                    }
                    normalised.push(component.as_os_str());
                },
            }
            //debug!(" > {:?}", normalised);
            if first {
                first = false;
            }
        }
    
        NormPathBuf(normalised)
    }
}
