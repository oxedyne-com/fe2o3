//! Agent tools — the "coding" half of Red.
//!
//! Tools are modelled as an enum (favouring concrete types over dynamic
//! dispatch, per the Oxedyne style) rather than a trait-object registry.
//! Each variant knows its name, description, JSON-schema parameters, and
//! how to execute against a [`ToolContext`] (workspace + executor).
//!
//! Arguments arrive as the raw JSON string from the LLM's `tool_call`;
//! each tool extracts the fields it needs with the same manual JSON
//! helpers used by the LLM client — no `serde`.

use oxedyne_fe2o3_core::prelude::*;

use crate::executor::Executor;
use crate::llm::{extract_json_string, json_escape};
use crate::workspace::Workspace;


/// Shared context every tool executes against.
#[derive(Clone, Debug)]
pub struct ToolContext {
    pub workspace: Workspace,
    pub executor:  Executor,
    /// Working subdirectory (relative to the workspace root) for shell
    /// commands.  Empty means the workspace root.
    pub cwd:       String,
}

/// Maximum bytes returned from a file read / command output before
/// truncation, to keep tool results within a sane context budget.
const MAX_OUTPUT: usize = 60_000;


/// A built-in agent tool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    FileRead,
    FileWrite,
    FileEdit,
    FileList,
    FileSearch,
    FileDelete,
    Shell,
}

impl Tool {

    /// The default tool set offered to the agent.
    pub fn defaults() -> Vec<Tool> {
        vec![
            Tool::FileRead,
            Tool::FileWrite,
            Tool::FileEdit,
            Tool::FileList,
            Tool::FileSearch,
            Tool::FileDelete,
            Tool::Shell,
        ]
    }

    /// The tool's stable name, as sent to and returned from the LLM.
    pub fn name(&self) -> &'static str {
        match self {
            Tool::FileRead   => "file_read",
            Tool::FileWrite  => "file_write",
            Tool::FileEdit   => "file_edit",
            Tool::FileList   => "file_list",
            Tool::FileSearch => "file_search",
            Tool::FileDelete => "file_delete",
            Tool::Shell      => "shell",
        }
    }

    /// Look a tool up by its wire name.
    pub fn from_name(name: &str) -> Option<Tool> {
        match name {
            "file_read"   => Some(Tool::FileRead),
            "file_write"  => Some(Tool::FileWrite),
            "file_edit"   => Some(Tool::FileEdit),
            "file_list"   => Some(Tool::FileList),
            "file_search" => Some(Tool::FileSearch),
            "file_delete" => Some(Tool::FileDelete),
            "shell"       => Some(Tool::Shell),
            _             => None,
        }
    }

    /// One-line description for the LLM.
    pub fn description(&self) -> &'static str {
        match self {
            Tool::FileRead   => "Read a UTF-8 text file from the workspace.",
            Tool::FileWrite  => "Create or overwrite a file in the workspace with the given content.",
            Tool::FileEdit   => "Replace an exact, unique substring in a workspace file.",
            Tool::FileList   => "List the entries of a workspace directory.",
            Tool::FileSearch => "Search workspace files for a substring; returns matching file:line: text.",
            Tool::FileDelete => "Delete a file from the workspace.",
            Tool::Shell      => "Run a shell command in the workspace and return its stdout/stderr and exit code.",
        }
    }

    /// The tool's JSON-Schema `parameters` object.
    fn parameters(&self) -> &'static str {
        match self {
            Tool::FileRead => r#"{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative file path"}},"required":["path"]}"#,
            Tool::FileWrite => r#"{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative file path"},"content":{"type":"string","description":"Full file content"}},"required":["path","content"]}"#,
            Tool::FileEdit => r#"{"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string","description":"Exact substring to replace (must be unique)"},"new_string":{"type":"string","description":"Replacement text"}},"required":["path","old_string","new_string"]}"#,
            Tool::FileList => r#"{"type":"object","properties":{"path":{"type":"string","description":"Workspace-relative directory (default '.')"}}}"#,
            Tool::FileSearch => r#"{"type":"object","properties":{"query":{"type":"string","description":"Substring to search for"},"path":{"type":"string","description":"Directory to search (default '.')"}},"required":["query"]}"#,
            Tool::FileDelete => r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#,
            Tool::Shell => r#"{"type":"object","properties":{"command":{"type":"string","description":"Shell command to run"}},"required":["command"]}"#,
        }
    }

    /// This tool as an OpenAI `tools` array element.
    pub fn definition_json(&self) -> String {
        fmt!(
            r#"{{"type":"function","function":{{"name":"{}","description":"{}","parameters":{}}}}}"#,
            self.name(), json_escape(self.description()), self.parameters(),
        )
    }

    /// Execute the tool with the given raw-JSON arguments (native
    /// transport — the file tools use `std::fs`, the shell tool the
    /// process [`Executor`]).
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn execute(&self, args_json: &str, ctx: &ToolContext) -> Outcome<String> {
        match self {
            Tool::FileRead   => Self::file_read(args_json, ctx),
            Tool::FileWrite  => Self::file_write(args_json, ctx),
            Tool::FileEdit   => Self::file_edit(args_json, ctx),
            Tool::FileList   => Self::file_list(args_json, ctx),
            Tool::FileSearch => Self::file_search(args_json, ctx),
            Tool::FileDelete => Self::file_delete(args_json, ctx),
            Tool::Shell      => Self::shell(args_json, ctx).await,
        }
    }

    /// Execute the tool in the browser (wasm32), backing the file tools
    /// with the async OPFS edge ([`crate::wasm::opfs`]).
    ///
    /// OPFS applies its own lexical path jail, so the raw workspace-
    /// relative path is passed straight through; the [`ToolContext`] is
    /// unused here.  The full file toolset — read, write, edit, list,
    /// search and delete — mirrors the native semantics and output format;
    /// only the `shell` tool escalates, as there is no in-browser process
    /// executor.
    #[cfg(target_arch = "wasm32")]
    pub async fn execute(&self, args_json: &str, _ctx: &ToolContext) -> Outcome<String> {
        match self {
            Tool::FileWrite => {
                let path = res!(Self::arg(args_json, "path"));
                let content = res!(Self::arg(args_json, "content"));
                res!(crate::wasm::opfs::write_file(&path, content.as_bytes()).await);
                Ok(fmt!("Wrote {} bytes to {}.", content.len(), path))
            }
            Tool::FileRead => {
                let path = res!(Self::arg(args_json, "path"));
                let bytes = res!(crate::wasm::opfs::read_file(&path).await);
                let mut s = String::from_utf8_lossy(&bytes).to_string();
                if s.len() > MAX_OUTPUT {
                    s.truncate(MAX_OUTPUT);
                    s.push_str("\n… [truncated]");
                }
                Ok(s)
            }
            Tool::FileEdit => {
                let path = res!(Self::arg(args_json, "path"));
                let old = res!(Self::arg(args_json, "old_string"));
                let new = res!(Self::arg(args_json, "new_string"));
                let bytes = res!(crate::wasm::opfs::read_file(&path).await);
                let data = String::from_utf8_lossy(&bytes).to_string();
                let count = data.matches(&old).count();
                if count == 0 {
                    return Err(err!(
                        "file_edit: old_string not found in '{}'.", path;
                        Invalid, Input, NotFound));
                }
                if count > 1 {
                    return Err(err!(
                        "file_edit: old_string appears {} times in '{}'; make it unique.", count, path;
                        Invalid, Input, Excessive));
                }
                let updated = data.replacen(&old, &new, 1);
                res!(crate::wasm::opfs::write_file(&path, updated.as_bytes()).await);
                Ok(fmt!("Edited {}.", path))
            }
            Tool::FileList => {
                let path = extract_json_string(args_json, "path").unwrap_or_else(|| ".".to_string());
                let mut entries = res!(crate::wasm::opfs::list_dir(&path).await);
                // Dirs first, then by name — matching the native ordering.
                entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                if entries.is_empty() {
                    return Ok(fmt!("{} is empty.", path));
                }
                let mut out = String::new();
                for (name, is_dir, size) in entries {
                    if is_dir {
                        out.push_str(&fmt!("{}/\n", name));
                    } else {
                        out.push_str(&fmt!("{}  ({} bytes)\n", name, size));
                    }
                }
                Ok(out)
            }
            Tool::FileSearch => {
                let query = res!(Self::arg(args_json, "query"));
                let start = extract_json_string(args_json, "path").unwrap_or_else(|| ".".to_string());
                let mut matches: Vec<String> = Vec::new();
                let cap = 200usize;
                let mut stack = vec![start];
                'walk: while let Some(dir) = stack.pop() {
                    let entries = match crate::wasm::opfs::list_dir(&dir).await {
                        Ok(e)  => e,
                        Err(_) => continue,
                    };
                    for (name, is_dir, size) in entries {
                        if name.starts_with('.') || name == "target" || name == "node_modules" {
                            continue; // skip hidden / build dirs
                        }
                        let child = Self::join_rel(&dir, &name);
                        if is_dir {
                            stack.push(child);
                        } else {
                            if size > 2_000_000 {
                                continue; // skip large files
                            }
                            let bytes = match crate::wasm::opfs::read_file(&child).await {
                                Ok(b)  => b,
                                Err(_) => continue,
                            };
                            let text = String::from_utf8_lossy(&bytes);
                            for (i, line) in text.lines().enumerate() {
                                if line.contains(&query) {
                                    matches.push(fmt!("{}:{}: {}", child, i + 1, line.trim()));
                                    if matches.len() >= cap {
                                        matches.push("… [more matches truncated]".to_string());
                                        break 'walk;
                                    }
                                }
                            }
                        }
                    }
                }
                if matches.is_empty() {
                    Ok(fmt!("No matches for '{}'.", query))
                } else {
                    Ok(matches.join("\n"))
                }
            }
            Tool::FileDelete => {
                let path = res!(Self::arg(args_json, "path"));
                res!(crate::wasm::opfs::delete_entry(&path, false).await);
                Ok(fmt!("Deleted {}.", path))
            }
            Tool::Shell => Err(err!(
                "Tool 'shell' is not available in the browser build (no in-browser process executor).";
                Unimplemented)),
        }
    }

    /// Join a workspace-relative directory and an entry name into a clean
    /// relative path, dropping a `.`/empty directory prefix so search
    /// results read like the native workspace-relative form.
    #[cfg(target_arch = "wasm32")]
    fn join_rel(dir: &str, name: &str) -> String {
        if dir.is_empty() || dir == "." {
            name.to_string()
        } else {
            fmt!("{}/{}", dir.trim_end_matches('/'), name)
        }
    }

    // ── File tools (sync std::fs; workspace files are small) ────────

    fn arg<'a>(args: &'a str, key: &str) -> Outcome<String> {
        match extract_json_string(args, key) {
            Some(v) => Ok(v),
            None => Err(err!("Tool: missing required argument '{}'.", key; Invalid, Input, Missing)),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_read(args: &str, ctx: &ToolContext) -> Outcome<String> {
        let path = res!(Self::arg(args, "path"));
        let abs = res!(ctx.workspace.resolve(&path));
        let data = res!(std::fs::read(&abs)
            .map_err(|e| err!(e, "file_read: cannot read '{}'.", path; IO, File, Read)));
        let mut s = String::from_utf8_lossy(&data).to_string();
        if s.len() > MAX_OUTPUT {
            s.truncate(MAX_OUTPUT);
            s.push_str("\n… [truncated]");
        }
        Ok(s)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_write(args: &str, ctx: &ToolContext) -> Outcome<String> {
        let path = res!(Self::arg(args, "path"));
        let content = res!(Self::arg(args, "content"));
        let abs = res!(ctx.workspace.resolve(&path));
        if let Some(parent) = abs.parent() {
            res!(std::fs::create_dir_all(parent)
                .map_err(|e| err!(e, "file_write: cannot create parent dirs for '{}'.", path; IO, File)));
        }
        res!(std::fs::write(&abs, content.as_bytes())
            .map_err(|e| err!(e, "file_write: cannot write '{}'.", path; IO, File, Write)));
        Ok(fmt!("Wrote {} bytes to {}.", content.len(), path))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_edit(args: &str, ctx: &ToolContext) -> Outcome<String> {
        let path = res!(Self::arg(args, "path"));
        let old = res!(Self::arg(args, "old_string"));
        let new = res!(Self::arg(args, "new_string"));
        let abs = res!(ctx.workspace.resolve(&path));
        let data = res!(std::fs::read_to_string(&abs)
            .map_err(|e| err!(e, "file_edit: cannot read '{}'.", path; IO, File, Read)));
        let count = data.matches(&old).count();
        if count == 0 {
            return Err(err!("file_edit: old_string not found in '{}'.", path; Invalid, Input, NotFound));
        }
        if count > 1 {
            return Err(err!(
                "file_edit: old_string appears {} times in '{}'; make it unique.", count, path;
                Invalid, Input, Excessive));
        }
        let updated = data.replacen(&old, &new, 1);
        res!(std::fs::write(&abs, updated.as_bytes())
            .map_err(|e| err!(e, "file_edit: cannot write '{}'.", path; IO, File, Write)));
        Ok(fmt!("Edited {}.", path))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_list(args: &str, ctx: &ToolContext) -> Outcome<String> {
        let path = extract_json_string(args, "path").unwrap_or_else(|| ".".to_string());
        let abs = res!(ctx.workspace.resolve(&path));
        let mut entries = res!(std::fs::read_dir(&abs)
            .map_err(|e| err!(e, "file_list: cannot list '{}'.", path; IO, File, Read)))
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let is_dir = e.path().is_dir();
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                (is_dir, name, size)
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1))); // dirs first, then name
        if entries.is_empty() {
            return Ok(fmt!("{} is empty.", path));
        }
        let mut out = String::new();
        for (is_dir, name, size) in entries {
            if is_dir {
                out.push_str(&fmt!("{}/\n", name));
            } else {
                out.push_str(&fmt!("{}  ({} bytes)\n", name, size));
            }
        }
        Ok(out)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_delete(args: &str, ctx: &ToolContext) -> Outcome<String> {
        let path = res!(Self::arg(args, "path"));
        let abs = res!(ctx.workspace.resolve(&path));
        res!(std::fs::remove_file(&abs)
            .map_err(|e| err!(e, "file_delete: cannot delete '{}'.", path; IO, File)));
        Ok(fmt!("Deleted {}.", path))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_search(args: &str, ctx: &ToolContext) -> Outcome<String> {
        let query = res!(Self::arg(args, "query"));
        let path = extract_json_string(args, "path").unwrap_or_else(|| ".".to_string());
        let root = res!(ctx.workspace.resolve(&path));
        let mut matches = Vec::new();
        let mut stack = vec![root.clone()];
        let cap = 200usize;
        while let Some(dir) = stack.pop() {
            let rd = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for ent in rd.filter_map(|e| e.ok()) {
                let p = ent.path();
                let name = ent.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue; // skip hidden / build dirs
                }
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let meta = ent.metadata().ok();
                    if meta.map(|m| m.len() > 2_000_000).unwrap_or(true) {
                        continue; // skip large / unreadable files
                    }
                    if let Ok(text) = std::fs::read_to_string(&p) {
                        for (i, line) in text.lines().enumerate() {
                            if line.contains(&query) {
                                let rel = ctx.workspace.display_rel(&p);
                                matches.push(fmt!("{}:{}: {}", rel, i + 1, line.trim()));
                                if matches.len() >= cap {
                                    matches.push("… [more matches truncated]".to_string());
                                    return Ok(matches.join("\n"));
                                }
                            }
                        }
                    }
                }
            }
        }
        if matches.is_empty() {
            Ok(fmt!("No matches for '{}'.", query))
        } else {
            Ok(matches.join("\n"))
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn shell(args: &str, ctx: &ToolContext) -> Outcome<String> {
        let command = res!(Self::arg(args, "command"));
        let cwd = res!(ctx.workspace.resolve(&ctx.cwd));
        let out = res!(ctx.executor.run(&command, &cwd).await);
        let mut s = String::new();
        if !out.stdout.is_empty() { s.push_str(&out.stdout); }
        if !out.stderr.is_empty() {
            if !s.is_empty() && !s.ends_with('\n') { s.push('\n'); }
            s.push_str("[stderr] ");
            s.push_str(&out.stderr);
        }
        s.push_str(&fmt!("\n[exit code: {}]", out.exit_code));
        if s.len() > MAX_OUTPUT {
            s.truncate(MAX_OUTPUT);
            s.push_str("\n… [truncated]");
        }
        Ok(s)
    }
}


/// The set of tools available to the agent, plus the context they run in.
#[derive(Clone, Debug)]
pub struct ToolRegistry {
    pub tools: Vec<Tool>,
    pub ctx:   ToolContext,
}

impl ToolRegistry {

    pub fn new(tools: Vec<Tool>, ctx: ToolContext) -> Self {
        Self { tools, ctx }
    }

    /// True if no tools are enabled (pure-chat mode).
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// The `tools` JSON array for the LLM request, or `None` if empty.
    pub fn definitions_json(&self) -> Option<String> {
        if self.tools.is_empty() {
            return None;
        }
        let defs: Vec<String> = self.tools.iter().map(|t| t.definition_json()).collect();
        Some(fmt!("[{}]", defs.join(",")))
    }

    /// Execute a tool call by name, returning its result text.  Unknown
    /// tools and errors are returned as text so the LLM can recover.
    pub async fn dispatch(&self, name: &str, args_json: &str) -> String {
        match Tool::from_name(name) {
            Some(t) => match t.execute(args_json, &self.ctx).await {
                Ok(s)  => s,
                Err(e) => fmt!("Error: {}", e),
            },
            None => fmt!("Error: unknown tool '{}'.", name),
        }
    }
}


// ┌───────────────────────────────────────────────────────────────┐
// │ Tests                                                          │
// └───────────────────────────────────────────────────────────────┘

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext {
        let mut dir = std::env::temp_dir();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir.push(fmt!("red_tools_test_{}", n));
        let ws = Workspace::new(dir).expect("ws");
        ToolContext { workspace: ws, executor: Executor::local_default(), cwd: String::new() }
    }

    #[test]
    fn test_write_read_edit() {
        let c = ctx();
        let w = Tool::FileWrite.execute_sync(r#"{"path":"a.txt","content":"hello world"}"#, &c);
        assert!(w.is_ok());
        let r = Tool::FileRead.execute_sync(r#"{"path":"a.txt"}"#, &c).expect("read");
        assert_eq!(r, "hello world");
        Tool::FileEdit.execute_sync(r#"{"path":"a.txt","old_string":"world","new_string":"Red"}"#, &c).expect("edit");
        let r2 = Tool::FileRead.execute_sync(r#"{"path":"a.txt"}"#, &c).expect("read2");
        assert_eq!(r2, "hello Red");
    }

    #[test]
    fn test_edit_ambiguous_rejected() {
        let c = ctx();
        Tool::FileWrite.execute_sync(r#"{"path":"b.txt","content":"x x"}"#, &c).expect("write");
        let e = Tool::FileEdit.execute_sync(r#"{"path":"b.txt","old_string":"x","new_string":"y"}"#, &c);
        assert!(e.is_err()); // appears twice
    }

    #[test]
    fn test_list_and_search() {
        let c = ctx();
        Tool::FileWrite.execute_sync(r#"{"path":"src/main.rs","content":"fn main() { needle }"}"#, &c).expect("w");
        let list = Tool::FileList.execute_sync(r#"{"path":"."}"#, &c).expect("list");
        assert!(list.contains("src/"));
        let found = Tool::FileSearch.execute_sync(r#"{"query":"needle"}"#, &c).expect("search");
        assert!(found.contains("main.rs"));
        assert!(found.contains("needle"));
    }

    #[test]
    fn test_delete() {
        let c = ctx();
        Tool::FileWrite.execute_sync(r#"{"path":"gone.txt","content":"x"}"#, &c).expect("w");
        Tool::FileDelete.execute_sync(r#"{"path":"gone.txt"}"#, &c).expect("del");
        assert!(Tool::FileRead.execute_sync(r#"{"path":"gone.txt"}"#, &c).is_err());
    }

    #[tokio::test]
    async fn test_shell_tool() {
        let c = ctx();
        let out = Tool::Shell.execute(r#"{"command":"echo hi"}"#, &c).await.expect("shell");
        assert!(out.contains("hi"));
        assert!(out.contains("exit code: 0"));
    }

    #[test]
    fn test_definitions_json() {
        let reg = ToolRegistry::new(Tool::defaults(), ctx());
        let defs = reg.definitions_json().expect("defs");
        assert!(defs.contains("file_read"));
        assert!(defs.contains("shell"));
        assert!(defs.starts_with('['));
    }
}

// Test-only synchronous shim for the file tools (which are sync anyway).
#[cfg(test)]
impl Tool {
    fn execute_sync(&self, args: &str, ctx: &ToolContext) -> Outcome<String> {
        match self {
            Tool::FileRead   => Self::file_read(args, ctx),
            Tool::FileWrite  => Self::file_write(args, ctx),
            Tool::FileEdit   => Self::file_edit(args, ctx),
            Tool::FileList   => Self::file_list(args, ctx),
            Tool::FileSearch => Self::file_search(args, ctx),
            Tool::FileDelete => Self::file_delete(args, ctx),
            Tool::Shell      => Err(err!("use execute() for shell"; Invalid)),
        }
    }
}
