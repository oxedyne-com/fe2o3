use crate::constant;

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_crypto::{
    enc::EncryptionScheme,
    keys::Wallet,
};
use oxedize_fe2o3_data::{
    ring::RingBuffer,
    time::Timestamped,
};
use oxedize_fe2o3_hash::kdf::KeyDerivationScheme;
use oxedize_fe2o3_iop_hash::kdf::KeyDeriver;

use std::io::{
    Read,
    Write,
};

use secrecy::{
    ExposeSecret,
    Secret,
};
use zeroize::Zeroize;


pub struct UserInput;

impl UserInput {

    pub fn ask(prompt: &str) -> Outcome<String> {
        print!("{}", prompt);
        res!(std::io::stdout().flush());
        let mut choice = String::new();
        res!(std::io::stdin().read_line(&mut choice));
        Ok(choice)
    }

    pub fn show_and_clear(sec: Secret<String>) -> Outcome<()> {
        let sec = sec.expose_secret();
        print!("{}", sec);
        res!(std::io::stdout().flush());
        let mut _input = [0; 1];
        res!(std::io::stdin().read(&mut _input)); // Wait for any key press.
        print!("\r{}\r", " ".repeat(sec.len()));
        res!(std::io::stdout().flush());
        Ok(())
    }

    pub fn create_pass(
        _rbuf_opt: Option<Wallet<{constant::NUM_PREV_PASSHASHES_TO_RETAIN}>>,
    )
        -> Outcome<Secret<String>>
    {
        let mut count: usize = 0;
        let pass = loop {
            let mut pass1 = res!(Self::ask_for_secret(Some("Enter a passphrase: ")));
            let mut pass2 = res!(Self::ask_for_secret(Some("Re-enter the passphrase: ")));
            if pass1.expose_secret() != pass2.expose_secret() {
                println!(" The passphrases do not match, try again.");
            } else {
                break pass1;
            }
            count += 1;
            if count > constant::MAX_CREATE_PASS_ATTEMPTS {
                return Err(err!(
                    "Number of failed attempts to repeat passphrase exceeded limit of {}.",
                    constant::MAX_CREATE_PASS_ATTEMPTS;
                Input, Invalid, Excessive, String));
            }
        };
        Ok(pass)
    }

    pub fn ask_for_secret(prompt_opt: Option<&str>) -> Outcome<Secret<String>> {
        if let Some(prompt) = prompt_opt {
            print!("{}", prompt);
        } else {
            print!("Enter app wallet passphrase: ");
        }
        res!(std::io::stdout().flush());
        let pass = res!(rpassword::read_password());
        Ok(Secret::new(pass))
    }

    pub fn derive_key(pass: Secret<String>) -> Outcome<(KeyDerivationScheme, Vec<u8>)> {
        let pass = pass.expose_secret().as_bytes();
        let mut kdf = res!(KeyDerivationScheme::new_argon2(
            "Argon2id",
            0x13,
            constant::KDF_MEM_COST_KB,
            constant::KDF_TIME_COST_PASSES,
            constant::KDF_SALT_LEN,
            constant::KDF_HASH_LEN,
        ));
        res!(kdf.derive(pass));
        let key = res!(kdf.get_hash()).to_vec();
        Ok((kdf, key))
    }
}
