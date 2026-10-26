// `file` (the mbox reader) and `msg` (the `EmailMessage`/`EmailHeader` types and
// the line-driven `EmailParser`) are tokio-free, so `smtp::cmd` and mbox reading work
// whether or not the async half is compiled; only `EmailMessage::read` is gated.
pub mod file;
pub mod header;
pub mod msg;
