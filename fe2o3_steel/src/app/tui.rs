#![forbid(unsafe_code)]

use crate::{
    app::{
        cfg::AppConfig,
        constant,
        dev,
        ext::{
            AppExtension,
            NoExtension,
        },
        repl::AppShellContext,
        syntax as app_syntax,
    },
    srv::{
        api::ApiHandlerRegistry,
        webhook::WebhookRegistry,
    },
};

use std::sync::{
    Arc,
    RwLock,
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
    Secret,
};


#[derive(Clone)]
pub enum State {
    NotStarted,
    Running,
    NotResponsive,
}

impl Default for State { fn default() -> Self { Self::NotStarted } }

#[derive(Clone, Default)]
pub struct AppStatus {
    pub first:  bool,
    pub log:    State,
    pub db:     State,
    pub web:    State,
}

/// Start the Steel application with no app extension.
///
/// This is the entry point for the stock `steel` binary. Apps that
/// need custom webhook handlers, API handlers or shell commands
/// should implement `AppExtension` and call [`run_with_extension`]
/// instead.
pub fn run() -> Outcome<()> {
    run_with_extension(NoExtension)
}

/// Start the Steel application with a custom app extension.
///
/// App binaries implement `AppExtension` for their integration
/// surface and call this entry point:
/// ```rust
/// struct MyApp;
/// impl AppExtension for MyApp { /* ... */ }
///
/// fn main() -> Outcome<()> {
///     run_with_extension(MyApp)
/// }
/// ```
///
/// Steel uses the extension to:
/// * populate the shell Syntax tree with the app's own commands
///   (so `./steel help` lists them with proper categories and args);
/// * build the server-wide webhook handler registry from
///   `AppExtension::webhook_handlers`;
/// * build the server-wide API handler registry from
///   `AppExtension::api_handlers`;
/// * dispatch any shell command not owned by Steel through
///   `AppExtension::dispatch_cmd`.
pub fn run_with_extension<E: AppExtension>(extension: E) -> Outcome<()> {

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

        println!("Welcome to the Hematite Steel Server, this appears to be a new app.");
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
    info!("┌───────────────────────┐");
    info!("│ New shell session.    │");
    info!("└───────────────────────┘");

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ BOOTSTRAP BYPASS FOR WALLET MIGRATE                                   │
    // │                                                                       │
    // │ If the caller is invoking `wallet --migrate`, the wallet is still in  │
    // │ the legacy passphrase-only layout and the normal unlock flow cannot  │
    // │ read it. Detect this one subcommand here and dispatch straight into  │
    // │ the migration routine, bypassing the unlock step entirely. All other │
    // │ subcommands require a successful unlock first.                        │
    // └───────────────────────────────────────────────────────────────────────┘
    let invocation_cmds: Vec<String> = std::env::args().skip(1).collect();
    let is_wallet_migrate = {
        let mut saw_wallet = false;
        let mut saw_migrate = false;
        for tok in &invocation_cmds {
            if tok == "wallet" { saw_wallet = true; }
            if tok == "-m" || tok == "--migrate" { saw_migrate = true; }
        }
        saw_wallet && saw_migrate
    };
    if is_wallet_migrate {
        res!(migrate_legacy_wallet_inline(&cfg));
        return Ok(());
    }

    // ┌───────────────────────────────────────────────────────────────────────┐
    // │ WALLET LOAD (NOT UNLOCK)                                              │
    // │                                                                       │
    // │ The wallet holds one or more admin entries. Each admin has its own    │
    // │ password-wrapped copy of the same wallet master key; any of them can  │
    // │ unlock the wallet with their own password. The master key is used as  │
    // │ the Ozone database encryption key.                                    │
    // │                                                                       │
    // │ The wallet is *loaded* here but deliberately NOT unlocked. Steel      │
    // │ starts sealed. The wrapped keys are useless without a passphrase, so  │
    // │ the file is safe to read, and reading it is enough to authenticate an │
    // │ admin later -- which is what lets the dashboard offer an unseal form  │
    // │ that needs no database behind it.                                     │
    // │                                                                       │
    // │ Demanding the passphrase here would make the *database* key a         │
    // │ precondition for the *websites* being up, which is backwards: a       │
    // │ static site does not touch Ozone. It also made every restart an       │
    // │ outage that waited on a human at a terminal, and made a headless      │
    // │ start (systemd, cron) impossible -- no tty, no prompt, crash loop.    │
    // │                                                                       │
    // │ So: no prompt at start-up. `STEEL_ADMIN_PASS` is still honoured for   │
    // │ development and scripted tests, and the operator can type `unseal` at │
    // │ the shell before `server` if they want the databases open from the    │
    // │ first request. Otherwise Steel binds, serves, renews certificates,    │
    // │ and waits for an admin to unseal at /admin.                           │
    // │                                                                       │
    // │ There is still deliberately no disk-resident fallback. A wallet that  │
    // │ can be unlocked with a secret stored on the same disk it protects     │
    // │ provides no real defence against the threat model it was built for -- │
    // │ disk theft.                                                           │
    // └───────────────────────────────────────────────────────────────────────┘
    let wallet_path = Path::new("./").join(constant::WALLET_NAME);
    let (wallet, db_default_enc_key, unlocked_admin_name, unlocked_admin_scopes) =
    if wallet_path.is_file() {
        let wallet = res!(Wallet::load(
            wallet_path,
            Some(DecoderConfig::<(), ()>::default()),
        ));
        // Only the environment variable unlocks eagerly. Absent it, stay
        // sealed -- `AppShellContext::require_master_key` prompts if and
        // when a command actually needs the key.
        match std::env::var(constant::ADMIN_PASS_ENV) {
            Ok(s) => {
                let pass = Secret::new(s);
                let unlocked = match wallet.unlock(pass.expose_secret().as_bytes()) {
                    Ok(u) => u,
                    Err(e) => {
                        println!("Wallet unlock failed: {}.", e);
                        return Ok(());
                    }
                };
                info!("Wallet unlocked by admin '{}' via {}.",
                    unlocked.admin_name, constant::ADMIN_PASS_ENV);
                let key = unlocked.master_key.expose_secret().clone();
                let name = unlocked.admin_name.clone();
                let scopes = unlocked.admin_scopes.clone();
                (wallet, Some(key), name, scopes)
            }
            Err(_) => {
                info!("Wallet loaded, sealed. {} admin entr{} available to unseal.",
                    wallet.admins().len(),
                    if wallet.admins().len() == 1 { "y" } else { "ies" });
                (wallet, None, String::new(), Vec::new())
            }
        }
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
                // A wallet just created from a passphrase the operator typed
                // is, by definition, unlocked -- start unsealed.
                let db_default_enc_key = unlocked.master_key.expose_secret().clone();
                let name = unlocked.admin_name.clone();
                let scopes = unlocked.admin_scopes.clone();
                (wallet, Some(db_default_enc_key), name, scopes)
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
    match dev::setup(&app_root) {
        Ok(s) => {
            if !s.is_empty() {
                warn!("{}", s);
            }
        }
        Err(e) => return Err(err!(e, "While setting up dev environment."; Init)),
    }

    // Build the syntax tree, then let the extension contribute its
    // own commands so they show up in `help` alongside Steel's
    // built-ins.
    let mut syntax_builder = res!(app_syntax::new_shell_raw(
        &cfg.app_human_name,
        &constant::VERSION,
        &fmt!("{} app: {}", cfg.app_human_name, cfg.app_description),
    ));
    syntax_builder = res!(extension.extend_syntax(syntax_builder));
    let syntax = oxedyne_fe2o3_syntax::core::SyntaxRef::new(syntax_builder);

    // Wrap the extension once and use the Arc clones from here on:
    // one clone lives in AppShellContext for CLI dispatch; another
    // is consumed below to drain its handlers into the registries.
    let extension_arc: Arc<dyn AppExtension> = Arc::new(extension);

    // Drain webhook + API handlers from the extension into their
    // registries.
    let mut webhook_registry = WebhookRegistry::new();
    for (name, h) in extension_arc.webhook_handlers() {
        webhook_registry.insert_boxed(name, h);
    }
    let mut api_handler_registry = ApiHandlerRegistry::new();
    for (name, h) in extension_arc.api_handlers() {
        api_handler_registry.insert_boxed(name, h);
    }

    let mut context = AppShellContext {
        stat:                   app_status,
        app_cfg:                cfg.clone(),
        syntax,
        ws:                     BTreeMap::new(),
        db_enc_key:             db_default_enc_key,
        wallet:                 Arc::new(RwLock::new(wallet)),
        unlocked_admin_name,
        unlocked_admin_scopes,
        webhook_registry:       Arc::new(webhook_registry),
        api_handler_registry:   Arc::new(api_handler_registry),
        extension:              extension_arc,
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


/// One-shot migration of a legacy (pre-admin-user) wallet file to
/// the new multi-admin layout.
///
/// Called by the `wallet --migrate` bootstrap bypass in [`run`],
/// before the normal wallet unlock step. Reads the existing wallet
/// file as raw `Dat`, pulls the legacy `wallet_pass_hashes` (for
/// passphrase verification) and `app_hashes.default.kdf_cfg` (for
/// deriving the database encryption key) out of it, verifies the
/// passphrase, derives the current database encryption key -- which
/// becomes the new wallet's master key `K` unchanged, so no Ozone
/// re-encryption is required -- and rewrites the wallet with a
/// single admin entry named "operator" (override via prompt)
/// wrapping the same `K`. The old wallet file is preserved as
/// `wallet.jdat.pre-admins` for rollback.
fn migrate_legacy_wallet_inline(_cfg: &AppConfig) -> Outcome<()> {
    use std::io::Write;
    use oxedyne_fe2o3_core::mem::Extract;
    use oxedyne_fe2o3_hash::kdf::KeyDerivationScheme;
    use oxedyne_fe2o3_iop_hash::kdf::KeyDeriver;
    use oxedyne_fe2o3_crypto::keystore::{
        AdminUser,
        DEFAULT_WALLET_KDF_NAME,
    };

    let wallet_path = Path::new("./").join(constant::WALLET_NAME);
    if !wallet_path.is_file() {
        println!("No wallet file to migrate at {:?}.", wallet_path);
        return Ok(());
    }
    let text = res!(std::fs::read_to_string(&wallet_path));
    let mut dat = res!(Dat::decode_string(&text));
    if dat.kind() != Kind::OrdMap && dat.kind() != Kind::Map {
        return Err(err!(
            "Legacy wallet at {:?} is not a map (kind={:?}).",
            wallet_path, dat.kind();
            Input, Invalid, Mismatch));
    }
    // Already new layout?
    if let Ok(_) = dat.map_get_must(&dat!("admins")) {
        println!("Wallet at {:?} is already in the admin-user layout.",
            wallet_path);
        return Ok(());
    }

    let pass = res!(UserInput::ask_for_secret(
        Some("Enter the current wallet passphrase: "),
    ));
    let pass_bytes = pass.expose_secret().as_bytes();

    // Verify the passphrase against the legacy passhash ring buffer.
    let ring = res!(dat.map_remove_must(&dat!("wallet_pass_hashes")));
    let current_hash_dat = {
        // The ring buffer serialises as `Tup2(list, index)` where
        // `list` is a Vek of `Opt<Tup2(data, timestamp)>`. Extract
        // the current slot's `data` daticle, which is the kdf map.
        let mut ring_parts = oxedyne_fe2o3_jdat::try_extract_tup2dat!(ring);
        let index: u64 = match ring_parts[1].extract() {
            Dat::U64(n) => n,
            other => return Err(err!(
                "Legacy ring buffer index must be u64 (got {:?}).", other.kind();
                Invalid, Input)),
        };
        let list = oxedyne_fe2o3_jdat::try_extract_dat!(
            ring_parts[0].extract(),
            Vek,
        );
        let slot = ok!(list.into_iter().nth(index as usize).ok_or_else(|| err!(
            "Legacy ring buffer index {} out of range.", index;
            Input, Invalid, Mismatch)));
        let some = match slot {
            Dat::Opt(inner) => match *inner {
                Some(d) => d,
                None => return Err(err!(
                    "Legacy ring buffer current slot is None.";
                    Input, Missing)),
            },
            other => return Err(err!(
                "Legacy ring buffer slot must be Opt (got {:?}).", other.kind();
                Input, Invalid, Mismatch)),
        };
        let mut slot_parts = oxedyne_fe2o3_jdat::try_extract_tup2dat!(some);
        slot_parts[0].extract()
    };
    let app_kdf_name = try_extract_dat!(
        res!(current_hash_dat.map_get_must(&dat!("kdf_name"))),
        Str,
    );
    let app_kdf_hash = try_extract_dat!(
        res!(current_hash_dat.map_get_must(&dat!("kdf_hash"))),
        Str,
    );
    let mut app_kdf = res!(KeyDerivationScheme::from_str(&app_kdf_name));
    res!(app_kdf.decode_from_string(&app_kdf_hash));
    if !res!(app_kdf.verify(pass_bytes)) {
        println!("Passphrase rejected -- nothing migrated.");
        return Ok(());
    }

    // Derive the legacy database encryption key -- becomes the new
    // master key unchanged.
    let app_hashes = res!(dat.map_remove_must(&dat!("app_hashes")));
    let default_entry = res!(app_hashes.map_get_must(&dat!("default"))).clone();
    let db_kdf_name = try_extract_dat!(
        res!(default_entry.map_get_must(&dat!("kdf_name"))),
        Str,
    );
    let db_kdf_cfg = try_extract_dat!(
        res!(default_entry.map_get_must(&dat!("kdf_cfg"))),
        Str,
    );
    let mut db_kdf = res!(KeyDerivationScheme::from_str(&db_kdf_name));
    res!(db_kdf.decode_cfg_from_string(&db_kdf_cfg));
    res!(db_kdf.derive(pass_bytes));
    let master_key = res!(db_kdf.get_hash()).to_vec();

    let metadata = match dat.map_remove(&dat!("metadata")) {
        Ok(Some(d)) => try_extract_dat!(d, Map),
        _ => DaticleMap::new(),
    };

    print!("New admin name (default 'operator'): ");
    res!(std::io::stdout().flush());
    let mut name_in = String::new();
    res!(std::io::stdin().read_line(&mut name_in));
    let admin_name = match name_in.trim() {
        "" => "operator".to_string(),
        s  => s.to_string(),
    };

    let admin = res!(AdminUser::new(
        admin_name.clone(),
        pass_bytes,
        &master_key,
        DEFAULT_WALLET_KDF_NAME,
        vec!["*".to_string()],
        0,
    ));
    let new_wallet = Wallet::new(metadata, vec![admin], DaticleMap::new());

    let backup_path = Path::new("./").join(fmt!("{}.pre-admins", constant::WALLET_NAME));
    if let Err(e) = std::fs::copy(&wallet_path, &backup_path) {
        return Err(err!(e,
            "Backing up {:?} to {:?}.", wallet_path, backup_path;
            IO, File, Write));
    }
    res!(new_wallet.save(
        &wallet_path,
        "  ",
        Some(EncoderConfig::<(), ()>::default()),
    ));
    println!();
    println!("Migrated wallet to the admin-user layout.");
    println!("  New admin: '{}'", admin_name);
    println!("  Passphrase: unchanged (the one you just typed)");
    println!("  Backup: {:?}", backup_path);
    println!();
    Ok(())
}

