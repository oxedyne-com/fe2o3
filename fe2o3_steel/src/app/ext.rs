/// App extension surface.
///
/// Steel is a server framework used by concrete applications. Each
/// app may need:
///
/// * its own shell subcommands (with proper `help`/`cat`/args in
///   the Syntax tree so `./steel help` lists them alongside the
///   built-ins).
/// * its own webhook handlers (incoming notifications from third
///   parties -- already covered by `srv::webhook::WebhookRegistry`).
/// * its own API handlers (in-process request handlers mounted at
///   `api_routes` paths that are marked with a `handler` name
///   instead of being proxied to a remote upstream).
///
/// This module defines a single trait, `AppExtension`, that app
/// binaries implement and hand to `run_with_extension`. Steel then
/// uses it to populate the shell Syntax tree, to build the webhook
/// and API registries at startup, and to dispatch shell commands it
/// does not recognise.
///
/// Steel binaries that do not need any extension can pass
/// `NoExtension`, which is the default handed to `run`.

use crate::srv::{
    api::ApiHandler,
    webhook::WebhookHandler,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_syntax::{
    Syntax,
    msg::MsgCmd,
};
use oxedyne_fe2o3_tui::lib_tui::repl::{
    Evaluation,
    ShellConfig,
};


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ APP EXTENSION TRAIT                                                       │
// └───────────────────────────────────────────────────────────────────────────┘

/// Extension surface an app binary hands to Steel at startup.
///
/// All four methods have sensible defaults -- an extension can
/// implement only the parts it needs (e.g. CLI-only, or
/// API-handlers-only).
pub trait AppExtension: Send + Sync + 'static {

    /// Contribute commands to the shell Syntax tree.
    ///
    /// Called once at shell startup, after Steel has built its own
    /// built-in commands. The extension adds its commands via
    /// `s.add_cmd(...)` and returns the augmented tree. Commands
    /// added here show up in `./steel help` automatically.
    fn extend_syntax(&self, s: Syntax) -> Outcome<Syntax> {
        Ok(s)
    }

    /// Dispatch a parsed shell command.
    ///
    /// Called by Steel's REPL loop when a parsed command name does
    /// not match any built-in. Return `Ok(None)` if the extension
    /// does not own this command (Steel will then log "command not
    /// implemented"). Return `Ok(Some(Evaluation))` if the extension
    /// handled the command.
    fn dispatch_cmd(
        &self,
        _cmd_name:  &str,
        _cmd:       &MsgCmd,
        _shell_cfg: &ShellConfig,
    )
        -> Outcome<Option<Evaluation>>
    {
        Ok(None)
    }

    /// Webhook handlers this extension wants registered at startup.
    ///
    /// Each `(name, handler)` pair populates the server-wide webhook
    /// registry so that `webhook_routes` entries with a matching
    /// `handler` name dispatch to the handler.
    fn webhook_handlers(&self) -> Vec<(String, Box<dyn WebhookHandler>)> {
        Vec::new()
    }

    /// API handlers this extension wants registered at startup.
    ///
    /// Each `(name, handler)` pair populates the server-wide API
    /// handler registry so that `api_routes` entries with a matching
    /// `handler` name dispatch to the handler instead of being
    /// proxied to `upstream`.
    fn api_handlers(&self) -> Vec<(String, Box<dyn ApiHandler>)> {
        Vec::new()
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ NO-OP EXTENSION                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

/// Placeholder extension that contributes nothing.
///
/// Used by the stock `steel` binary (no app crate) and as the base
/// type for `run()`. Apps that build on Steel provide their own
/// type implementing `AppExtension` and pass it to
/// `run_with_extension`.
pub struct NoExtension;

impl AppExtension for NoExtension {}
