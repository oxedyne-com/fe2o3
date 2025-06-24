#![forbid(unsafe_code)]

use crate::{
    app::{
        cfg::AppConfig,
        constant,
        dev,
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
use oxedyne_fe2o3_crypto::{
    keys::Wallet,
};
use oxedyne_fe2o3_data::{
    ring::RingBuffer,
    time::Timestamped,
};
use oxedyne_fe2o3_hash::{
    kdf::KeyDerivationScheme,
};
use oxedyne_fe2o3_iop_hash::{
    kdf::KeyDeriver,
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

    // ┌───────────────────────┐
    // │ The config is loaded, │
    // │ now load the wallet   │
    // │ file.                 │
    // └───────────────────────┘
    // ---------------------------------------------------------------------------------------------
    // It contains the hash of the passphrase, used for overall authentication, as well as the KDF
    // configurations for deriving database encryption keys.  The wallet file itself is open and
    // contains no encryption itself.
    // ---------------------------------------------------------------------------------------------
    const PH: usize = constant::NUM_PREV_PASSHASHES_TO_RETAIN;
    let wallet_path = Path::new("./").join(constant::WALLET_NAME);
    let (wallet, db_default_enc_key) = if wallet_path.is_file() {
        // ┌───────────────────────┐
        // │ The wallet exists,    │
        // │ load it, authenticate │
        // │ and derive the        │
        // │ default database key. │
        // └───────────────────────┘
        // -----------------------------------------------------------------------------------------
        // The wallet contains:
        // - metadata:
        //  Persisting some kind data out in the open,
        // - app_hashes:
        //  Provides hashes when users are required to enter passphrases for access,
        // - app_encrypted_secrets:
        //  Provides encrypted keys when the plain keys are required, decrypted by the wallet
        //  passphrase,
        // - wallet_pass_hashes:
        //  Provides the current and historical wallet passphrase hashes, as a fixed size ring
        //  buffer.  This map includes a U64 index identifying the current passphrase hash.
        // -----------------------------------------------------------------------------------------
        let wallet = res!(Wallet::<{PH}, Dat>::load(wallet_path, Some(DecoderConfig::<(), ()>::default())));
        let kdf_map_dat = match wallet.kdf_cfgs().get(&dat!("default")) {
            Some(map_dat) => map_dat,
            None => return Err(err!(
                "The wallet does not contain a 'default' KDF entry.";
                Data, Configuration, Missing)),
        };
        let db_default_kdf_name = try_extract_dat!(
            res!(kdf_map_dat.map_get_must(&dat!("kdf_name"))),
            Str,
        );
        let mut db_default_kdf = res!(KeyDerivationScheme::from_str(&db_default_kdf_name));
        let db_default_kdf_cfg = try_extract_dat!(
            res!(kdf_map_dat.map_get_must(&dat!("kdf_cfg"))),
            Str,
        );
        let app_kdf = match wallet.passhashes().get() {
            Some(Timestamped { data: kdf_dat, .. }) => {
                let kdf_name = try_extract_dat!(res!(kdf_dat.map_get_must(&dat!("kdf_name"))), Str);
                let kdf_hash = try_extract_dat!(res!(kdf_dat.map_get_must(&dat!("kdf_hash"))), Str);
                let mut app_kdf = res!(KeyDerivationScheme::from_str(&kdf_name));
                res!(app_kdf.decode_from_string(kdf_hash));
                app_kdf
            }
            None => return Err(err!(
                "The current passhash is None in {}.",
                constant::WALLET_NAME;
                Data, Configuration, Missing)),
        };
        let pass = res!(UserInput::ask_for_secret(None));
        let pass = pass.expose_secret().as_bytes();
        if res!(app_kdf.verify(pass)) {
            res!(db_default_kdf.decode_cfg_from_string(&db_default_kdf_cfg));
            res!(db_default_kdf.derive(pass));
            let db_default_enc_key = res!(db_default_kdf.get_hash()).to_vec();
            (wallet, db_default_enc_key)
        } else {
            println!("The passphrase does not match, goodbye!");
            return Ok(());
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
                // │ from scratch.         │
                // └───────────────────────┘
                println!("Ok, let's create the first app wallet passphrase.");
                let pass = res!(UserInput::create_pass(constant::MAX_CREATE_PASS_ATTEMPTS));
                let pass = pass.expose_secret().as_bytes();
                let mut app_kdf = res!(KeyDerivationScheme::from_str(&cfg.kdf_name));
                res!(app_kdf.derive(pass));
                let mut kdf_map = DaticleMap::new();
                kdf_map.insert(dat!("kdf_name"), dat!(fmt!("{}", app_kdf)));
                let kdf_hash = res!(app_kdf.encode_to_string());
                kdf_map.insert(dat!("kdf_hash"), dat!(kdf_hash));
                let mut passhashes = RingBuffer::<{PH}, Timestamped<Dat>>::default();
                passhashes.set(res!(Timestamped::new(dat!(kdf_map))));

                let mut kdf_cfgs = DaticleMap::new();
                let mut db_kdf = res!(KeyDerivationScheme::from_str(&cfg.kdf_name));
                res!(db_kdf.derive(pass));
                let mut kdf_map = DaticleMap::new();
                kdf_map.insert(dat!("kdf_name"), dat!(fmt!("{}", db_kdf)));
                kdf_map.insert(dat!("kdf_nid"), dat!(fmt!("{}", res!(db_kdf.name_id()))));
                kdf_map.insert(dat!("kdf_cfg"), dat!(res!(db_kdf.encode_cfg_to_string())));
                kdf_cfgs.insert(dat!("default"), dat!(kdf_map));
                let db_default_enc_key = res!(db_kdf.get_hash()).to_vec();

                let mut metadata = BTreeMap::new();
                metadata.insert(dat!("app_name"), dat!(cfg.app_name.clone()));
                metadata.insert(dat!("app_root"), dat!(cfg.app_root.clone()));
                metadata.insert(dat!("this_dir"), dat!(cwd_str));

                let wallet = Wallet::<{PH}, Dat>::new(
                    metadata,
                    kdf_cfgs,
                    DaticleMap::new(),
                    passhashes,
                );
                res!(wallet.save(&wallet_path, "  ", Some(EncoderConfig::<(), ()>::default())));
                println!("Thank you, {:?} created.", wallet_path);
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
    match dev::setup(&app_root) {
        Ok(s) => {
            if !s.is_empty() {
                warn!("{}", s);
            }
        }
        Err(e) => return Err(err!(e, "While setting up dev environment."; Init)),
    }

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
