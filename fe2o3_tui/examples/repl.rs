pub mod syntax;

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_jdat::{
    prelude::*,
    version::SemVer,
};
use oxedize_fe2o3_syntax::{
    core::SyntaxRef,
    msg::Msg,
};
use oxedize_fe2o3_ui::{
    cmds,
    repl::{
        Evaluation,
        Shell,
        ShellConfig,
        ShellContext,
        Splitters,
    },
};

struct AppContext {
    ws: BTreeMap<Dat, Dat>,
    //db: Database,
    //server: Server,
}

impl ShellContext for AppContext {
    fn eval(
        &mut self,
        input:      &String,
        cfg:        &ShellConfig,
        splitters:  &Splitters,
        syntax:     SyntaxRef,
    )
        -> Outcome<Evaluation>
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
                        return Err(err!(errmsg!(
                            "The left hand side of the assignment is a {:?} but must be a Kind::Str.",
                            lhs.kind(),
                        ), ErrTag::Input, ErrTag::Mismatch));
                    }
                    self.ws.insert(lhs, rhs);
                    continue;
                },
                _ => return Err(err!(errmsg!(
                    "Only single assignment such as a = b is permitted.",
                ), ErrTag::Input, ErrTag::Mismatch)),
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
                return Ok(Evaluation::Output(input.clone()));
            }
            let msgrx = Msg::new(syntax.clone());
            let msgrx = res!(msgrx.rx_text_iter(parts));  
            for (cmd_key, cmd) in &msgrx.cmds {
                match cmd_key.as_str() {
                    "help" => {
                        for line in syntax.help(true) {
                            println!("{}", line);
                        }
                    },
                    "cd" => return cmds::change_directory(cmd),
                    "ls" => return cmds::list_directory_contents(cmd),
                    "pwd" => return cmds::print_working_directory(),
                    "exit" => return cmds::exit_shell(&cfg.exit_msg),
                    _ => todo!(),
                }
            }
        }
        Ok(Evaluation::None)
    }
}

fn main() -> Outcome<()> {
    let context = AppContext { ws: BTreeMap::new(), };
    let syntax = res!(syntax::new("Test", SemVer::new(0, 1, 0), "An example"));
    let mut shell = res!(Shell::new(
        ShellConfig::default(),
        context,
        syntax,
    ));
    shell.start()
}
