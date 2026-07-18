#![forbid(unsafe_code)]
use crate::{
    app::{
        cfg::AppConfig,
        constant,
        repl::AppShellContext,
        syntax as app_syntax,
    },
    srv::{
        context::new_db,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    log::{
        bot::FileConfig,
    },
};
use oxedyne_fe2o3_crypto::keystore::{
    DEFAULT_WALLET_KDF_NAME,
    Wallet,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
    file::{
        JdatFile,
        JdatMapFile,
    },
    string::{
        dec::DecoderConfig,
        enc::EncoderConfig,
    },
};
use oxedyne_fe2o3_tui::lib_tui::{
    repl::{
        Evaluation,
        ShellConfig,
    },
    input::UserInput,
};
use oxedyne_fe2o3_namex::InNamex;

use std::{
    collections::BTreeMap,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
};

use secrecy::{
    ExposeSecret,
};


/// Lifecycle state of a subsystem within the application.
#[derive(Clone)]
pub enum State {
    /// The subsystem has not yet been started.
    NotStarted,
    /// The subsystem is running.
    Running,
    /// The subsystem is running but not responding.
    NotResponsive,
}

impl Default for State { fn default() -> Self { Self::NotStarted } }

/// Aggregated status of the application's subsystems.
#[derive(Clone, Default)]
pub struct AppStatus {
    /// Whether this is the application's first run.
    pub first:  bool,
    /// State of the logging subsystem.
    pub log:    State,
    /// State of the database subsystem.
    pub db:     State,
    /// State of the web subsystem.
    pub web:    State,
}

/// Runs the interactive terminal application: loads or creates the config,
/// starts logging, unlocks or creates the wallet, opens the database and then
/// either executes command-line arguments or launches the interactive shell.
pub fn run() -> Outcome<()> {

    let mut app_status = AppStatus::default();
    let cwd = res!(std::env::current_dir());
    let cwd_str = res!(cwd.to_str().ok_or(err!(
        "Converting the current working directory path '{:?}' to a string.", cwd;
        Conversion, String)));
    let err_str = fmt!("Failed to obtain the directory name from the current working path '{:?}'.",
        cwd);
    let this_dir = res!(
        res!(cwd.file_name().ok_or(err!("{}", &err_str; Conversion, String)))
        .to_str().ok_or(err!("{}", &err_str; Conversion, String))
    );

    // ┌───────────────────────────────────────────────────────────────────────────────────────────┐
    // │ LOGIN STEP                                                                                │
    // │ The app executable, configuration and wallet files must be co-located but can exist       │
    // │ separately from the application root directory which contains all other data including    │
    // │ the database and logs.  We first try and load the configuration, then the wallet.         │
    // └───────────────────────────────────────────────────────────────────────────────────────────┘
    
    // ┌───────────────────────┐
    // │ Load the config file. │
    // │ It contains the app   │
    // │ root directory,       │
    // │ among other things.   │
    // └───────────────────────┘
    let cfg_path = Path::new("./").join(constant::CONFIG_NAME);

    if !cfg_path.is_file() {
        app_status.first = true;
        let mut cfg = res!(AppConfig::new());

        println!("Welcome to the Hematite Shield Server, this appears to be a new app.");
        println!("You'll now be asked to enter a human name and a description...");
        for (field, prompt) in [
            (&mut cfg.app_human_name, "App human name"),
            (&mut cfg.app_description, "App description"),
        ] {
            print!("{}: ", prompt);
            res!(std::io::stdout().flush());
            let mut input = String::new();
            res!(std::io::stdin().read_line(&mut input));
            *field = input.trim().to_string();
        }
        cfg.app_name = this_dir.to_string();

        res!(cfg.save(&cfg_path, "  ", false));
        println!(
            "There is no {} file, a default has been created at {:?}.",
            constant::CONFIG_NAME, cfg_path,
        );
    }
    let mut cfg = res!(AppConfig::load(cfg_path));
    res!(cfg.check_and_fix());
    println!("Welcome to {}.", cfg.app_human_name);

    // ┌───────────────────────┐
    // │ Start logging.        │
    // └───────────────────────┘
    let mut log_cfg = log_get_config!();
    log_cfg.console = None;
    log_cfg.level = match LogLevel::from_str(&cfg.app_log_level) {
        Ok(level) => level,
        _ => res!(LogLevel::from_str(constant::DEFAULT_LOG_LEVEL)),
    };
    log_cfg.file = Some(FileConfig::new(
        PathBuf::from(cfg.app_root.clone()),
        cfg.app_name.clone(),
        constant::LOG_FILE_EXTENSION.to_string(),
        0,
        None, // No multiple log file archiving, just use same file.
    ));
    log_set_config!(log_cfg);
    println!("Shell now logging at {:?}", log_get_file_path!());
    info!(async_log::stream(), "┌───────────────────────┐");
    info!(async_log::stream(), "│ New shell session.    │");
    info!(async_log::stream(), "└───────────────────────┘");

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ WALLET UNLOCK                                                         │
    // │                                                                       │
    // │ See `fe2o3_crypto::keystore::Wallet` for the data model. Any admin   │
    // │ password unwraps their entry recovers the shared master key, which   │
    // │ is then used as the Ozone database encryption key. Password is read  │
    // │ from stdin (echo off) with an optional `SHIELD_ADMIN_PASS` env var   │
    // │ override for non-interactive test flows.                              │
    // └───────────────────────────────────────────────────────────────────────┘
    let wallet_path = Path::new("./").join(constant::WALLET_NAME);
    let (wallet, db_default_enc_key) = if wallet_path.is_file() {
        let wallet = res!(Wallet::load(
            wallet_path,
            Some(DecoderConfig::<(), ()>::default()),
        ));
        let pass = if let Ok(s) = std::env::var("SHIELD_ADMIN_PASS") {
            secrecy::Secret::new(s)
        } else {
            res!(UserInput::ask_for_secret(None))
        };
        let unlocked = match wallet.unlock(pass.expose_secret().as_bytes()) {
            Ok(u) => u,
            Err(e) => {
                println!("Wallet unlock failed: {}.", e);
                return Ok(());
            }
        };
        info!(async_log::stream(),
            "Wallet unlocked by admin '{}'.", unlocked.admin_name);
        let db_default_enc_key = unlocked.master_key.expose_secret().clone();
        (wallet, db_default_enc_key)
    } else {
        // ┌───────────────────────┐
        // │ Wallet not found.     │
        // └───────────────────────┘
        println!(
            "There is no {} file.\nYou can replace it with a backup and restart, or create a new one.",
            constant::WALLET_NAME,
        );
        println!("What would you like to do?");
        println!("  1. Exit, replace with a backup file, and restart.");
        println!("  2. Create a new {} file.", constant::WALLET_NAME);
        print!("Please choose: ");
        res!(std::io::stdout().flush());
        let mut choice = String::new();
        res!(std::io::stdin().read_line(&mut choice));
        match choice.trim() {
            "1" => {
                println!("Ok, good luck!");
                return Ok(());
            },
            "2" => {
                // ┌───────────────────────┐
                // │ Create the wallet     │
                // │ from scratch with one │
                // │ operator admin entry. │
                // └───────────────────────┘
                println!("Ok, let's create the first wallet admin and passphrase.");
                print!("Admin name (default 'operator'): ");
                res!(std::io::stdout().flush());
                let mut name_in = String::new();
                res!(std::io::stdin().read_line(&mut name_in));
                let admin_name = match name_in.trim() {
                    "" => "operator".to_string(),
                    s  => s.to_string(),
                };
                let pass = res!(UserInput::create_pass(constant::MAX_CREATE_PASS_ATTEMPTS));
                let pass_bytes = pass.expose_secret().as_bytes();

                let mut metadata = BTreeMap::new();
                metadata.insert(dat!("app_name"), dat!(cfg.app_name.clone()));
                metadata.insert(dat!("app_root"), dat!(cfg.app_root.clone()));
                metadata.insert(dat!("this_dir"), dat!(cwd_str));

                let (wallet, unlocked) = res!(Wallet::create_with_first_admin(
                    metadata,
                    admin_name,
                    pass_bytes,
                    DEFAULT_WALLET_KDF_NAME,
                ));
                res!(wallet.save(
                    &wallet_path,
                    "  ",
                    Some(EncoderConfig::<(), ()>::default()),
                ));
                println!("Thank you, {:?} created.", wallet_path);
                let db_default_enc_key = unlocked.master_key.expose_secret().clone();
                (wallet, db_default_enc_key)
            },
            _ => return Err(err!(
                "Invalid response, goodbye!";
                Invalid, Input)),
        }
    };

    // ┌───────────────────────────────────────────────────────────────────────────────────────────┐
    // │ EXECUTION STEP                                                                            │
    // │ Functions can be executed directly from the command line, or within a shell.  If no       │
    // │ commands are supplied, the user is presented with the shell.  The command line is         │
    // │ technically part of the shell.                                                            │
    // └───────────────────────────────────────────────────────────────────────────────────────────┘

    let app_root = Path::new(&cfg.app_root);
    let db_root = app_root.join(constant::DB_DIR);

    let invocation_cmds: Vec<String> = std::env::args().skip(1).collect();

    let syntax = res!(app_syntax::new_shell(
        &cfg.app_human_name,
        &constant::VERSION,
        &fmt!("{} app: {}", cfg.app_human_name, cfg.app_description),
    ));

    let mut context = AppShellContext {
        stat:       app_status,
        app_cfg:    cfg.clone(),
        syntax,
        ws:         BTreeMap::new(),
        db:         res!(new_db(&db_root, &db_default_enc_key)),
        wallet,
    };

    let mut shell_cfg = ShellConfig::default();

    if invocation_cmds.len() > 0 {
        match context.execute(invocation_cmds, &shell_cfg) {
            Ok(evals) => for eval in evals {
                match eval {
                    Evaluation::Output(s) => println!("{}", s),
                    Evaluation::Exit => {
                        println!("Exiting {} now.", cfg.app_human_name);
                    }
                    _ => (),
                }
            }
            Err(e) => {
                //println!("{} error: {}", cfg.app_human_name, e);
                return Err(e);
            }
        }
    } else {
        shell_cfg.greeting_msg = 
            fmt!("Welcome, type \"help\" for a help menu.");
        res!(context.start_shell(&shell_cfg, None));
    }

    Ok(())
}
