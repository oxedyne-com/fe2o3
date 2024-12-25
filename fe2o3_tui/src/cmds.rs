use crate::{
    repl::{
        Evaluation,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_jdat::prelude::*;
use oxedize_fe2o3_units::{
    si::SI,
    system::Units,
};
use oxedize_fe2o3_stds::chars::Term;
use oxedize_fe2o3_syntax::{
    msg::{
        MsgCmd,
    },
};

use std::{
    fs,
};

// ┌───────────────────────┐
// │ CONTROL               │
// └───────────────────────┘

pub fn exit_shell(exit_msg: &str) -> Outcome<Evaluation> {
    println!("{}", exit_msg);
    return Ok(Evaluation::Exit);
}

// ┌───────────────────────┐
// │ FILE SYSTEM           │
// └───────────────────────┘

pub fn change_directory(cmd: &MsgCmd) -> Outcome<Evaluation> {
    if cmd.vals.len() > 0 {
        if let Dat::Str(s) = &cmd.vals[0] { 
            match std::env::set_current_dir(s) {
                Err(e) => println!(
                    "{}{}{}{}",
                    Term::SET_BRIGHT_FORE_RED,
                    Term::BOLD,
                    e,
                    Term::RESET,
                ),
                _ => {},
            }
        }
    }
    Ok(Evaluation::None)
}

pub fn list_directory_contents(cmd: &MsgCmd) -> Outcome<Evaluation> {
    let mut lines = Vec::new();
    let mut max_raw_bytes_width = 0;
    for entry in res!(fs::read_dir(".")) {
        let mut line = Vec::new();
        let entry = res!(entry);
        let mdata = res!(entry.metadata());
        let b = res!(Units::<SI>::bytes(mdata.len() as f64, 4)).humanise();
        // 0 size - raw
        let bytes = mdata.len().to_string();
        if bytes.len() > max_raw_bytes_width {
            max_raw_bytes_width = bytes.len() + b.symbol().len() + 1;
        }
        line.push(format!("{} {}", bytes, b.symbol()));
        // 1 size - humanised
        line.push(format!("{:5.prec$} {:>2}{:<1}",
            b.val(),
            b.prefix(),
            b.symbol(),
            prec = 1,
        ));
        // 2 type
        line.push(format!("{}{}",
            if mdata.is_dir() {"d"} else {" "},
            if mdata.permissions().readonly() {"r"} else {" "},
        ));
        // 3 name
        line.push(format!("{}", 
            entry.path().file_name().unwrap().to_str().unwrap(),
        ));
        lines.push(line);
    }
    
    match cmd.get_arg_vals("sort") {
        Some(vals) => {
            if vals.len() > 0 {
                if let Dat::Str(s) = &vals[0] { 
                    match &s[..] {
                        "size" => {
                            lines.sort_by(|a, b| a[0].cmp(&b[0]));
                        },
                        "type" => {
                            lines.sort_by(|a, b| a[2].cmp(&b[2]));
                        },
                        "name" => {
                            lines.sort_by(|a, b| a[3].cmp(&b[3]));
                        },
                        _ => unimplemented!(),
                    }
                    if cmd.has_arg(&fmt!("reverse")) {
                        lines.reverse();
                    }
                }
            }
        },
        None => {},
    }
    for line in lines {
        if cmd.has_arg(&fmt!("bytes")) {
            println!(
                "{:>width$} {} {}",
                line[0], line[2], line[3],
                width = max_raw_bytes_width,
            );
        } else {
            println!("{} {} {}", line[1], line[2], line[3]);
        }
    }
    Ok(Evaluation::None)
}

pub fn print_working_directory() -> Outcome<Evaluation> {
    let path = res!(std::env::current_dir());
    let output = if let Some(s) = path.to_str() {
        fmt!("{}", s)
    } else {
        fmt!("{:?}", path)
    };
    return Ok(Evaluation::Output(output));
}

// ┌───────────────────────┐
// │ WORKSPACE             │
// └───────────────────────┘
