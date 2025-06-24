//! Development mode functionality for automatic rebuilding and page refresh.
//!
//! # Development Strategy
//! The development mode operates in two stages:
//!
//! 1. Source File Processing:
//!    - Monitors `www/src/js` and `www/src/styles` for changes.
//!    - When JavaScript/TypeScript files change, rebundles to `www/public/js/bundle.js`.
//!    - When SCSS files change, recompiles to `www/public/css/styles.css`.
//!
//! 2. Public File Monitoring:
//!    - Watches `www/public` directory for any file changes.
//!    - When bundled files are written or other public files change, notifies clients.
//!    - Excludes temporary files (vim swaps, backups) from triggering refresh.
//!
//! This two-stage approach ensures that source changes trigger rebuilding first,
//! then the resulting file changes in public trigger browser refresh.
use crate::srv::dev::{
    js::{
        FileType,
        JsBundle,
    },
    sass::SassBundle,
};

use oxedyne_fe2o3_core::prelude::*;

use std::{
    path::{
        Path,
        PathBuf,
    },
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
    time::Duration,
};

use notify::{
    RecommendedWatcher,
    Watcher,
    RecursiveMode,
    Event,
    EventKind,
    event::{
        ModifyKind,
        CreateKind,
        RemoveKind,
    },
};
use tokio::sync::broadcast;

/// Handles WebSocket connections from browser clients in development mode to support
/// automatic rebuilding and page refresh when files change. Each client that connects to the 
/// /dev-refresh endpoint gets a DevRefreshHandler instance. When the file watcher
/// detects changes, all connected clients receive a refresh message. The handler
/// ignores any incoming messages from clients as this is a one-way notification
/// system.
#[derive(Clone, Debug)]
pub struct DevRefreshManager {
    sender:             broadcast::Sender<()>,
    running:            Arc<AtomicBool>,
    js_bundles_map:     Vec<(PathBuf, PathBuf)>,
    js_import_aliases:  Vec<(String, PathBuf)>,
    css_paths:          (PathBuf, PathBuf),
    src_path:           PathBuf,
    public_path:        PathBuf,
}

impl DevRefreshManager {

    pub fn new(
        root_path:          &Path,
        js_bundles_map:     Vec<(PathBuf, PathBuf)>,
        js_import_aliases:  Vec<(String, PathBuf)>,
        css_paths:          (PathBuf, PathBuf),
    )
        -> Self
    {
        // Buffer size of 16 should be plenty.
        let (sender, _) = broadcast::channel(16);

        // Validate inputs and filter out invalid ones.
        let valid_js_bundles: Vec<(PathBuf, PathBuf)> = js_bundles_map
            .into_iter()
            .filter(|(src, _)| src.exists())
            .collect();
            
        let valid_css_paths = if css_paths.0.exists() {
            css_paths
        } else {
            (PathBuf::new(), PathBuf::new()) // Empty paths = disabled.
        };
        
        info!("DevRefreshManager initialised with {} JS bundles, CSS: {}",
            valid_js_bundles.len(),
            if valid_css_paths.0.as_os_str().is_empty() {
                "disabled"
            } else {
                "enabled"
            }
        );

        Self {
            sender,
            running:            Arc::new(AtomicBool::new(true)),
            js_bundles_map:     valid_js_bundles,
            js_import_aliases,
            css_paths:          valid_css_paths,
            src_path:           root_path.join("www/src"),
            public_path:        root_path.join("www/public"),
        }
    }

    pub fn get_receiver(&self) -> broadcast::Receiver<()> {
        self.sender.subscribe()
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Determines whether a file should trigger processing based on its path.
    /// Excludes temporary and backup files.
    fn should_process_file(path: &Path) -> bool {
        if path.is_file() {
            if let Some(filename) = path.file_name() {
                if let Some(filename) = filename.to_str() {
                    return !filename.starts_with(".") && !filename.contains('~');
                }
            }
        }
        false
    }

    pub fn refresh(&self) -> Outcome<()> {
        // Only bundle JS if we have valid bundles configured.
        if !self.js_bundles_map.is_empty() {
            res!(self.bundle_js());
        } else {
            debug!("Skipping JS bundling - no bundles configured");
        }
        
        // Only bundle SASS if we have valid CSS paths.
        if !self.css_paths.0.as_os_str().is_empty() && self.css_paths.0.exists() {
            res!(self.bundle_sass());
        } else {
            debug!("Skipping SASS bundling - no valid CSS source directory");
        }
        
        Ok(())
    }

    pub fn bundle_js(&self) -> Outcome<()> {
        Self::js_bundler(
            self.src_path.clone(),
            self.js_bundles_map.clone(),
            self.js_import_aliases.clone(),
        )
    }

    pub fn bundle_sass(&self) -> Outcome<()> {
        Self::sass_bundler(
            &self.css_paths,
        )
    }

    /// Associated function for bundling javascript.
    pub fn js_bundler(
        src_path:           PathBuf,
        js_bundles_map:     Vec<(PathBuf, PathBuf)>,
        js_import_aliases:  Vec<(String, PathBuf)>,
    )
        -> Outcome<()>
    {
        let bundler = JsBundle::new(
            js_bundles_map,
            js_import_aliases,
        );
        
        // Bundle all JS/TS files.
        res!(bundler.bundle_entries(
            &src_path.join("js"),
        ));
        
        debug!("JavaScript/TypeScript bundling completed.");

        Ok(())
    }

    /// Associated function for bundling css.
    pub fn sass_bundler(
        css_paths: &(PathBuf, PathBuf),
    )
        -> Outcome<()>
    {
        let bundler = SassBundle::new();
        
        // Compile all SCSS files.
        res!(bundler.compile_directory(css_paths));
        
        debug!("SCSS compilation completed.");

        Ok(())
    }

    /// Processes source file changes by running appropriate bundler.
    async fn handle_src_change(
        src_path:           PathBuf,
        js_bundles_map:     Vec<(PathBuf, PathBuf)>,
        js_import_aliases:  Vec<(String, PathBuf)>,
        css_paths:          &(PathBuf, PathBuf),
        path:               &Path,
    )
        -> Outcome<()>
    {
        // Determine file type and run appropriate bundler.
        if let Some(ext) = path.extension() {
            if let Some(ext_str) = ext.to_str() {
                match FileType::from_str(ext_str) {
                    Ok(_) => {
                        debug!("JavaScript/TypeScript file changed, rebundling...");
                        res!(Self::js_bundler(
                            src_path,
                            js_bundles_map,
                            js_import_aliases,
                        ));
                    }
                    _ => if ext_str == "scss" || ext_str == "sass" {
                        debug!("SCSS file changed, recompiling...");
                        res!(Self::sass_bundler(
                            css_paths,
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn watch(&self) -> Outcome<()> {
        info!("Starting file watchers for src and public directories.");
        
        let sender = self.sender.clone();
        let src_path = self.src_path.clone();
        let js_bundles_map = self.js_bundles_map.clone();
        let js_import_aliases = self.js_import_aliases.clone();
        let css_paths = self.css_paths.clone();
    
        // Create watcher for source files.
        let mut src_watcher: RecommendedWatcher = res!(notify::recommended_watcher(
            move |res: Result<Event, _>| {
                if let Ok(event) = res {
                    //debug!("Source file event detected: {:?}", event);
                    
                    for path in &event.paths {
                        if !Self::should_process_file(path) {
                            continue;
                        }
                        
                        // Only process actual file modifications.
                        match event.kind {
                            EventKind::Modify(ModifyKind::Data(_)) |
                            EventKind::Create(CreateKind::File) |
                            EventKind::Remove(RemoveKind::File) => {
                                if path.starts_with(&src_path) &&
                                    Self::should_process_file(path)
                                {
                                    let src_path = src_path.clone();
                                    let js_bundles_map = js_bundles_map.clone();
                                    let js_import_aliases = js_import_aliases.clone();
                                    //let css_paths = css_paths.clone();

                                    // Handle source changes asynchronously.
                                    let rt = match tokio::runtime::Runtime::new() {
                                        Ok(rt) => rt,
                                        Err(e) => {
                                            error!(err!(e,
                                                "Failed to create Tokio runtime for bundling.";
                                                Init));
                                            return;
                                        }
                                    };
                                    
                                    if let Err(e) = rt.block_on(Self::handle_src_change(
                                        src_path,
                                        js_bundles_map,
                                        js_import_aliases,
                                        &css_paths,
                                        path,
                                    )) {
                                        // Continue serving if there is a failure.
                                        error!(err!(e,
                                            "Error processing source file change: {:?}", path;
                                            IO, File));
                                    }
                                }
                            }
                            _ => debug!("Ignoring event: {:?}", event.kind),
                        }
                    }
                }
            }
        ));
    
        let public_path = self.public_path.clone();

        // Create watcher for public files.
        let mut public_watcher: RecommendedWatcher = res!(notify::recommended_watcher(
            move |res: Result<Event, _>| {
                if let Ok(event) = res {
                    //debug!("Public file event detected: {:?}", event);
                    
                    // Only trigger on actual file content changes.
                    match event.kind {
                        EventKind::Modify(ModifyKind::Data(_)) |
                        EventKind::Create(CreateKind::File) |
                        EventKind::Remove(RemoveKind::File) => {
                            for path in &event.paths {
                                if path.starts_with(&public_path) &&
                                    Self::should_process_file(path)
                                {
                                    info!("Broadcasting refresh for file: {:?}.", path);
                                    match sender.send(()) {
                                        Ok(_) => debug!("Refresh notification sent successfully."),
                                        Err(e) => debug!("No active subscribers: {}", e),
                                    }
                                    break;
                                }
                            }
                        }
                        _ => debug!("Ignoring event: {:?}", event.kind),
                    }
                }
            }
        ));
    
        info!("Starting source file watcher for: {:?}", self.src_path);
        res!(src_watcher.watch(&self.src_path, RecursiveMode::Recursive));
        info!("Starting public file watcher for: {:?}", self.public_path);
        res!(public_watcher.watch(&self.public_path, RecursiveMode::Recursive));
    
        while self.running.load(Ordering::SeqCst) {
            std::thread::park_timeout(Duration::from_millis(100));
        }
    
        info!("File watchers stopped.");
    
        Ok(())
    }
}

// Modifies HTML content to inject refresh script in dev mode.
pub struct HtmlModifier;

impl HtmlModifier {
    pub fn inject_dev_refresh(html: &str) -> Outcome<String> {
        if !html.contains("</body>") {
            return Ok(html.to_string());
        }
        let dev_refresh_script = r#"
    <script>
    function getTimestamp() {
        return new Date().toISOString();
    }
    const initDevRefresh = () => {
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${wsProtocol}//${window.location.host}/ws`;
        console.log('Connecting to WebSocket:', wsUrl);
        
        const ws = new WebSocket(wsUrl);
        let pingInterval;
        
        ws.onopen = () => {
            console.log('Development refresh connection opened.');
            ws.send('dev_connect');
        };
    
        ws.onmessage = (event) => {
            console.log('Received message:', event.data);
            if (event.data === 'info "connected"') {
            //if (event.data.includes('connected')) {
                console.log('Development refresh connection established.');
                // Start sending periodic pings to keep connection alive
                pingInterval = setInterval(() => {
                    if (ws.readyState === WebSocket.OPEN) {
                        console.log('Sending ping...');
                        ws.send('dev_ping');
                    }
                }, 15000);
            } else if (event.data === 'info "pong"') {
            //} else if (event.data.includes('pong')) {
                console.log('Received dev pong response.');
            } else if (event.data === 'dev_refresh') {
                console.log('Server requested page refresh.');
                window.location.reload();
            }
        };
    
        ws.onclose = (event) => {
            console.log('WebSocket closed:', event.code, event.reason || '<no reason>', event.wasClean);
            //console.log('Development refresh connection closed, attempting reconnect...');
            console.log('Development refresh connection closed');
            if (pingInterval) {
                clearInterval(pingInterval);
            }
            //setTimeout(initDevRefresh, 2000);
        };
    
        ws.onerror = (error) => {
            console.error('WebSocket error:', error);
        };

        // Ensure clean shutdown
        window.addEventListener('beforeunload', () => {
            if (ws.readyState === WebSocket.OPEN) {
                ws.close();
            }
        });
    };
    
    // Start connection when document loads
    if (document.readyState === 'loading') {
        console.log(`${getTimestamp()} Document loading, waiting for DOMContentLoaded`);
        document.addEventListener('DOMContentLoaded', initDevRefresh);
    } else {
        console.log(`${getTimestamp()} Document ready, initialising immediately`);
        initDevRefresh();
    }
    </script>
    </body>"#;
        Ok(html.replace("</body>", dev_refresh_script))
    }
}
