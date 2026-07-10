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
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    File,
    FileSystemDirectoryHandle,
    FileSystemFileHandle,
    FileSystemGetDirectoryOptions,
    FileSystemGetFileOptions,
    FileSystemWritableFileStream,
};


/// Split a workspace-relative path into jailed components.
///
/// Mirrors [`crate::workspace::Workspace::resolve`]: leading slashes are
/// stripped (treated as relative), `.` is skipped, and any absolute
/// component or `..` that would escape the root is rejected.  Returns the
/// ordered directory/file names, the last of which is the leaf.
fn jail_components(rel: &str) -> Outcome<Vec<String>> {
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
