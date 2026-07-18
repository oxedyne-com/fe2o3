//use oxedyne_fe2o3_core::prelude::*;

/// A control command dispatched to the running server.
#[derive(Clone, Debug)]
pub enum Command {
    /// Placeholder work request for the server to act upon.
    DoSomething,
    /// Instructs the server to shut down.
    Finish,
}
