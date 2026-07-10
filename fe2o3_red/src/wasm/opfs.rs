//! OPFS filesystem edge — the browser's persistent storage for Red.
//!
//! The Origin Private File System (OPFS) is reached on the main thread
//! via `navigator.storage.getDirectory()`, which yields the origin's
//! private root directory.  All access is asynchronous, so this edge is
//! built on `wasm-bindgen-futures`.
//!
//! Paths are workspace-relative and jailed with the same lexical
//! discipline as [`crate::workspace::Workspace::resolve`]: absolute
//! paths and `..` traversal that escapes the root are rejected, so a
//! path can only ever address a descendant of the OPFS root.
//!
//! The synchronous `createSyncAccessHandle` path (single-writer Worker,
//! for the append-only `.red` log) is deferred; this async edge covers
//! whole-file read and write, which is what the first vertical needs.
// TODO(wasm-opfs-sync): add a Worker-hosted `createSyncAccessHandle`
// backend for the append-only session log, where synchronous positioned
// writes matter.

use crate::wasm::js_str;

use oxedyne_fe2o3_core::prelude::*;

use std::path::{Component, Path};

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    File,
    FileSystemDirectoryHandle,
    FileSystemFileHandle,
    FileSystemGetDirectoryOptions,
    FileSystemGetFileOptions,
    FileSystemRemoveOptions,
    FileSystemWritableFileStream,
};


/// Split a workspace-relative path into jailed components, tolerating an
/// empty result (which addresses the OPFS root itself).
///
/// Mirrors [`crate::workspace::Workspace::resolve`]: leading slashes are
/// stripped (treated as relative), `.` is skipped, and any absolute
/// component or `..` that would escape the root is rejected.  Returns the
/// ordered directory/file names; an empty vector means the root directory.
fn split_components(rel: &str) -> Outcome<Vec<String>> {
    let rel = rel.trim_start_matches('/');
    let mut out: Vec<String> = Vec::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => out.push(c.to_string_lossy().to_string()),
            Component::CurDir    => {},
            Component::ParentDir => {
                // Never pop above the root.
                if out.pop().is_none() {
                    return Err(err!(
                        "OPFS: path '{}' escapes the OPFS root.", rel;
                        Invalid, Input, Path));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(err!(
                    "OPFS: absolute path '{}' is not allowed.", rel;
                    Invalid, Input, Path));
            }
        }
    }
    Ok(out)
}

/// Split a workspace-relative path into jailed components, requiring at
/// least one component (a leaf file or directory name).
///
/// A wrapper over [`split_components`] for the file-addressing tools,
/// which always name a leaf; the empty (root) case is rejected here.
fn jail_components(rel: &str) -> Outcome<Vec<String>> {
    let out = res!(split_components(rel));
    if out.is_empty() {
        return Err(err!(
            "OPFS: path '{}' has no file component.", rel;
            Invalid, Input, Path));
    }
    Ok(out)
}

/// Acquire the OPFS root directory handle for this origin.
///
/// Runs on the main thread via `window.navigator.storage`; a secure
/// context (https or localhost) is required, which the browser enforces.
async fn root() -> Outcome<FileSystemDirectoryHandle> {
    let win = res!(web_sys::window()
        .ok_or_else(|| err!("OPFS: no window (main-thread OPFS requires a document)."; System, Missing)));
    let storage = win.navigator().storage();
    let dir_val = res!(JsFuture::from(storage.get_directory()).await
        .map_err(|e| err!("OPFS: getDirectory failed: {}.", js_str(&e); IO, File)));
    let dir: FileSystemDirectoryHandle = res!(dir_val.dyn_into()
        .map_err(|_| err!("OPFS: getDirectory did not return a directory handle."; IO, File)));
    Ok(dir)
}

/// Descend into (creating as needed) the directory components of a
/// jailed path, returning the handle to the directory that will hold the
/// leaf file plus the leaf name.
async fn descend(components: Vec<String>) -> Outcome<(FileSystemDirectoryHandle, String)> {
    let mut dir = res!(root().await);
    let last = components.len() - 1;
    let mut leaf = String::new();
    for (i, name) in components.into_iter().enumerate() {
        if i == last {
            leaf = name;
            break;
        }
        let opts = FileSystemGetDirectoryOptions::new();
        opts.set_create(true);
        let next_val = res!(JsFuture::from(
                dir.get_directory_handle_with_options(&name, &opts)).await
            .map_err(|e| err!("OPFS: open/create dir '{}' failed: {}.", name, js_str(&e); IO, File)));
        dir = res!(next_val.dyn_into()
            .map_err(|_| err!("OPFS: dir handle for '{}' was not a directory.", name; IO, File)));
    }
    Ok((dir, leaf))
}

/// Write `content` to `path`, creating parent directories and the file
/// as needed, replacing any existing contents.
pub async fn write_file(path: &str, content: &[u8]) -> Outcome<()> {
    let components = res!(jail_components(path));
    let (dir, leaf) = res!(descend(components).await);

    let opts = FileSystemGetFileOptions::new();
    opts.set_create(true);
    let file_val = res!(JsFuture::from(
            dir.get_file_handle_with_options(&leaf, &opts)).await
        .map_err(|e| err!("OPFS: open/create file '{}' failed: {}.", leaf, js_str(&e); IO, File)));
    let file: FileSystemFileHandle = res!(file_val.dyn_into()
        .map_err(|_| err!("OPFS: file handle for '{}' was not a file.", leaf; IO, File)));

    let writable_val = res!(JsFuture::from(file.create_writable()).await
        .map_err(|e| err!("OPFS: create writable for '{}' failed: {}.", leaf, js_str(&e); IO, File, Write)));
    let writable: FileSystemWritableFileStream = res!(writable_val.dyn_into()
        .map_err(|_| err!("OPFS: writable for '{}' had the wrong type.", leaf; IO, File, Write)));

    let write_promise = res!(writable.write_with_u8_array(content)
        .map_err(|e| err!("OPFS: queue write for '{}' failed: {}.", leaf, js_str(&e); IO, File, Write)));
    res!(JsFuture::from(write_promise).await
        .map_err(|e| err!("OPFS: write '{}' failed: {}.", leaf, js_str(&e); IO, File, Write)));

    // `close` is inherited from `WritableStream` and flushes the file.
    res!(JsFuture::from(writable.close()).await
        .map_err(|e| err!("OPFS: close '{}' failed: {}.", leaf, js_str(&e); IO, File, Write)));
    Ok(())
}

/// Read the entire contents of `path` as bytes.  Errors if any path
/// component (directory or the file itself) does not exist.
pub async fn read_file(path: &str) -> Outcome<Vec<u8>> {
    let components = res!(jail_components(path));
    let mut dir = res!(root().await);
    let last = components.len() - 1;
    let mut leaf = String::new();
    for (i, name) in components.into_iter().enumerate() {
        if i == last {
            leaf = name;
            break;
        }
        let next_val = res!(JsFuture::from(dir.get_directory_handle(&name)).await
            .map_err(|e| err!("OPFS: open dir '{}' failed: {}.", name, js_str(&e); IO, File, Read)));
        dir = res!(next_val.dyn_into()
            .map_err(|_| err!("OPFS: dir handle for '{}' was not a directory.", name; IO, File, Read)));
    }

    let file_val = res!(JsFuture::from(dir.get_file_handle(&leaf)).await
        .map_err(|e| err!("OPFS: open file '{}' failed: {}.", leaf, js_str(&e); IO, File, Read)));
    let handle: FileSystemFileHandle = res!(file_val.dyn_into()
        .map_err(|_| err!("OPFS: file handle for '{}' was not a file.", leaf; IO, File, Read)));

    // `get_file` yields a `File` (a `Blob`); read its bytes via
    // `arrayBuffer`, which returns the whole contents.
    let blob_val = res!(JsFuture::from(handle.get_file()).await
        .map_err(|e| err!("OPFS: get file '{}' failed: {}.", leaf, js_str(&e); IO, File, Read)));
    let file: File = res!(blob_val.dyn_into()
        .map_err(|_| err!("OPFS: get_file for '{}' returned a non-file.", leaf; IO, File, Read)));
    let buf_val = res!(JsFuture::from(file.array_buffer()).await
        .map_err(|e| err!("OPFS: read bytes of '{}' failed: {}.", leaf, js_str(&e); IO, File, Read)));
    let bytes = js_sys::Uint8Array::new(&buf_val).to_vec();
    Ok(bytes)
}

/// Descend into an *existing* directory path (no creation), returning the
/// handle.  An empty path (`""`, `"."`, `"/"`) resolves to the OPFS root.
/// Errors if any component does not exist or is not a directory.
async fn descend_dir(path: &str) -> Outcome<FileSystemDirectoryHandle> {
    let components = res!(split_components(path));
    let mut dir = res!(root().await);
    for name in components {
        let next_val = res!(JsFuture::from(dir.get_directory_handle(&name)).await
            .map_err(|e| err!("OPFS: open dir '{}' failed: {}.", name, js_str(&e); IO, File, Read)));
        dir = res!(next_val.dyn_into()
            .map_err(|_| err!("OPFS: dir handle for '{}' was not a directory.", name; IO, File, Read)));
    }
    Ok(dir)
}

/// Descend into the *existing* parent directory of a jailed path (no
/// creation), returning the parent handle plus the leaf name.
async fn open_parent(components: Vec<String>) -> Outcome<(FileSystemDirectoryHandle, String)> {
    let mut dir = res!(root().await);
    let last = components.len() - 1;
    let mut leaf = String::new();
    for (i, name) in components.into_iter().enumerate() {
        if i == last {
            leaf = name;
            break;
        }
        let next_val = res!(JsFuture::from(dir.get_directory_handle(&name)).await
            .map_err(|e| err!("OPFS: open dir '{}' failed: {}.", name, js_str(&e); IO, File)));
        dir = res!(next_val.dyn_into()
            .map_err(|_| err!("OPFS: dir handle for '{}' was not a directory.", name; IO, File)));
    }
    Ok((dir, leaf))
}

/// Read the entries of `dir`, returning `(name, is_dir, size)` per entry.
///
/// OPFS directory iteration is exposed as an async iterator via
/// `FileSystemDirectoryHandle.entries()` (web-sys returns a
/// [`js_sys::AsyncIterator`]).  Each `next()` yields a `Promise` resolving
/// to an `{ done, value }` record whose `value` is a `[name, handle]`
/// pair; the record fields are read with [`js_sys::Reflect`].  A file
/// entry's size comes from its [`File`] (`getFile().size`); directory
/// entries report a size of zero.
async fn read_entries(dir: &FileSystemDirectoryHandle) -> Outcome<Vec<(String, bool, u64)>> {
    let iter = dir.entries();
    let mut out: Vec<(String, bool, u64)> = Vec::new();
    loop {
        let promise = res!(iter.next()
            .map_err(|e| err!("OPFS: directory iterator next() failed: {}.", js_str(&e); IO, File, Read)));
        let record = res!(JsFuture::from(promise).await
            .map_err(|e| err!("OPFS: awaiting directory entry failed: {}.", js_str(&e); IO, File, Read)));

        // `done` signals iterator exhaustion; treat a missing/unreadable
        // flag as done so a malformed record cannot spin forever.
        let done = js_sys::Reflect::get(&record, &JsValue::from_str("done"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if done {
            break;
        }

        let value = res!(js_sys::Reflect::get(&record, &JsValue::from_str("value"))
            .map_err(|e| err!("OPFS: read directory entry value failed: {}.", js_str(&e); IO, File, Read)));
        let pair = js_sys::Array::from(&value);
        let name = pair.get(0).as_string().unwrap_or_default();
        let handle = pair.get(1);

        // The handle's `kind` distinguishes files from directories.
        let is_dir = js_sys::Reflect::get(&handle, &JsValue::from_str("kind"))
            .ok()
            .and_then(|v| v.as_string())
            .map(|k| k == "directory")
            .unwrap_or(false);

        let size = if is_dir {
            0u64
        } else {
            match handle.dyn_into::<FileSystemFileHandle>() {
                Ok(fh) => {
                    let file_val = res!(JsFuture::from(fh.get_file()).await
                        .map_err(|e| err!("OPFS: get file '{}' failed: {}.", name, js_str(&e); IO, File, Read)));
                    match file_val.dyn_into::<File>() {
                        Ok(f)  => f.size() as u64,
                        Err(_) => 0u64,
                    }
                }
                Err(_) => 0u64,
            }
        };
        out.push((name, is_dir, size));
    }
    Ok(out)
}

/// List the entries of the directory at `path`, returning
/// `(name, is_dir, size)` per entry (unsorted — the caller orders them).
/// An empty path addresses the OPFS root.
pub async fn list_dir(path: &str) -> Outcome<Vec<(String, bool, u64)>> {
    let dir = res!(descend_dir(path).await);
    read_entries(&dir).await
}

/// Delete the entry at `path`.  With `recursive` set, a directory and all
/// its contents are removed; otherwise a non-empty directory is rejected
/// by the browser.  Errors if the entry or any parent does not exist.
pub async fn delete_entry(path: &str, recursive: bool) -> Outcome<()> {
    let components = res!(jail_components(path));
    let (dir, leaf) = res!(open_parent(components).await);
    let opts = FileSystemRemoveOptions::new();
    opts.set_recursive(recursive);
    res!(JsFuture::from(dir.remove_entry_with_options(&leaf, &opts)).await
        .map_err(|e| err!("OPFS: remove '{}' failed: {}.", leaf, js_str(&e); IO, File)));
    Ok(())
}

/// Whether an entry (file or directory) exists at `path`.
pub async fn exists(path: &str) -> Outcome<bool> {
    let components = res!(jail_components(path));
    let (dir, leaf) = match open_parent(components).await {
        Ok(v)  => v,
        Err(_) => return Ok(false),
    };
    if JsFuture::from(dir.get_file_handle(&leaf)).await.is_ok() {
        return Ok(true);
    }
    if JsFuture::from(dir.get_directory_handle(&leaf)).await.is_ok() {
        return Ok(true);
    }
    Ok(false)
}
