//! The main idea behind laying out tests this way is to ensure that the logger is shut down
//! cleanly, after capturing all lingering messages.
//!

mod client;
mod server;

use oxedize_fe2o3_core::{
    prelude::*,
    log::bot::FileConfig,
};

use std::time::Duration;


fn setup_log() -> Outcome<()> {
    let mut log_cfg = log_get_config!();
    log_cfg.level = res!(LogLevel::from_str("trace"));
    let file_cfg = FileConfig::new(
        res!(std::env::current_dir()),
        "steel_test".to_string(),
        "log".to_string(),
        0,
        1_048_576,
    );
    let log_path = file_cfg.path();
    log_cfg.file = Some(file_cfg);
    log_set_config!(log_cfg);
    info!("Logging at {:?}", log_path);
    Ok(())
}

#[tokio::main]
async fn run_integrated() -> Outcome<()> {

    let filter = "all";
    let result = client::test_client(filter).await;
    res!(result);
    let result = server::test_server(filter).await;
    res!(result);

    Ok(())
}

/// Run as
/// ```ignore
///     cargo test server -- --nocapture
/// ```
#[test]
fn server() -> Outcome<()> {
    
    res!(setup_log());

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return Err(err!(e, "Failed to create Tokio runtime."; IO, Init)),
    };

    let outcome = runtime.block_on(async {
        server::test_server("all").await
    });

    if let Err(e) = &outcome {
        error!(e.clone());
    }

    log_finish_wait!();

    outcome
}

/// Run as
/// ```ignore
///     cargo test client -- --nocapture
/// ```
#[test]
fn client() -> Outcome<()> {
    
    res!(setup_log());

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return Err(err!(e, "Failed to create Tokio runtime."; IO, Init)),
    };

    let outcome = runtime.block_on(async {
        client::test_client("websocket").await
    });

    if let Err(e) = &outcome {
        error!(e.clone());
    }

    log_finish_wait!();

    outcome
}

/// Run as
/// ```ignore
///     cargo test integrated -- --nocapture
/// ```
#[tokio::test]
async fn integrated() -> Outcome<()> {
    res!(setup_log());

    // Start the server in a separate task.
    let server_handle = tokio::spawn(server::test_server("all"));

    // Give the server a short time to start up.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Run the client test.
    let client_result = client::test_client("websocket").await;

    // Stop the server.
    server_handle.abort();

    // Check the results.
    res!(client_result);

    Ok(())
}
