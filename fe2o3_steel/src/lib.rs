//! A TCP server implementation providing HTTPS, WebSocket and SMTPS support.
//! 
//! Steel provides both a server implementation and a complete application framework. The server layer
//! handles TLS certificates, static file serving, WebSocket upgrades and SMTPS connections. The
//! application layer provides configuration management, development tooling and a shell interface.
//!
//! # Building
//! 
//! Build the Steel server application with:
//! ```bash
//! cargo build --release
//! ```
//! 
//! This creates the `steel` binary in the target/release directory.
//!
//! # Architecture
//!
//! The crate is structured into two main modules:
//! - `srv`: The core server implementation providing HTTPS, WebSocket and SMTPS support
//! - `app`: The application framework including configuration, development tools and shell interface
//!
//! # Features
//!
//! - Development mode with hot reloading and automated self-signed certificates
//! - Production mode with Let's Encrypt certificate automation
//! - JavaScript/TypeScript bundling and SASS compilation in development
//! - Configurable static file serving and routing
//! - WebSocket support with protocol abstractions
//! - Clean separation between server and application concerns
//! - Post-quantum cryptography options
//!
//! # Shell Interface
//!
//! The Steel server operates as an interactive shell, allowing management of:
//! - TLS certificates
//! - Server configuration
//! - Development mode
//! - File serving
//! - Encrypted secrets
//!
//! # Extending Functionality 
//!
//! New shell commands can be added by:
//!
//! 1. Adding a command to the syntax in app/syntax.rs:
//! ```rust
//! let cmd = Cmd::from(CmdConfig {
//!     name:   fmt!("mycommand"),
//!     help:   Some(fmt!("Description of my command")),
//!     cat:    fmt!("Category"),
//!     ..Default::default()
//! });
//! s = res!(s.add_cmd(cmd));
//! ```
//!
//! 2. Adding a match arm in app/repl.rs execute() method:
//! ```rust
//! match cmd_key.as_str() {
//!     "mycommand" => evals.push(res!(self.my_command(&shell_cfg, Some(cmd)))),
//!     // ... other commands ...
//! }
//! ```
//!
//! 3. Implementing the command handler in the AppShellContext:
//! ```rust
//! impl AppShellContext {
//!     pub fn my_command(
//!         &mut self,
//!         shell_cfg: &ShellConfig,
//!         cmd: Option<&MsgCmd>,
//!     ) 
//!         -> Outcome<Evaluation>
//!     {
//!         // Command implementation
//!         Ok(Evaluation::Output(fmt!("Command executed")))
//!     }
//! }
//! ```
//!
//! # Configuration
//!
//! On first run, Steel creates a default configuration file config.jdat. This contains settings for:
//! - Server ports and addresses
//! - TLS certificate paths
//! - Static file serving paths
//! - Development mode options
//! - Logging configuration
//! 
//! The configuration can be modified directly or through the shell interface.
//!
#![forbid(unsafe_code)]
pub mod srv;
pub mod app;

/// Public API for apps that build on Steel.
///
/// Apps that need custom webhook handlers, in-process API handlers
/// or shell commands depend on `fe2o3_steel` as a library, implement
/// the `AppExtension` trait, and call `run_with_extension`.
pub mod prelude {
    pub use crate::app::ext::{
        AppExtension,
        NoExtension,
    };
    pub use crate::app::server::build_outbound_tls_client;
    pub use crate::app::tui::{
        run,
        run_with_extension,
    };
    pub use crate::srv::api::{
        ApiHandler,
        ApiHandlerRegistry,
    };
    pub use crate::srv::cfg::{
        ApiRoute,
        WebhookRoute,
    };
    pub use crate::srv::webhook::{
        WebhookHandler,
        WebhookRegistry,
        // Utilities for handler implementations.
        url_encode,
        extract_value,
        extract_json_string,
        // Stripe webhook signature verification.
        verify_stripe_signature,
        STRIPE_SIG_TOLERANCE_SECS,
    };

    // Request-side HTTP types that handlers need to read incoming
    // request headers and construct response messages without
    // depending on `fe2o3_net` directly.
    pub use oxedyne_fe2o3_net::http::{
        fields::{
            HeaderFields,
            HeaderFieldValue,
            HeaderName,
        },
        header::{
            HttpHeadline,
            HttpMethod,
        },
        loc::HttpLocator,
        msg::HttpMessage,
        status::HttpStatus,
    };

    // Re-exports from the upstream syntax / TUI crates so that app
    // extensions can implement `AppExtension` without depending on
    // them directly.
    pub use oxedyne_fe2o3_syntax::{
        Syntax,
        msg::MsgCmd,
        cmd::{
            Cmd,
            CmdConfig,
        },
        arg::{
            Arg,
            ArgConfig,
        },
        opt::OptionRefVec,
    };
    pub use oxedyne_fe2o3_tui::lib_tui::repl::{
        Evaluation,
        ShellConfig,
    };
}
