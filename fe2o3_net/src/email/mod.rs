// `file` (the mbox reader) is tokio-driven throughout; `msg` holds the tokio-free
// `EmailMessage`/`EmailHeader` types (with only its one async `read` gated), which
// `smtp::cmd` needs whether or not the async half of the crate is compiled.
#[cfg(feature = "async")]
pub mod file;
pub mod header;
pub mod msg;
