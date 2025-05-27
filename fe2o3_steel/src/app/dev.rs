use crate::app::constant;

use oxedize_fe2o3_core::prelude::*;

use std::{
    fs,
    path::Path,
};


pub fn setup(app_root: &Path) -> Outcome<String> {

    // Check if this looks like an existing website.
    let existing_indicators = [
        "www/public/index.html",
        "www/src/index.html", 
        "www/public/main.js",
        "www/src/js",
        "package.json",
    ];
    
    let mut existing_files = Vec::new();
    for &indicator in &existing_indicators {
        let path = app_root.join(indicator);
        if res!(fs::exists(&path)) {
            existing_files.push(indicator);
        }
    }

    if !existing_files.is_empty() {
        info!("Detected existing website with: {:?}", existing_files);
        return Ok(fmt!("Existing website detected, skipping dev initialisation. \
            Found: {}", existing_files.join(", ")));
    }

    // Only create new structure if nothing exists.
    for dir in constant::INIT_TREE_HALT.iter() {
        let dir_path = app_root.join(dir);
        if res!(fs::exists(&dir_path)) {
            return Ok(fmt!("Directory {:?} exists, skipping dev setup.", dir_path));
        }
    }

    info!("No existing website detected, creating development structure.");

    // Halt initialisation if certain existing directories are detected.
    for dir in constant::INIT_TREE_HALT.iter() {
        let dir_path = app_root.join(dir);
        if res!(fs::exists(&dir_path)) {
            return Ok(fmt!("{:?} exists, bypassing dev initialisation.", dir_path));
        }
    }

    info!("Creating development tree because no existing web code was detected.");
    for dir in constant::DEV_TREE_CREATE.iter() {
        let dir_path = app_root.join(dir);
        res!(fs::create_dir_all(&dir_path));
        info!(" Created: {}", dir_path.display());
    }

    // Create HTML files.
    let html_index = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Database Test Interface</title>
    <link rel="stylesheet" href="/styles.css">
</head>
<body>
    <div class="container">

        <img src="/assets/img/logo.svg" alt="Hematite Rust Library" height="30"/>

        <h2>Database Test Interface</h2>

        <div class="input-group">
            <label>Key:</label>
            <input type="text" id="keyInput" placeholder="Enter key">
        </div>

        <div class="input-group">
            <label>Value:</label>
            <input type="text" id="valueInput" placeholder="Enter value">
        </div>

        <div class="input-group">
            <button id="storeBtn">Store</button>
            <button id="retrieveBtn">Retrieve</button>
        </div>

        <div id="status"></div>
    </div>

    <script type="module" src="/bundles/js/main.bundle.js"></script>
</body>
</html>"#;    

    let html_admin = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Admin Interface</title>
    <link rel="stylesheet" href="/styles.css">
</head>
<body>
    <div class="container">
        <img src="/assets/img/logo.svg" alt="Hematite Rust Library" height="30"/>
        <h2>Admin Interface</h2>
        <!-- Admin-specific content here -->
        <div id="status"></div>
    </div>
    <script type="module" src="/bundles/js/admin.bundle.js"></script>
</body>
</html>
"#;

    // Create WebSocket utility module.
    let js_websocket = r#"export class WebSocketManager {
    constructor(statusCallback) {
        this.ws = null;
        this.reconnectAttempts = 0;
        this.MAX_RECONNECT_ATTEMPTS = 3;
        this.reconnectTimeout = null;
        this.statusCallback = statusCallback;
    }

    connect() {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) return;

        if (this.reconnectTimeout) {
            clearTimeout(this.reconnectTimeout);
            this.reconnectTimeout = null;
        }

        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsUrl = `${wsProtocol}//${window.location.host}/ws`;
        
        try {
            this.ws = new WebSocket(wsUrl);

            this.ws.onopen = () => {
                console.log('WebSocket connected');
                this.statusCallback('Connected to server.');
                this.reconnectAttempts = 0;
            };

            this.ws.onclose = (event) => {
                this.ws = null;
                console.log('WebSocket closed:', event.code, event.reason || '<no reason>');
                
                // Only attempt reconnect if we haven't exceeded attempts and it wasn't a clean closure
                if (this.reconnectAttempts < this.MAX_RECONNECT_ATTEMPTS && event.code !== 1000) {
                    this.reconnectAttempts++;
                    const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts - 1), 10000);
                    this.statusCallback(`Connection lost. Retry attempt ${this.reconnectAttempts} in ${delay/1000} seconds...`);
                    this.reconnectTimeout = setTimeout(() => this.connect(), delay);
                } else {
                    const msg = event.code === 1000 
                        ? 'Connection closed normally.'
                        : 'Unable to connect to server. Please refresh the page to try again.';
                    this.statusCallback(msg);
                }
            };

            this.ws.onerror = (error) => {
                console.error('WebSocket error:', error);
                this.statusCallback('Connection error occurred.');
            };

        } catch (error) {
            console.error('Failed to create WebSocket:', error);
            this.statusCallback('Failed to create connection.');
        }
    }

    send(message) {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            try {
                this.ws.send(message);
                return true;
            } catch (error) {
                console.error('Send error:', error);
                this.statusCallback('Failed to send message.');
                return false;
            }
        }
        return false;
    }

    onMessage(callback) {
        if (this.ws) {
            this.ws.onmessage = callback;
        }
    }

    close() {
        if (this.reconnectTimeout) {
            clearTimeout(this.reconnectTimeout);
            this.reconnectTimeout = null;
        }

        if (this.ws) {
            try {
                if (this.ws.readyState === WebSocket.OPEN) {
                    this.ws.close(1000, 'Normal closure');
                    this.statusCallback('Connection closed cleanly.');
                }
            } catch (error) {
                console.error('Error during WebSocket closure:', error);
            } finally {
                this.ws = null;
            }
        }
    }
}"#;

    // Create main application module.
    //let js_main_index = r#"import { WebSocketManager } from '../../utils/websocket.mjs';
    let js_main_index = r#"import { WebSocketManager } from '@utils/websocket.mjs';

class App {
    constructor() {
        console.log('App initialising...');
        
        // Bind methods to instance
        this.storeData = this.storeData.bind(this);
        this.retrieveData = this.retrieveData.bind(this);
        this.showStatus = this.showStatus.bind(this);
        
        this.wsManager = new WebSocketManager((msg) => {
            console.log('Status update:', msg);
            this.showStatus(msg);
        });

        // Initialize immediately
        this.init();
    }

    init() {
        // Check if DOM is loaded
        if (document.readyState === 'loading') {
            document.addEventListener('DOMContentLoaded', () => this.setupAll());
        } else {
            this.setupAll();
        }
    }

    setupAll() {
        console.log('Setting up all components...');
        this.setupWebSocket();
        this.setupWindowEvents();
        this.setupButtons();
    }

    showStatus(message) {
        console.log('Status message:', message);
        const status = document.getElementById('status');
        if (status) {
            status.textContent = message;
        } else {
            console.error('Status element not found.');
        }
    }

    setupButtons() {
        console.log('Setting up buttons...');
        const storeBtn = document.getElementById('storeBtn');
        const retrieveBtn = document.getElementById('retrieveBtn');
        
        if (storeBtn) {
            console.log('Store button found, adding listener.');
            storeBtn.onclick = () => {
                console.log('Store button clicked.');
                this.storeData();
            };
        } else {
            console.error('Store button not found.');
        }

        if (retrieveBtn) {
            console.log('Retrieve button found, adding listener.');
            retrieveBtn.onclick = () => {
                console.log('Retrieve button clicked.');
                this.retrieveData();
            };
        } else {
            console.error('Retrieve button not found.');
        }
    }

    setupWebSocket() {
        console.log('Setting up WebSocket...');
        this.wsManager.connect();
        this.wsManager.onMessage((event) => {
            const response = event.data;
            console.log('Received WebSocket message:', response);
    
            if (response.startsWith('data')) {
                const value = response.split(' ')[1].replace(/['"]/g, '');
                const valueInput = document.getElementById('valueInput');
                if (valueInput) {
                    valueInput.value = value;
                    this.showStatus('Data retrieved successfully.');
                }
            } else {
                this.showStatus(`Server response: ${response}`);
            }
        });
    }

    setupWindowEvents() {
        window.addEventListener('beforeunload', () => {
            console.log('Window closing, cleaning up...');
            this.wsManager.close();
        });
    }

    storeData() {
        console.log('storeData called');
        const keyInput = document.getElementById('keyInput');
        const valueInput = document.getElementById('valueInput');
        
        if (!keyInput || !valueInput) {
            console.error('Input elements not found.');
            return;
        }

        const key = keyInput.value.trim();
        const value = valueInput.value.trim();
    
        if (!key || !value) {
            this.showStatus('Both key and value are required');
            return;
        }
    
        const message = `insert (t2|[(str|${key}),(str|${value})])`;
        console.log('Sending message:', message);

        if (!this.wsManager.send(message)) {
            console.log('WebSocket not ready, attempting to reconnect...');
            this.showStatus('Connecting to server...');
            this.wsManager.connect();
            setTimeout(() => this.storeData(), 1000);
        }
    }

    retrieveData() {
        console.log('retrieveData called');
        const keyInput = document.getElementById('keyInput');
        
        if (!keyInput) {
            console.error('Key input element not found.');
            return;
        }

        const key = keyInput.value.trim();
    
        if (!key) {
            this.showStatus('Key is required for retrieval.');
            return;
        }
    
        const message = `get_data (str|${key})`;
        console.log('Sending message:', message);

        if (!this.wsManager.send(message)) {
            console.log('WebSocket not ready, attempting to reconnect...');
            this.showStatus('Connecting to server...');
            this.wsManager.connect();
            setTimeout(() => this.retrieveData(), 1000);
        }
    }
}

// Create instance immediately and expose it globally
console.log('Creating App instance...');
window.app = new App();"#;

    // Create admin module.
    let js_admin_index = r#"import { WebSocketManager } from '../../utils/websocket.mjs';

class AdminApp {
    constructor() {
        console.log('Admin app initialising...');

        this.showStatus = this.showStatus.bind(this);

        this.wsManager = new WebSocketManager((msg) => {
            console.log('Status update:', msg);
            this.showStatus(msg);
        });

        this.init();
    }

    init() {
        if (document.readyState === 'loading') {
            document.addEventListener('DOMContentLoaded', () => this.setupAll());
        } else {
            this.setupAll();
        }
    }

    setupAll() {
        console.log('Setting up admin components...');
        this.setupWebSocket();
        this.setupWindowEvents();
    }

    // Re-use other methods from main App but customize for admin...
    showStatus(message) {
        console.log('Status message:', message);
        const status = document.getElementById('status');
        if (status) {
            status.textContent = message;
        }
    }

    setupWebSocket() {
        console.log('Setting up WebSocket...');
        this.wsManager.connect();
        this.wsManager.onMessage((event) => {
            const response = event.data;
            console.log('Received WebSocket message:', response);
            this.showStatus(`Server response: ${response}`);
        });
    }

    setupWindowEvents() {
        window.addEventListener('beforeunload', () => {
            console.log('Window closing, cleaning up...');
            this.wsManager.close();
        });
    }
}

console.log('Creating AdminApp instance...');
window.adminApp = new AdminApp();
"#;

    // Create base SCSS file.
    let base_styles = r#"// Import components
@use 'common' as *;

// Base styles
.container {
    margin: 20px;
    padding: 10px;
}"#;

    // Create common styles.
    let common_styles = r#"$primary-color: #4CAF50;
$hover-color: #45a049;
$border-color: #ccc;
$status-bg: #f8f8f8;

@mixin button-style {
    padding: 5px 15px;
    margin-right: 10px;
    background: $primary-color;
    color: white;
    border: none;
    cursor: pointer;
    &:hover {
        background: $hover-color;
    }
}"#;

    // Create input component styles.
    let input_styles = r#"@use '../common' as *;

.input-group {
    margin-bottom: 15px;

    label {
        display: inline-block;
        width: 50px;
        margin-right: 10px;
    }

    input {
        padding: 5px;
        margin-right: 10px;
        border: 1px solid $border-color;
        width: 200px;
    }

    button {
        @include button-style;
    }
}"#;

    // Create status component styles.
    let status_styles = r#"@use '../common' as *;

#status {
    margin-top: 20px;
    padding: 10px;
    background: $status-bg;
    border-left: 4px solid $primary-color;
}"#;

    // Create main page styles.
    let main_styles = r#"@use './common' as *;
@use 'components/input';
@use 'components/status';

.container {
    margin: 20px;
    padding: 10px;
}
"#;

    // Create admin page styles.
    let admin_styles = r#"@use './common' as *;
@use 'components/input';
@use 'components/status';

.container {
    margin: 20px;
    padding: 10px;
}
"#;

    // Create status component styles.
    let logo_svg = include_str!("logo.svg");

    // Write all files.
    let files = [
        ("www/public/index.html", html_index),
        ("www/public/admin.html", html_admin),
        ("www/public/assets/img/logo.svg", logo_svg),
        ("www/src/js/utils/websocket.mjs", js_websocket),
        ("www/src/js/pages/main/index.mjs", js_main_index),
        ("www/src/js/pages/admin/index.mjs", js_admin_index),
        ("www/src/styles/base.scss", base_styles),
        ("www/src/styles/_common.scss", common_styles),
        ("www/src/styles/components/_input.scss", input_styles),
        ("www/src/styles/components/_status.scss", status_styles),
        ("www/src/styles/main.scss", main_styles),
        ("www/src/styles/admin.scss", admin_styles),
    ];

    for (path, content) in files.iter() {
        let file_path = app_root.join(path);
        if let Some(parent) = file_path.parent() {
            res!(fs::create_dir_all(parent));
        }
        res!(fs::write(&file_path, content));
        info!(" Created: {}", file_path.display());
    }

    Ok(fmt!(""))
}

pub fn ensure_compatibility(app_root: &Path) -> Outcome<()> {
    info!("Ensuring compatibility with existing website structure...");

    // Create any missing directories that Steel needs.
    let required_dirs = [
        "tls/dev",
        "tls/prod",
        "www/logs",
    ];

    for dir in &required_dirs {
        let dir_path = app_root.join(dir);
        if !dir_path.exists() {
            res!(fs::create_dir_all(&dir_path));
            info!("Created required directory: {}", dir);
        }
    }

    // Create minimal missing files if needed
    let index_path = app_root.join("www/public/index.html");
    if !index_path.exists() {
        warn!("No index.html found in www/public/. Creating minimal placeholder.");
        let minimal_html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Steel Server</title>
</head>
<body>
    <h1>Welcome</h1>
    <p>Steel server is running. Please add your content to www/public/</p>
</body>
</html>"#;
        res!(fs::write(&index_path, minimal_html));
        info!("Created minimal index.html");
    }

    Ok(())
}
