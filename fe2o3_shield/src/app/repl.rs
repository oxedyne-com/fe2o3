use crate::{
    app::{
        cfg::AppConfig,
        constant as app_const,
        server,
        tui::AppStatus,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    log::{
        bot::FileConfig,
        console::{
            switch_to_logger_console,
            StdoutLoggerConsole,
        },
    },
    path::NormalPath,
};
use oxedize_fe2o3_crypto::{
    enc::EncryptionScheme,
    keys::Wallet,
};
use oxedize_fe2o3_hash::{
    csum::ChecksumScheme,
    hash::HashScheme,
    kdf::KeyDerivationScheme,
};
use oxedize_fe2o3_iop_crypto::{
    keys::KeyManager,
    enc::Encrypter,
};
use oxedize_fe2o3_iop_hash::kdf::KeyDeriver;
use oxedize_fe2o3_jdat::{
    prelude::*,
    file::JdatFile,
    string::enc::EncoderConfig,
};
use oxedize_fe2o3_net::id;
use oxedize_fe2o3_o3db_sync::O3db;
use oxedize_fe2o3_syntax::{
    core::SyntaxRef,
    help::Help,
    msg::{
        Msg,
        MsgCmd,
    },
    opt::OptionRefVec,
};
use oxedize_fe2o3_text::base2x;
use oxedize_fe2o3_tui::lib_tui::{
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
use oxedize_fe2o3_namex::InNamex;

use std::{
    collections::BTreeMap,
    path::{
        Path,
        PathBuf,
    },
};

use secrecy::{
    ExposeSecret,
    Secret,
};
use zeroize::Zeroize;


#[derive(Clone)]
pub struct AppShellContext {
    pub stat:       AppStatus,
    pub app_cfg:    AppConfig,
    pub syntax:     SyntaxRef,
    pub ws:         BTreeMap<Dat, Dat>,
    pub db:         O3db<
                        { id::UID_LEN },
                        id::Uid,
                        EncryptionScheme,
                        HashScheme,
                        HashScheme,
                        ChecksumScheme,
                    >,
    pub wallet:     Wallet<{ app_const::NUM_PREV_PASSHASHES_TO_RETAIN }, Dat>,
    //server: Server,
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
                "server"    => {
                    let root_path = Path::new(&self.app_cfg.app_root)
                        .normalise() // Now a NormPathBuf.
                        .absolute();
                    // ┌───────────────────────┐
                    // │ Reconfigure logging.  │
                    // └───────────────────────┘
                    info!("Reconfiguring log to stdout and file...");
                    res!(switch_to_logger_console::<StdoutLoggerConsole<_>>());
                    let mut log_cfg = log_get_config!();
                    log_cfg.file = Some(FileConfig::new(
                        PathBuf::from(&root_path).join("www").join("logs"),
                        self.app_cfg.app_name.clone(),
                        "log".to_string(),
                        0,
                        Some(1_048_576), // Activate multiple log file archiving using this max size.
                    ));
                    (log_cfg.level, _) = res!(self.app_cfg.server_log_level());
                    log_set_config!(log_cfg);
                    info!("Server now logging at {:?}", log_get_file_path!());
                    info!("┌───────────────────────┐");
                    info!("│ New server session.   │");
                    info!("└───────────────────────┘");

                    let rt = res!(tokio::runtime::Runtime::new());
                    let (eval, _cmd_chan) = res!(rt.block_on(
                        server::start_server(
                            &self.app_cfg,
                            &self.stat,
                            self.db.clone(),
                            Some(cmd),
                            None,
                        )
                    ));
                    evals.push(eval);
                }
                "shell"     => evals.push(res!(self.start_shell(&shell_cfg, Some(cmd)))),
                // Filesystem
                "cd"        => evals.push(res!(cmds::change_directory(cmd))),
                "ls"        => evals.push(res!(cmds::list_directory_contents(cmd))),
                "pwd"       => evals.push(res!(cmds::print_working_directory())),
                // Wallet
                "secrets"   => evals.push(res!(self.secrets(&shell_cfg, Some(cmd)))),
                _ => (), // Not implemented yet.
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
                    if self.wallet.enc_secs().get(name).is_some() {
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
                    if let Some(enc_sec_map) = self.wallet.enc_secs_mut().get_mut(name) {
                        *enc_sec_map = dat!(map);
                    } else {
                        self.wallet.enc_secs_mut().insert(name.clone(), dat!(map));
                    }
                    let wallet_path = Path::new("./").join(app_const::WALLET_NAME);
                    res!(self.wallet.save(
                        &wallet_path, "  ", Some(EncoderConfig::<(), ()>::default()),
                    ));
                } else if res!(msg_cmd.has_only_arg("recover")) {
                    let vals = res!(msg_cmd.get_arg_vals("recover").with_len(1));
                    let name = &vals[0];
                    let enc_sec_dat = match self.wallet.enc_secs().get(name) {
                        Some(map_dat) => map_dat,
                        None => return Ok(Evaluation::Output(
                            fmt!("Secret '{}' not found in wallet.", name)
                        )),
                    };
                    // Derive the encryption key from the wallet passphrase using the kdf
                    // configuration.  Drop the pass as soon as we can.
                    let key = {
                        let pass = res!(UserInput::ask_for_secret(None));
                        let pass = pass.expose_secret();

                        let kdf_name = try_extract_dat!(
                            res!(enc_sec_dat.map_get_type_must(&dat!("kdf_name"), &[&Kind::Str])),
                            Str,
                        );
                        let mut kdf = res!(KeyDerivationScheme::from_str(kdf_name));
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
                    let mut enc = res!(EncryptionScheme::from_str(enc_name));
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
}
