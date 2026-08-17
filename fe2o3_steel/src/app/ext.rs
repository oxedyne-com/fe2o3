//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

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
pub trait AppExtension: Send + Sync + 'static {

    /// Called once at shell startup, after Steel's own built-ins are in place.
    fn extend_syntax(&self, s: Syntax) -> Outcome<Syntax> {
        Ok(s)
    }

    /// Called when a parsed command name matches no built-in. `Ok(None)` means
    /// the extension does not own the command.
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

    /// Each `(name, handler)` pair is reached by the `handler` name in a
    /// `webhook_routes` entry.
    fn webhook_handlers(&self) -> Vec<(String, Box<dyn WebhookHandler>)> {
        Vec::new()
    }

    /// Each `(name, handler)` pair is reached by the `handler` name in an
    /// `api_routes` entry, which dispatches in process instead of proxying to
    /// `upstream`.
    fn api_handlers(&self) -> Vec<(String, Box<dyn ApiHandler>)> {
        Vec::new()
    }
}


// ┌───────────────────────────────────────────────────────────────────────────┐
// │ NO-OP EXTENSION                                                           │
// └───────────────────────────────────────────────────────────────────────────┘

pub struct NoExtension;

impl AppExtension for NoExtension {}
