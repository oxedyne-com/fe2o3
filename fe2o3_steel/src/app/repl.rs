use crate::{
    app::{
        cfg::AppConfig,
        constant as app_const,
        ext::AppExtension,
        tui::AppStatus,
    },
    srv::{
        admin::audit,
        api::ApiHandlerRegistry,
        cert::Certificate,
        cfg::ServerConfig,
        constant as srv_const,
        webhook::WebhookRegistry,
    },
};

use std::sync::Arc;

use oxedyne_fe2o3_core::{
    prelude::*,
    mem::Extract,
    path::NormalPath,
};
use oxedyne_fe2o3_crypto::{
    enc::EncryptionScheme,
    keystore::Wallet,
};
use oxedyne_fe2o3_hash::{
    kdf::KeyDerivationScheme,
};
use oxedyne_fe2o3_iop_crypto::{
    keys::KeyManager,
    enc::Encrypter,
};
use oxedyne_fe2o3_iop_hash::kdf::KeyDeriver;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    file::JdatFile,
    string::enc::EncoderConfig,
};
use oxedyne_fe2o3_syntax::{
    core::SyntaxRef,
    help::Help,
    msg::{
        Msg,
        MsgCmd,
    },
    opt::OptionRefVec,
};
use oxedyne_fe2o3_text::base2x;
use oxedyne_fe2o3_tui::lib_tui::{
    cmds,
    repl::{
        Evaluation,
        Shell,
        ShellConfig,
        ShellContext,
        Splitters,
    },
    input::UserInput,
};
use oxedyne_fe2o3_namex::InNamex;

use std::{
    collections::BTreeMap,
    path::{
        Path,
    },
    sync::RwLock,
};

use secrecy::{
    ExposeSecret,
    Secret,
};
use zeroize::Zeroize;


#[derive(Clone)]
pub struct AppShellContext {
    pub stat:               AppStatus,
    pub app_cfg:            AppConfig,
    pub syntax:             SyntaxRef,
    pub ws:                 BTreeMap<Dat, Dat>,
    /// Default database encryption key -- the wallet master key. The
    /// `server` command uses it to open one Ozone instance per
    /// configured vhost.
    ///
    /// `None` until an admin supplies a passphrase. Steel no longer
    /// demands one at start-up: the shell opens sealed, and a command
    /// that genuinely needs the key calls [`Self::require_master_key`],
    /// which prompts at that point. Starting the server while sealed is
    /// legitimate and expected -- see `srv::admin::state`.
    pub db_enc_key:         Option<Vec<u8>>,
    /// The wallet is shared via `Arc<RwLock<_>>` so that the admin
    /// dashboard handler (running inside the HTTPS server task) can
    /// hold a clone of the same wallet that the REPL mutates from
    /// the operator's terminal. CLI admin verbs and dashboard admin
    /// verbs therefore observe each other's changes without any
    /// reload-from-disk dance.
    pub wallet:             Arc<RwLock<Wallet>>,
    /// Name of the admin whose password unlocked the wallet at
    /// start-up. Empty when the wallet was just created. Threaded
    /// through so privileged subcommands do not have to re-prompt
    /// the caller for a password they have already proven once.
    pub unlocked_admin_name:    String,
    /// Scope list of the unlocking admin. Checked by privileged
    /// subcommands to enforce verb-level authorisation.
    pub unlocked_admin_scopes:  Vec<String>,
    /// App-registered webhook handlers, dispatched by name from the
    /// webhook route config.
    pub webhook_registry:   Arc<WebhookRegistry>,
    /// App-registered API handlers, dispatched by name from any
    /// `api_routes` entry that has its `handler` field set.
    pub api_handler_registry: Arc<ApiHandlerRegistry>,
    /// App extension that contributed shell commands and handlers.
    /// Held so the REPL dispatch loop can route unknown commands
    /// through `AppExtension::dispatch_cmd`.
    pub extension:          Arc<dyn AppExtension>,
}

impl ShellContext for AppShellContext {
    fn eval(
        &mut self,
        input:      &String,
        cfg:        &ShellConfig,
        splitters:  &Splitters,
    )
        -> Outcome<Vec<Evaluation>>
    {
        for expr in splitters.command.split(input).into_iter() {
            let parts = splitters.assignment.split(expr.val_ref());
            // 1. try state manipulation
            match parts.len() {
                0 => unreachable!(),
                1 => { // evaluation
                    //let lhs = Dat::decode_string(parts[0].val_ref())?;
                    ////if lhs.kind() != Kind::Str {
                    ////    return Err(Error::Local{
                    ////        tags: vec![ErrTag::Input, ErrTag::Mismatch],
                    ////        kind: ErrKind::Unexpected,
                    ////    	msg: errmsg!(
                    ////        "The left hand side of the assignment is a {:?} but must be a Kind::Str.",
                    ////        lhs.kind(),
                    ////    )});
                    ////}
                    //if let Some(rhs) = state.get_recursive(&lhs) {
                    //    println!("{} = {:?}", lhs, rhs);
                    //} else {
                    //    println!("{:?}", lhs);
                    //}
                    //continue;
                },
                2 => { // assignment lhs = rhs
                    let lhs = res!(Dat::decode_string(parts[0].val_ref()));
                    let rhs = res!(Dat::decode_string(parts[1].val_ref()));
                    if lhs.kind() != Kind::Str {
                        return Err(err!(
                            "The left hand side of the assignment is a {:?} but must be a Kind::Str.",
                            lhs.kind();
                            Input, Mismatch));
                    }
                    self.ws.insert(lhs, rhs);
                    continue;
                },
                _ => return Err(err!(
                    "Only single assignment such as a = b is permitted.";
                    Input, Mismatch)),
            }
            // 2. Try syntax command
            // Split into words and downgrade from phrases to string iterator.
            let mut parts = splitters.word
                .split(expr.val_ref())
                .into_iter()
                .map(|x| x.to_val())
                .peekable();
            // Currently the "echo" command is not in the syntax and therefore not in the help.
            if let Some("echo") = parts.peek().map(|s| s.as_ref()) {
                return Ok(vec![Evaluation::Output(input.clone())]);
            }
            return self.execute(parts, &cfg);
        }
        Ok(vec![Evaluation::None])
    }
}

impl AppShellContext {

    pub fn execute<I: IntoIterator<Item=String>>(
        &mut self,
        parts:      I,
        shell_cfg:  &ShellConfig,
    )
        -> Outcome<Vec<Evaluation>>
    {
        let mut evals = Vec::new();
        let msgrx = Msg::new(self.syntax.clone());
        let msgrx = res!(msgrx.rx_text_iter(
            parts,
            Some(app_const::SYNTAX_CMD_SIMILARITY_THRESHOLD),
        ));  
        for (cmd_key, cmd) in &msgrx.cmds {
            match cmd_key.as_str() {
                "help" => {
                    let help = Help::default(); // TODO consider creating only once?
                    for line in res!(help.to_lines(&self.syntax)) {
                        println!("{}", line);
                    }
                },
                // Control
                "exit"      => evals.push(res!(cmds::exit_shell(&shell_cfg.exit_msg))),
                "server"    => evals.push(res!(self.start_server(&shell_cfg, Some(cmd)))),
                "shell"     => evals.push(res!(self.start_shell(&shell_cfg, Some(cmd)))),
                "cert"      => evals.push(res!(self.manage_certificates(&shell_cfg, Some(cmd)))),
                "acme"      => evals.push(res!(self.manage_acme(&shell_cfg, Some(cmd)))),
                // Filesystem
                "cd"        => evals.push(res!(cmds::change_directory(cmd))),
                "ls"        => evals.push(res!(cmds::list_directory_contents(cmd))),
                "pwd"       => evals.push(res!(cmds::print_working_directory())),
                // Wallet
                "unseal"    => evals.push(res!(self.unseal(&shell_cfg, Some(cmd)))),
                "secrets"   => evals.push(res!(self.secrets(&shell_cfg, Some(cmd)))),
                "wallet"    => evals.push(res!(self.manage_wallet(&shell_cfg, Some(cmd)))),
                "admin"     => evals.push(res!(self.manage_admin(&shell_cfg, Some(cmd)))),
                // Mail
                "mailpass"  => evals.push(res!(self.mailpass(&shell_cfg, Some(cmd)))),
                _ => {
                    // Not a built-in command -- offer it to the app
                    // extension. Cloning the Arc is cheap and lets the
                    // borrow checker see that `self` is not aliased.
                    let ext = self.extension.clone();
                    match res!(ext.dispatch_cmd(cmd_key.as_str(), cmd, shell_cfg)) {
                        Some(eval) => evals.push(eval),
                        None => {
                            warn!("Command '{}' is not implemented.", cmd_key);
                        }
                    }
                }
            }
        }
        Ok(evals)
    }

    pub fn start_shell(
        &mut self,
        shell_cfg:  &ShellConfig,
        _cmd:       Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        let mut shell = res!(Shell::new(
            shell_cfg.clone(),
            self.clone(),
        ));
        res!(shell.start());
        Ok(Evaluation::None)
    }

    pub fn secrets(
        &mut self,
        _shell_cfg:  &ShellConfig,
        cmd:        Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        if let Some(msg_cmd) = cmd {
            if msg_cmd.has_args() {
                if res!(msg_cmd.has_only_arg("create")) {
                    let vals = res!(msg_cmd.get_arg_vals("create").with_len(1));
                    let name = &vals[0];
                    let pass = res!(UserInput::ask_for_secret(None));
                    let mut kdf = res!(KeyDerivationScheme::from_str(&self.app_cfg.kdf_name));
                    let key = res!(UserInput::derive_key(&mut kdf, pass));
                    let already_present = {
                        let w = lock_read!(self.wallet);
                        w.enc_secs().get(name).is_some()
                    };
                    if already_present {
                        if res!(UserInput::ask(
                            fmt!("Encrypted secret '{}' already exists, replace? (Y/N): ", name).as_str(),
                        )).to_lowercase().as_str() != "y" {
                            return Ok(Evaluation::Output(fmt!("Creation of encrypted secret aborted.")));
                        }
                    }
                    let mut map = DaticleMap::new();
                    map.insert(dat!("kdf_name"), dat!(fmt!("{}", kdf)));
                    map.insert(dat!("kdf_nid"), dat!(fmt!("{}", res!(kdf.name_id()))));
                    map.insert(dat!("kdf_cfg"), dat!(res!(kdf.encode_cfg_to_string())));
                    let enc = res!(EncryptionScheme::new_aes_256_gcm_with_key(&key));
                    map.insert(dat!("enc_name"), dat!(fmt!("{:?}", enc)));
                    map.insert(dat!("enc_nid"), dat!(fmt!("{}", res!(enc.name_id()))));
                    let sec = res!(UserInput::ask_for_secret(
                        Some("Enter the secret you want to encrypt: ")
                    ));
                    let enc_sec = res!(enc.encrypt(sec.expose_secret().as_bytes()));
                    let base2x = base2x::HEMATITE64;
                    let b2x_sec = base2x.to_string(&enc_sec);
                    map.insert(dat!("enc_sec"), dat!(b2x_sec));
                    let wallet_path = Path::new("./").join(app_const::WALLET_NAME);
                    {
                        let mut w = lock_write!(self.wallet);
                        if let Some(enc_sec_map) = w.enc_secs_mut().get_mut(name) {
                            *enc_sec_map = dat!(map);
                        } else {
                            w.enc_secs_mut().insert(name.clone(), dat!(map));
                        }
                        res!(w.save(
                            &wallet_path, "  ", Some(EncoderConfig::<(), ()>::default()),
                        ));
                    }
                } else if res!(msg_cmd.has_only_arg("recover")) {
                    let vals = res!(msg_cmd.get_arg_vals("recover").with_len(1));
                    let name = &vals[0];
                    // Clone the encrypted-secret map out of the wallet
                    // so we can drop the read lock before the
                    // interactive passphrase prompt below.
                    let enc_sec_dat = {
                        let w = lock_read!(self.wallet);
                        match w.enc_secs().get(name) {
                            Some(map_dat) => map_dat.clone(),
                            None => return Ok(Evaluation::Output(
                                fmt!("Secret '{}' not found in wallet.", name)
                            )),
                        }
                    };
                    let enc_sec_dat = &enc_sec_dat;
                    // Derive the encryption key from the wallet passphrase using the kdf
                    // configuration.  Drop the pass as soon as we can.
                    let key = {
                        let pass = res!(UserInput::ask_for_secret(None));
                        let pass = pass.expose_secret();

                        let kdf_name = try_extract_dat!(
                            res!(enc_sec_dat.map_get_type_must(&dat!("kdf_name"), &[&Kind::Str])),
                            Str,
                        );
                        let mut kdf = res!(KeyDerivationScheme::from_str(&kdf_name));
                        let kdf_cfg = try_extract_dat!(
                            res!(enc_sec_dat.map_get_type_must(&dat!("kdf_cfg"), &[&Kind::Str])),
                            Str,
                        );
                        res!(kdf.decode_cfg_from_string(&kdf_cfg));
                        res!(kdf.derive(pass.as_bytes()));
                        res!(kdf.get_hash()).to_vec()
                    };

                    let enc_name = try_extract_dat!(
                        res!(enc_sec_dat.map_get_type_must(&dat!("enc_name"), &[&Kind::Str])),
                        Str,
                    );
                    let mut enc = res!(EncryptionScheme::from_str(&enc_name));
                    enc = res!(enc.set_secret_key(Some(&key)));
                    let enc_sec_base2x = try_extract_dat!(
                        res!(enc_sec_dat.map_get_type_must(&dat!("enc_sec"), &[&Kind::Str])),
                        Str,
                    );
                    let base2x = base2x::HEMATITE64;
                    let enc_sec_byts = res!(base2x.from_str(&enc_sec_base2x));
                    let sec_byts = res!(enc.decrypt(&enc_sec_byts));
                    let mut sec_str = res!(String::from_utf8(sec_byts));
                    res!(UserInput::show_and_clear(
                        Secret::new(fmt!("Press enter to clear: secret is '{}'", sec_str))
                    ));
                    sec_str.zeroize();
                }
            } else {
                return Err(err!("Missing message command."; Invalid, Input, Missing));
            }
        }
        Ok(Evaluation::None)
    }

    pub fn mailpass(
        &mut self,
        _shell_cfg: &ShellConfig,
        cmd:        Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        let msg_cmd = match cmd {
            Some(c) => c,
            None => return Err(err!(
                "mailpass requires arguments."; Invalid, Input, Missing)),
        };
        let address_vals = res!(msg_cmd.get_arg_vals("address").with_len(1));
        let address = try_extract_dat!(&address_vals[0], Str).clone();
        let delivery_vals = res!(msg_cmd.get_arg_vals("delivery-dir").with_len(1));
        let delivery = try_extract_dat!(&delivery_vals[0], Str).clone();
        // Allow the password to be supplied via the STEEL_MAIL_PASS
        // env var so this command works in non-interactive contexts
        // (CI, scripts, deploy automation).
        let pass: secrecy::Secret<String> = match std::env::var("STEEL_MAIL_PASS") {
            Ok(p) => secrecy::Secret::new(p),
            Err(_) => res!(UserInput::ask_for_secret(
                Some("Enter password for mailbox: "),
            )),
        };
        // Use a moderate cost so the prompt feels snappy. 64 MB / 3
        // iterations is the OWASP minimum for Argon2id.
        let mut kdf = res!(KeyDerivationScheme::new_argon2(
            "Argon2id",
            0x13,
            65_536,
            3,
            16,
            32,
        ));
        res!(kdf.derive(pass.expose_secret().as_bytes()));
        let encoded = res!(kdf.encode_to_string());
        // Print the whole file, not just the entry. The entry alone invites
        // the reader to paste it into a bare list, which the parser rejects
        // with "no 'users' list" -- and the only clue that the wrapper exists
        // is a doc comment in another crate.
        println!();
        println!("Add this entry to the \"users\" list in your mail users.jdat.");
        println!("A file with a single user looks like this in full:");
        println!();
        println!("  {{");
        println!("    \"users\": [");
        println!("      {{");
        println!("        \"address\":      \"{}\",", address);
        println!("        \"delivery_dir\": \"{}\",", delivery);
        println!("        \"argon2id\":     \"{}\"", encoded);
        println!("      }}");
        println!("    ]");
        println!("  }}");
        println!();
        Ok(Evaluation::None)
    }

    /// `wallet --migrate`: one-shot migrate a pre-admin-user wallet
    /// into the multi-admin layout.
    ///
    /// Reads the existing wallet file as raw `Dat`, extracts the
    /// legacy `wallet_pass_hashes` (for passphrase verification) and
    /// `app_hashes.default.kdf_cfg` (for deriving the database
    /// encryption key), verifies the current passphrase, derives the
    /// current database encryption key -- which becomes the new
    /// wallet's master key `K` unchanged, so no Ozone re-encryption is
    /// required -- and rewrites the wallet with a single admin entry
    /// named "jason" (override via prompt) wrapping the same `K`. The
    /// old wallet file is preserved as `wallet.jdat.pre-admins` for
    /// rollback.
    pub fn manage_wallet(
        &mut self,
        _shell_cfg: &ShellConfig,
        cmd:        Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        let msg_cmd = match cmd {
            Some(c) => c,
            None => return Err(err!(
                "wallet requires a subcommand argument."; Invalid, Input, Missing)),
        };
        if res!(msg_cmd.has_only_arg("migrate")) {
            return self.migrate_wallet();
        }
        Ok(Evaluation::Output(fmt!(
            "No recognised 'wallet' subcommand argument supplied.")))
    }

    fn migrate_wallet(&mut self) -> Outcome<Evaluation> {
        let wallet_path = Path::new("./").join(app_const::WALLET_NAME);
        if !wallet_path.is_file() {
            return Ok(Evaluation::Output(fmt!(
                "No wallet file to migrate at {:?}.", wallet_path)));
        }
        // Load the wallet file as raw Dat so we can read the legacy
        // layout without depending on the old `Wallet<PH, D>` struct
        // (which no longer exists in the source tree).
        let text = res!(std::fs::read_to_string(&wallet_path));
        let mut dat = res!(Dat::decode_string(&text));
        if dat.kind() != Kind::OrdMap && dat.kind() != Kind::Map {
            return Err(err!(
                "Legacy wallet file at {:?} is not a map (kind={:?}).",
                wallet_path, dat.kind();
                Input, Invalid, Mismatch));
        }
        // If the file already has an "admins" list, it is already the
        // new layout and there is nothing to do.
        if let Ok(_) = dat.map_get_must(&dat!("admins")) {
            return Ok(Evaluation::Output(fmt!(
                "Wallet at {:?} is already in the admin-user layout.",
                wallet_path)));
        }
        // Pull the current passphrase from the caller.
        let pass = res!(UserInput::ask_for_secret(
            Some("Enter the current wallet passphrase: "),
        ));
        let pass_bytes = pass.expose_secret().as_bytes();

        // Verify the passphrase against the legacy `wallet_pass_hashes`
        // ring buffer. We extract just the first (current) entry --
        // older entries are historical and not used for verification.
        let ring = res!(dat.map_remove_must(&dat!("wallet_pass_hashes")));
        let current_hash_dat = res!(extract_legacy_current_passhash(ring));
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
            return Ok(Evaluation::Output(fmt!(
                "Passphrase rejected -- nothing migrated.")));
        }

        // Derive the current database encryption key via the legacy
        // `app_hashes.default` KDF config. That derived key becomes
        // the new wallet's master key, unchanged, so the on-disk
        // Ozone data does not need to be re-encrypted.
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

        // Preserve the existing metadata if present.
        let metadata = match dat.map_remove(&dat!("metadata")) {
            Ok(Some(d)) => try_extract_dat!(d, Map),
            _ => DaticleMap::new(),
        };

        // Prompt for the new admin name, defaulting to the current
        // unix user name or "operator".
        print!("New admin name (default 'operator'): ");
        {
            use std::io::Write;
            res!(std::io::stdout().flush());
        }
        let mut name_in = String::new();
        res!(std::io::stdin().read_line(&mut name_in));
        let admin_name = match name_in.trim() {
            "" => "operator".to_string(),
            s  => s.to_string(),
        };

        // Build the fresh admin entry (wraps `master_key` under the
        // same passphrase the caller just typed), assemble a new
        // Wallet, and save it. The caller keeps using the same
        // passphrase; nothing changes on the Ozone side.
        let admin = res!(oxedyne_fe2o3_crypto::keystore::AdminUser::new(
            admin_name.clone(),
            pass_bytes,
            &master_key,
            oxedyne_fe2o3_crypto::keystore::DEFAULT_WALLET_KDF_NAME,
            vec!["*".to_string()],
            0,
        ));
        let new_wallet = Wallet::new(metadata, vec![admin], DaticleMap::new());

        // Back up the old wallet first.
        let backup_path = Path::new("./").join(fmt!("{}.pre-admins", app_const::WALLET_NAME));
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
        {
            let mut w = lock_write!(self.wallet);
            *w = new_wallet;
        }
        audit::append(&admin_name, "wallet.migrate", "ok",
            &fmt!("backup={:?}", backup_path));
        Ok(Evaluation::Output(fmt!(
            "Migrated wallet to the admin-user layout. New admin '{}' \
            can unlock with the existing passphrase. Old wallet saved \
            as {:?}.",
            admin_name, backup_path,
        )))
    }

    /// `admin --add NAME --scopes ... --expires-in N`,
    /// `admin --remove NAME`,
    /// `admin --list`.
    pub fn manage_admin(
        &mut self,
        _shell_cfg: &ShellConfig,
        cmd:        Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        let msg_cmd = match cmd {
            Some(c) => c,
            None => return Err(err!(
                "admin requires a subcommand argument."; Invalid, Input, Missing)),
        };
        if msg_cmd.has_arg("list") {
            return self.admin_list();
        }
        if msg_cmd.has_arg("passwd") {
            return self.admin_passwd();
        }
        if msg_cmd.has_arg("add") {
            let vals = res!(msg_cmd.get_arg_vals("add").with_len(1));
            let name = try_extract_dat!(&vals[0], Str).clone();
            let scopes: Vec<String> = match msg_cmd.get_arg_vals("scopes") {
                Some(vs) if !vs.is_empty() => {
                    let s = try_extract_dat!(&vs[0], Str).clone();
                    s.split(',').map(|t| t.trim().to_string()).collect()
                },
                _ => Vec::new(),
            };
            let expires_in: u64 = match msg_cmd.get_arg_vals("expires-in") {
                Some(vs) if !vs.is_empty() => match &vs[0] {
                    Dat::U64(n) => *n,
                    Dat::U32(n) => *n as u64,
                    _ => 0u64,
                },
                _ => 0u64,
            };
            let expires_at = if expires_in == 0 {
                0
            } else {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                now.saturating_add(expires_in)
            };
            return self.admin_add(&name, scopes, expires_at);
        }
        if msg_cmd.has_arg("remove") {
            let vals = res!(msg_cmd.get_arg_vals("remove").with_len(1));
            let name = try_extract_dat!(&vals[0], Str).clone();
            return self.admin_remove(&name);
        }
        Ok(Evaluation::Output(fmt!(
            "No recognised 'admin' subcommand argument supplied.")))
    }

    /// `unseal`: unwrap the wallet master key with an admin passphrase.
    ///
    /// Optional. Steel serves perfectly well sealed; this is for the
    /// operator who is already at the shell and would rather have the
    /// databases open before the listeners bind than unseal from the
    /// dashboard afterwards.
    pub fn unseal(
        &mut self,
        _shell_cfg: &ShellConfig,
        _cmd:       Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        if self.db_enc_key.is_some() {
            return Ok(Evaluation::Output(fmt!(
                "Already unsealed by admin '{}'.", self.unlocked_admin_name,
            )));
        }
        res!(self.require_master_key());
        Ok(Evaluation::Output(fmt!(
            "Unsealed by admin '{}'. The databases will open when the server \
            starts.", self.unlocked_admin_name,
        )))
    }

    /// The wallet master key, prompting for a passphrase if the shell is
    /// still sealed.
    ///
    /// Steel defers the wallet unlock: the shell starts without a
    /// passphrase so that `server` can bind and serve with the databases
    /// shut. A command that actually needs the key -- opening a database,
    /// enrolling an admin -- calls this, and the prompt happens then,
    /// rather than as a toll on every invocation.
    ///
    /// `STEEL_ADMIN_PASS` is honoured first, for scripted and test use.
    /// On success the caller's identity and scopes are cached, so a run
    /// of several privileged subcommands prompts once.
    pub fn require_master_key(&mut self) -> Outcome<Vec<u8>> {
        if let Some(key) = &self.db_enc_key {
            return Ok(key.clone());
        }
        let pass = match std::env::var(app_const::ADMIN_PASS_ENV) {
            Ok(s) => Secret::new(s),
            Err(_) => res!(UserInput::ask_for_secret(
                Some("Enter an admin passphrase to unseal: "),
            )),
        };
        let unlocked = {
            let w = lock_read!(self.wallet);
            res!(w.unlock(pass.expose_secret().as_bytes()))
        };
        let key = unlocked.master_key.expose_secret().clone();
        self.db_enc_key = Some(key.clone());
        self.unlocked_admin_name = unlocked.admin_name.clone();
        self.unlocked_admin_scopes = unlocked.admin_scopes.clone();
        info!("Wallet unlocked by admin '{}'.", unlocked.admin_name);
        Ok(key)
    }

    /// `admin --passwd`: rotate the caller's own password in place.
    ///
    /// The caller is whichever admin unlocked the wallet (captured in
    /// `self.unlocked_admin_name`). Scopes and expiry are preserved;
    /// only the wrap is replaced. The cached master key from the unlock
    /// is reused, so the caller does not need to re-type their current
    /// password.
    fn admin_passwd(&mut self) -> Outcome<Evaluation> {
        let master = res!(self.require_master_key());
        let caller_name = self.unlocked_admin_name.clone();
        if caller_name.is_empty() {
            audit::append("(unknown)", "admin.passwd", "err",
                "reason=no_caller_identity");
            return Err(err!(
                "No caller identity is known -- `admin --passwd` can only \
                be invoked inside a running session that has already \
                unlocked the wallet.";
                Input, Invalid, Security));
        }
        let new_pass = res!(UserInput::create_pass(app_const::MAX_CREATE_PASS_ATTEMPTS));
        let wallet_path = Path::new("./").join(app_const::WALLET_NAME);
        {
            let mut w = lock_write!(self.wallet);
            if let Err(e) = w.change_password(
                &caller_name,
                &master,
                new_pass.expose_secret().as_bytes(),
                oxedyne_fe2o3_crypto::keystore::DEFAULT_WALLET_KDF_NAME,
            ) {
                audit::append(&caller_name, "admin.passwd", "err",
                    &fmt!("reason={}", e));
                return Err(e);
            }
            res!(w.save(
                &wallet_path,
                "  ",
                Some(EncoderConfig::<(), ()>::default()),
            ));
        }
        audit::append(&caller_name, "admin.passwd", "ok", "self");
        Ok(Evaluation::Output(fmt!(
            "Password for admin '{}' rotated in place. The new password \
            takes effect at the next Steel start-up; the running session \
            keeps using the master key recovered at its original unlock.",
            caller_name,
        )))
    }

    fn admin_list(&self) -> Outcome<Evaluation> {
        let mut lines = Vec::new();
        lines.push(fmt!(
            "{:<24} {:<12} {}",
            "name", "expires_at", "scopes",
        ));
        let count;
        {
            let w = lock_read!(self.wallet);
            for a in w.admins() {
                let expiry = if a.expires_at == 0 {
                    "never".to_string()
                } else {
                    fmt!("{}", a.expires_at)
                };
                lines.push(fmt!(
                    "{:<24} {:<12} {}",
                    a.name, expiry, a.scopes.join(","),
                ));
            }
            count = w.admins().len();
        }
        audit::append("(anon)", "admin.list", "ok",
            &fmt!("count={}", count));
        Ok(Evaluation::Output(lines.join("\n")))
    }

    /// Return `true` if the admin who unlocked the wallet at start
    /// holds the `"admin"` scope or the `"*"` wildcard.
    fn unlocked_has_admin_scope(&self) -> bool {
        self.unlocked_admin_scopes.iter()
            .any(|s| s == "*" || s == "admin")
    }

    fn admin_add(
        &mut self,
        new_name:   &str,
        new_scopes: Vec<String>,
        expires_at: u64,
    )
        -> Outcome<Evaluation>
    {
        // Unseal first: the caller's identity and scopes are exactly what
        // the unlock establishes, so there is nothing to authorise until
        // it has happened.
        let master = res!(self.require_master_key());
        let caller_name = self.unlocked_admin_name.clone();
        if !self.unlocked_has_admin_scope() {
            audit::append(&caller_name, "admin.add", "err",
                &fmt!("target={} reason=caller_scope", new_name));
            return Err(err!(
                "Admin '{}' does not hold the 'admin' scope; cannot \
                enrol new admins.", caller_name;
                Input, Invalid, Security));
        }
        let new_pass = res!(UserInput::create_pass(app_const::MAX_CREATE_PASS_ATTEMPTS));
        let wallet_path = Path::new("./").join(app_const::WALLET_NAME);
        {
            let mut w = lock_write!(self.wallet);
            if let Err(e) = w.enrol(
                &master,
                new_name,
                new_pass.expose_secret().as_bytes(),
                new_scopes.clone(),
                expires_at,
                oxedyne_fe2o3_crypto::keystore::DEFAULT_WALLET_KDF_NAME,
            ) {
                audit::append(&caller_name, "admin.add", "err",
                    &fmt!("target={} reason={}", new_name, e));
                return Err(e);
            }
            res!(w.save(
                &wallet_path,
                "  ",
                Some(EncoderConfig::<(), ()>::default()),
            ));
        }
        audit::append(&caller_name, "admin.add", "ok",
            &fmt!("target={} scopes={} expires_at={}",
                new_name, new_scopes.join(","), expires_at));
        Ok(Evaluation::Output(fmt!(
            "Added admin '{}'.", new_name,
        )))
    }

    fn admin_remove(&mut self, target_name: &str) -> Outcome<Evaluation> {
        // As in `admin_add`: the unlock is what establishes who the caller
        // is, so it has to precede the scope check.
        res!(self.require_master_key());
        let caller_name = self.unlocked_admin_name.clone();
        if !self.unlocked_has_admin_scope() {
            audit::append(&caller_name, "admin.remove", "err",
                &fmt!("target={} reason=caller_scope", target_name));
            return Err(err!(
                "Admin '{}' does not hold the 'admin' scope; cannot \
                remove admin entries.", caller_name;
                Input, Invalid, Security));
        }
        let wallet_path = Path::new("./").join(app_const::WALLET_NAME);
        {
            let mut w = lock_write!(self.wallet);
            if let Err(e) = w.remove_by_name(target_name) {
                audit::append(&caller_name, "admin.remove", "err",
                    &fmt!("target={} reason={}", target_name, e));
                return Err(e);
            }
            res!(w.save(
                &wallet_path,
                "  ",
                Some(EncoderConfig::<(), ()>::default()),
            ));
        }
        audit::append(&caller_name, "admin.remove", "ok",
            &fmt!("target={}", target_name));
        Ok(Evaluation::Output(fmt!(
            "Removed admin '{}'.", target_name,
        )))
    }

    pub fn manage_certificates(
        &mut self,
        _shell_cfg: &ShellConfig,
        cmd:        Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        if let Some(msg_cmd) = cmd {
            if msg_cmd.has_args() {
                if res!(msg_cmd.has_only_arg("create-dev")) {
                    info!("Generating self-signed development certificates...");
                    res!(Certificate::new_dev(
                        &ServerConfig::default(),
                        &Path::new(&self.app_cfg.app_root).normalise().absolute(),
                    ));
                    return Ok(Evaluation::Output(fmt!(
                        "Self-signed development certificates generated in {}/tls/{}",
                        self.app_cfg.app_root,
                        srv_const::TLS_DIR_DEV,
                    )));
                }
            } else {
                let avail_args = if let Some(cmd) = msg_cmd.syntax.get_cmd(&*msg_cmd.name) {
                    cmd.collect_short_arg_names()
                        .iter()
                        .map(|s| fmt!("-{}", s))
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    fmt!("<no args>")
                };
                return Ok(Evaluation::Error(fmt!(
                    "Must use one of '{}' for command '{}'.  Type 'help' for more info.",
                    avail_args, msg_cmd.name,
                )));
            }
        }
        Ok(Evaluation::None)
    }

    /// `acme` shell command: inspect the configured ACME state.
    ///
    /// Currently supports:
    ///   - `acme -s` / `acme --status` — print the configured vhost hostnames
    ///     and the ACME directory URL that will be used on next start-up.
    ///   - `acme -r` / `acme --renew` — schedule a forced renewal by clearing
    ///     the ACME cache directory. Steel re-issues on next start-up.
    ///
    /// Live inspection and renewal of a running ACME state is a follow-up;
    /// for now the shell only inspects and resets cached state.
    pub fn manage_acme(
        &mut self,
        _shell_cfg: &ShellConfig,
        cmd:        Option<&MsgCmd>,
    )
        -> Outcome<Evaluation>
    {
        let server_cfg = res!(ServerConfig::from_datmap(self.app_cfg.server_cfg.clone()));
        let acme_cfg = res!(server_cfg.get_acme());
        let vhosts = res!(server_cfg.get_vhosts());

        if let Some(msg_cmd) = cmd {
            if msg_cmd.has_args() {
                if res!(msg_cmd.has_only_arg("status")) {
                    let mut lines = Vec::new();
                    lines.push(fmt!("ACME enabled:   {}", acme_cfg.enabled));
                    lines.push(fmt!("Directory URL:  {}", acme_cfg.directory_url));
                    lines.push(fmt!("Contact email:  {}",
                        if acme_cfg.contact_email.is_empty() {
                            fmt!("(not set)")
                        } else {
                            acme_cfg.contact_email.clone()
                        }));
                    lines.push(fmt!("Cache dir:      {}", acme_cfg.cache_dir_rel));
                    lines.push(fmt!("Vhost hostnames:"));
                    for vh in &vhosts {
                        lines.push(fmt!("  - {}", vh.hostnames.join(", ")));
                    }
                    return Ok(Evaluation::Output(lines.join("\n")));
                } else if res!(msg_cmd.has_only_arg("renew")) {
                    let root = Path::new(&self.app_cfg.app_root).normalise().absolute();
                    let cache_dir = res!(acme_cfg.get_cache_dir(&root));
                    info!("Clearing ACME cache at {:?} to force renewal on next start-up.",
                        cache_dir);
                    match std::fs::remove_dir_all(&cache_dir) {
                        Ok(()) => (),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(e) => return Err(err!(e,
                            "Failed to clear ACME cache at {:?}.", cache_dir;
                            IO, File)),
                    }
                    res!(std::fs::create_dir_all(&cache_dir));
                    return Ok(Evaluation::Output(fmt!(
                        "ACME cache cleared at {:?}. Restart the server to re-issue.",
                        cache_dir,
                    )));
                }
            } else {
                return Ok(Evaluation::Error(fmt!(
                    "Use 'acme -s' for status or 'acme -r' to schedule renewal.",
                )));
            }
        }
        Ok(Evaluation::None)
    }
}

/// Extract the "current" passhash from a legacy `wallet_pass_hashes`
/// ring buffer `Dat`. The ring buffer is a 2-tuple of `(list, index)`
/// where `list` holds N optional timestamped entries and `index`
/// points at the active one. Used by the one-shot `wallet migrate`
/// flow to recover the pre-admin-user passphrase verification hash.
fn extract_legacy_current_passhash(ring: Dat) -> Outcome<Dat> {
    // Tuples round-trip as `Dat::Tup2` values that we destructure
    // with `try_extract_tup2dat`. The first element is the slot
    // list, the second is the index.
    let mut parts = oxedyne_fe2o3_jdat::try_extract_tup2dat!(ring);
    let index: u64 = match parts[1].extract() {
        Dat::U64(n) => n,
        other => return Err(err!(
            "Legacy ring buffer index must be u64 (got {:?}).", other.kind();
            Invalid, Input)),
    };
    let list = oxedyne_fe2o3_jdat::try_extract_dat!(parts[0].extract(), Vek);
    let slot = list.into_iter().nth(index as usize).ok_or_else(|| err!(
        "Legacy ring buffer index {} out of range.", index;
        Input, Invalid, Mismatch))?;
    // Each slot is an `Opt<Tup2(data, timestamp)>`. The caller wants
    // the `data` daticle, which is the kdf map.
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
    Ok(slot_parts[0].extract())
}

