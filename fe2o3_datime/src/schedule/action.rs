/// Action system for scheduled tasks
/// 
/// This module defines the Action trait and various implementations for
/// executing different types of scheduled actions.

use oxedyne_fe2o3_core::prelude::*;
use crate::time::CalClock;
use std::{
    fmt,
    sync::Arc,
};

/// Context information passed to actions during execution
#[derive(Debug, Clone)]
pub struct ActionContext {
    /// The scheduled execution time
    pub scheduled_time: CalClock,
    /// The actual execution time
    pub execution_time: CalClock,
    /// Task name
    pub task_name: String,
    /// Number of previous executions
    pub execution_count: u32,
    /// Whether this is a retry attempt
    pub is_retry: bool,
    /// Retry attempt number (0 for first execution)
    pub retry_count: u32,
}

/// Result of action execution
#[derive(Debug, Clone)]
pub enum ActionResult {
    /// Action completed successfully
    Success,
    /// Action completed with a warning message
    Warning(String),
    /// Action failed with an error
    Error(ActionError),
}

/// Error types for action execution
#[derive(Debug, Clone)]
pub enum ActionError {
    /// Execution timeout
    Timeout,
    /// Invalid configuration or parameters
    InvalidConfig(String),
    /// External dependency failure
    ExternalFailure(String),
    /// Action was cancelled
    Cancelled,
    /// Generic execution error
    ExecutionError(String),
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionError::Timeout => write!(f, "Action execution timed out"),
            ActionError::InvalidConfig(msg) => write!(f, "Invalid configuration: {}", msg),
            ActionError::ExternalFailure(msg) => write!(f, "External failure: {}", msg),
            ActionError::Cancelled => write!(f, "Action was cancelled"),
            ActionError::ExecutionError(msg) => write!(f, "Execution error: {}", msg),
        }
    }
}

/// Trait for executable actions in the scheduling system
pub trait Action: Send + Sync + fmt::Debug {
    /// Executes the action with the given context
    fn execute(&self, context: &ActionContext) -> Outcome<ActionResult>;
    
    /// Returns a human-readable description of the action
    fn description(&self) -> &str;
    
    /// Creates a clone of this action (for task cloning)
    fn box_clone(&self) -> Box<dyn Action>;
    
    /// Validates the action configuration
    fn validate(&self) -> Outcome<()> {
        Ok(())
    }
    
    /// Called before execution to prepare the action
    fn prepare(&self, _context: &ActionContext) -> Outcome<()> {
        Ok(())
    }
    
    /// Called after execution for cleanup
    fn cleanup(&self, _context: &ActionContext, _result: &ActionResult) -> Outcome<()> {
        Ok(())
    }
}

/// Action that executes a closure/callback function
#[derive(Clone)]
pub struct CallbackAction {
    callback: Arc<dyn Fn() -> Outcome<()> + Send + Sync>,
    description: String,
}

impl fmt::Debug for CallbackAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallbackAction")
            .field("description", &self.description)
            .field("callback", &"<function>")
            .finish()
    }
}

impl CallbackAction {
    /// Creates a new callback action
    pub fn new<F>(callback: F) -> Self 
    where 
        F: Fn() -> Outcome<()> + Send + Sync + 'static
    {
        CallbackAction {
            callback: Arc::new(callback),
            description: "Callback action".to_string(),
        }
    }
    
    /// Creates a new callback action with description
    pub fn with_description<F, S>(callback: F, description: S) -> Self 
    where 
        F: Fn() -> Outcome<()> + Send + Sync + 'static,
        S: Into<String>
    {
        CallbackAction {
            callback: Arc::new(callback),
            description: description.into(),
        }
    }
}

impl Action for CallbackAction {
    fn execute(&self, _context: &ActionContext) -> Outcome<ActionResult> {
        match (self.callback)() {
            Ok(()) => Ok(ActionResult::Success),
            Err(e) => Ok(ActionResult::Error(ActionError::ExecutionError(e.to_string()))),
        }
    }
    
    fn description(&self) -> &str {
        &self.description
    }
    
    fn box_clone(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

/// Action that executes a system command
#[derive(Debug, Clone)]
pub struct CommandAction {
    command: String,
    args: Vec<String>,
    working_dir: Option<String>,
    env_vars: Vec<(String, String)>,
    description: String,
}

impl CommandAction {
    /// Creates a new command action
    pub fn new<S: Into<String>>(command: S) -> Self {
        let cmd = command.into();
        CommandAction {
            description: format!("Command: {}", cmd),
            command: cmd,
            args: Vec::new(),
            working_dir: None,
            env_vars: Vec::new(),
        }
    }
    
    /// Adds arguments to the command
    pub fn args<I, S>(mut self, args: I) -> Self 
    where 
        I: IntoIterator<Item = S>,
        S: Into<String>
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
    
    /// Sets the working directory
    pub fn working_dir<S: Into<String>>(mut self, dir: S) -> Self {
        self.working_dir = Some(dir.into());
        self
    }
    
    /// Adds environment variables
    pub fn env<K, V>(mut self, key: K, value: V) -> Self 
    where 
        K: Into<String>,
        V: Into<String>
    {
        self.env_vars.push((key.into(), value.into()));
        self
    }
}

impl Action for CommandAction {
    fn execute(&self, _context: &ActionContext) -> Outcome<ActionResult> {
        use std::process::Command;
        
        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args);
        
        if let Some(ref dir) = self.working_dir {
            cmd.current_dir(dir);
        }
        
        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }
        
        match cmd.output() {
            Ok(output) => {
                if output.status.success() {
                    Ok(ActionResult::Success)
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Ok(ActionResult::Error(ActionError::ExecutionError(
                        format!("Command failed with exit code {:?}: {}", output.status.code(), stderr)
                    )))
                }
            },
            Err(e) => Ok(ActionResult::Error(ActionError::ExecutionError(
                format!("Failed to execute command: {}", e)
            ))),
        }
    }
    
    fn description(&self) -> &str {
        &self.description
    }
    
    fn validate(&self) -> Outcome<()> {
        // Basic validation - check if command exists
        if self.command.is_empty() {
            return Err(err!("Command cannot be empty"; Invalid, Input));
        }
        Ok(())
    }
    
    fn box_clone(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

/// Action that writes to a log file
#[derive(Debug, Clone)]
pub struct LogAction {
    message: String,
    log_file: Option<String>,
    description: String,
}

impl LogAction {
    /// Creates a new log action that writes to stdout
    pub fn new<S: Into<String>>(message: S) -> Self {
        LogAction {
            message: message.into(),
            log_file: None,
            description: "Log message".to_string(),
        }
    }
    
    /// Creates a new log action that writes to a file
    pub fn to_file<M, F>(message: M, file_path: F) -> Self 
    where 
        M: Into<String>,
        F: Into<String>
    {
        let file = file_path.into();
        LogAction {
            message: message.into(),
            description: format!("Log to file: {}", file),
            log_file: Some(file),
        }
    }
}

impl Action for LogAction {
    fn execute(&self, context: &ActionContext) -> Outcome<ActionResult> {
        let timestamp = context.execution_time.to_string();
        let log_entry = format!("[{}] Task '{}': {}", timestamp, context.task_name, self.message);
        
        if let Some(ref file_path) = self.log_file {
            match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(file_path) 
            {
                Ok(mut file) => {
                    use std::io::Write;
                    if let Err(e) = writeln!(file, "{}", log_entry) {
                        return Ok(ActionResult::Error(ActionError::ExecutionError(
                            format!("Failed to write to log file: {}", e)
                        )));
                    }
                },
                Err(e) => {
                    return Ok(ActionResult::Error(ActionError::ExecutionError(
                        format!("Failed to open log file: {}", e)
                    )));
                }
            }
        } else {
            println!("{}", log_entry);
        }
        
        Ok(ActionResult::Success)
    }
    
    fn description(&self) -> &str {
        &self.description
    }
    
    fn box_clone(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

/// Action that sends an HTTP request
#[derive(Debug, Clone)]
pub struct HttpAction {
    url: String,
    #[allow(dead_code)]
    method: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
    description: String,
}

impl HttpAction {
    /// Creates a new HTTP GET action
    pub fn get<S: Into<String>>(url: S) -> Self {
        let url_str = url.into();
        HttpAction {
            description: format!("HTTP GET: {}", url_str),
            url: url_str,
            method: "GET".to_string(),
            headers: Vec::new(),
            body: None,
        }
    }
    
    /// Creates a new HTTP POST action
    pub fn post<S: Into<String>>(url: S) -> Self {
        let url_str = url.into();
        HttpAction {
            description: format!("HTTP POST: {}", url_str),
            url: url_str,
            method: "POST".to_string(),
            headers: Vec::new(),
            body: None,
        }
    }
    
    /// Adds a header to the request
    pub fn header<K, V>(mut self, key: K, value: V) -> Self 
    where 
        K: Into<String>,
        V: Into<String>
    {
        self.headers.push((key.into(), value.into()));
        self
    }
    
    /// Sets the request body
    pub fn body<S: Into<String>>(mut self, body: S) -> Self {
        self.body = Some(body.into());
        self
    }
}

impl Action for HttpAction {
    fn execute(&self, _context: &ActionContext) -> Outcome<ActionResult> {
        // Note: In a real implementation, you'd use a proper HTTP client like reqwest
        // For now, this is a placeholder that simulates HTTP requests
        
        if self.url.is_empty() {
            return Ok(ActionResult::Error(ActionError::InvalidConfig(
                "URL cannot be empty".to_string()
            )));
        }
        
        // Simulate HTTP request
        if self.url.starts_with("https://") || self.url.starts_with("http://") {
            Ok(ActionResult::Success)
        } else {
            Ok(ActionResult::Error(ActionError::InvalidConfig(
                "Invalid URL format".to_string()
            )))
        }
    }
    
    fn description(&self) -> &str {
        &self.description
    }
    
    fn validate(&self) -> Outcome<()> {
        if self.url.is_empty() {
            return Err(err!("URL cannot be empty"; Invalid, Input));
        }
        
        if !self.url.starts_with("http://") && !self.url.starts_with("https://") {
            return Err(err!("URL must start with http:// or https://"; Invalid, Input));
        }
        
        Ok(())
    }
    
    fn box_clone(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

/// Composite action that executes multiple actions in sequence
#[derive(Debug)]
pub struct CompositeAction {
    actions: Vec<Box<dyn Action>>,
    description: String,
    stop_on_failure: bool,
}

impl Clone for CompositeAction {
    fn clone(&self) -> Self {
        CompositeAction {
            actions: self.actions.iter().map(|a| a.box_clone()).collect(),
            description: self.description.clone(),
            stop_on_failure: self.stop_on_failure,
        }
    }
}

impl CompositeAction {
    /// Creates a new composite action
    pub fn new<S: Into<String>>(description: S) -> Self {
        CompositeAction {
            actions: Vec::new(),
            description: description.into(),
            stop_on_failure: true,
        }
    }
    
    /// Adds an action to the sequence
    pub fn add_action<A: Action + 'static>(mut self, action: A) -> Self {
        self.actions.push(Box::new(action));
        self
    }
    
    /// Sets whether to stop execution on first failure
    pub fn stop_on_failure(mut self, stop: bool) -> Self {
        self.stop_on_failure = stop;
        self
    }
}

impl Action for CompositeAction {
    fn execute(&self, context: &ActionContext) -> Outcome<ActionResult> {
        let mut warnings = Vec::new();
        
        for (i, action) in self.actions.iter().enumerate() {
            match res!(action.execute(context)) {
                ActionResult::Success => continue,
                ActionResult::Warning(msg) => {
                    warnings.push(format!("Action {}: {}", i + 1, msg));
                    continue;
                },
                ActionResult::Error(err) => {
                    if self.stop_on_failure {
                        return Ok(ActionResult::Error(err));
                    } else {
                        warnings.push(format!("Action {} failed: {}", i + 1, err));
                        continue;
                    }
                }
            }
        }
        
        if warnings.is_empty() {
            Ok(ActionResult::Success)
        } else {
            Ok(ActionResult::Warning(warnings.join("; ")))
        }
    }
    
    fn description(&self) -> &str {
        &self.description
    }
    
    fn validate(&self) -> Outcome<()> {
        for action in &self.actions {
            res!(action.validate());
        }
        Ok(())
    }
    
    fn box_clone(&self) -> Box<dyn Action> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::CalClockZone;

    #[test]
    fn test_callback_action() {
        let action = CallbackAction::new(|| {
            println!("Test callback executed");
            Ok(())
        });
        
        let context = ActionContext {
            scheduled_time: CalClock::new(2024, 1, 1, 12, 0, 0, 0, CalClockZone::utc()).unwrap(),
            execution_time: CalClock::new(2024, 1, 1, 12, 0, 1, 0, CalClockZone::utc()).unwrap(),
            task_name: "test".to_string(),
            execution_count: 1,
            is_retry: false,
            retry_count: 0,
        };
        
        let result = action.execute(&context).unwrap();
        assert!(matches!(result, ActionResult::Success));
    }

    #[test]
    fn test_log_action() {
        let action = LogAction::new("Test log message");
        
        let context = ActionContext {
            scheduled_time: CalClock::new(2024, 1, 1, 12, 0, 0, 0, CalClockZone::utc()).unwrap(),
            execution_time: CalClock::new(2024, 1, 1, 12, 0, 1, 0, CalClockZone::utc()).unwrap(),
            task_name: "test_task".to_string(),
            execution_count: 1,
            is_retry: false,
            retry_count: 0,
        };
        
        let result = action.execute(&context).unwrap();
        assert!(matches!(result, ActionResult::Success));
    }

    #[test]
    fn test_command_action_validation() {
        let action = CommandAction::new("ls").args(["-la"]);
        assert!(action.validate().is_ok());
        
        let empty_action = CommandAction::new("");
        assert!(empty_action.validate().is_err());
    }

    #[test]
    fn test_http_action_validation() {
        let valid_action = HttpAction::get("https://example.com");
        assert!(valid_action.validate().is_ok());
        
        let invalid_action = HttpAction::get("invalid-url");
        assert!(invalid_action.validate().is_err());
        
        let empty_action = HttpAction::get("");
        assert!(empty_action.validate().is_err());
    }

    #[test]
    fn test_composite_action() {
        let composite = CompositeAction::new("Test composite")
            .add_action(LogAction::new("First action"))
            .add_action(LogAction::new("Second action"))
            .stop_on_failure(false);
        
        assert!(composite.validate().is_ok());
        
        let context = ActionContext {
            scheduled_time: CalClock::new(2024, 1, 1, 12, 0, 0, 0, CalClockZone::utc()).unwrap(),
            execution_time: CalClock::new(2024, 1, 1, 12, 0, 1, 0, CalClockZone::utc()).unwrap(),
            task_name: "composite_test".to_string(),
            execution_count: 1,
            is_retry: false,
            retry_count: 0,
        };
        
        let result = composite.execute(&context).unwrap();
        assert!(matches!(result, ActionResult::Success));
    }
}