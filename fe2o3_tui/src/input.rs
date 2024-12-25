use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_hash::kdf::KeyDerivationScheme;
use oxedize_fe2o3_iop_hash::kdf::KeyDeriver;

use std::io::{
    self,
    Write,
};

use crossterm::{
    cursor,
    event::{
        self,
        Event,
        KeyCode,
    },
    execute,
    terminal::{
        self,
        ClearType,
    },
};
use secrecy::{
    ExposeSecret,
    Secret,
};


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
        res!(terminal::enable_raw_mode());
        // Wait for any key press event.
        if let Event::Key(_) = res!(event::read()) {
            res!(terminal::disable_raw_mode());
            res!(execute!(
                io::stdout(),
                cursor::MoveToColumn(0), // Move to the start of the line.
                terminal::Clear(ClearType::CurrentLine), // Clear the current line.
            ));
            res!(std::io::stdout().flush());
        }
        Ok(())
    }

    pub fn create_pass(
        max_attempts: usize,
    )
        -> Outcome<Secret<String>>
    {
        let mut count: usize = 0;
        let pass = loop {
            let pass1 = res!(Self::ask_for_secret(Some("Enter a passphrase: ")));
            let pass2 = res!(Self::ask_for_secret(Some("Re-enter the passphrase: ")));
            if pass1.expose_secret() != pass2.expose_secret() {
                println!(" The passphrases do not match, try again.");
            } else {
                break pass1;
            }
            count += 1;
            if count > max_attempts {
                return Err(err!(errmsg!(
                    "Number of failed attempts to repeat passphrase exceeded limit of {}.",
                    max_attempts,
                ), Input, Invalid, Excessive, String));
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
        res!(terminal::enable_raw_mode());

        let mut pass = String::new();
        loop {
            if let Event::Key(key_event) = res!(event::read()) {
                match key_event.code {
                    KeyCode::Char(c) => {
                        pass.push(c);
                    }
                    KeyCode::Enter => {
                        break;
                    }
                    KeyCode::Backspace => {
                        pass.pop();
                    }
                    _ => {}
                }
            }
        }

        // Disable raw mode and print a newline.
        res!(terminal::disable_raw_mode());
        println!();

        Ok(Secret::new(pass))
    }

    pub fn derive_key(
        kdf:    &mut KeyDerivationScheme,
        pass:   Secret<String>,
    )
        -> Outcome<Vec<u8>>
    {
        let pass = pass.expose_secret().as_bytes();
        res!(kdf.derive(pass));
        Ok(res!(kdf.get_hash()).to_vec())
    }
}
