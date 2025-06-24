use crate::{
    arg::Arg,
    core::Syntax,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    map::Recursive,
};
use oxedyne_fe2o3_stds::chars::Term;


#[derive(Clone, Debug)]
pub struct HelpDisplayConfig {
    pub cmds_only:  bool,
    pub colour:     bool,
    pub col:        [usize; 7], 
    pub col_str:    [String; 7], 
    pub cmd_effect: String,
    pub val_effect: String,
    pub arg_effect: String,
}

impl Default for HelpDisplayConfig {
    fn default() -> Self {
        const EMPTY_STRING: String = String::new();
        let col = [5, 7, 9, 20, 22, 24, 80];
        let mut col_str = [EMPTY_STRING; 7];
        for (i, c) in col.iter().enumerate() {
            col_str[i] = " ".repeat(*c);
        }
        Self {
            cmds_only:  false,
            colour:     true,
            col,
            col_str,
            cmd_effect: Term::SET_BRIGHT_FORE_YELLOW.to_string() + Term::BOLD,
            val_effect: Term::SET_BRIGHT_FORE_MAGENTA.to_string() + Term::BOLD,
            arg_effect: Term::SET_BRIGHT_FORE_GREEN.to_string() + Term::BOLD,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Help {
    pub cfg: HelpDisplayConfig,
}

impl Help {

    pub fn new(cfg: HelpDisplayConfig) -> Self {
        Self {
            cfg,
        }
    }

    // TODO
    // - add a column before 0 for categories
    // - organise by categories
    // - allow separate effects for descriptions
    ///```ignore
    /// |      0   1   2                3   4   5                       6
    /// |      :   :   :                :   :   :                       :
    /// |MSG:  :   :   :                :   :   :                       :
    /// |      :[a_val]:                :   :val description.           :
    /// |      :-a_arg :                :   :arg description.           :
    /// |      :   :[a_val]             :   :   :arg val description.   :
    /// |      :   :   :                :   :   :                       :
    /// |CMD:  :   :   :                :   :   :                       :
    /// |      :a_cmd  :                :cmd description.               :
    /// |      :   :[a_val]             :   :val description.           :
    /// |      :   :-a_arg              :   :arg description.           :
    /// |      :   :   :[a_val]         :   :   :arg val description.   :
    ///
    ///
    ///```
    pub fn to_lines(
        &self,
        syntax: &Syntax,
    )
        -> Outcome<Vec<String>>
    {
        let mut lines = Vec::new();
        let cmds_only = self.cfg.cmds_only
            || (syntax.config().vals.len() == 0 && syntax.args.len() == 0);
        let cmd_effect = if self.cfg.colour { &self.cfg.cmd_effect }  else { "" };
        let arg_effect = if self.cfg.colour { &self.cfg.arg_effect }  else { "" };
        let val_effect = if self.cfg.colour { &self.cfg.val_effect }  else { "" };
        let reset = if self.cfg.colour { &Term::RESET }  else { "" };
        if let Some(about) = &syntax.config().about {
            let line = fmt!(
                " {}{}{} ",
                if self.cfg.colour {
                    Term::ITALIC.to_owned() + Term::BOLD + Term::SET_BRIGHT_FORE_RED
                } else {
                    Term::ITALIC.to_owned() + Term::BOLD
                },
                about,
                reset,
            );
            lines.push(line);    
        }
        lines.push(String::new());
        lines.push(String::from("USAGE:"));
        if cmds_only {
            lines.push(fmt!(
                "{}{}CMD{} {}[cmd_vals]{} {}cmd_arg{} {}[cmd_arg_vals]{} ..",
                self.cfg.col_str[0],
                cmd_effect, reset,
                val_effect, reset,
                arg_effect, reset,
                val_effect, reset,
            ));
        } else {
            lines.push(fmt!(
                "{}{}[msg_vals]{} {}msg_arg{} {}[msg_arg_vals]{} .. {}CMD{} {}[cmd_vals]{} \
                {}cmd_arg{} {}[cmd_arg_vals]{} ..",
                self.cfg.col_str[0],
                val_effect, reset,
                arg_effect, reset,
                val_effect, reset,
                cmd_effect, reset,
                val_effect, reset,
                arg_effect, reset,
                val_effect, reset,
            ));
        }
        if !cmds_only {
            lines.push(String::from("MSG:"));
            // Message values.
            for (kind, help_txt) in &syntax.config().vals {
                let mut line = String::new();
                let mut len = 0;
                line.push_str(&fmt!(
                    "{}{}[{}]",
                    self.cfg.col_str[1],
                    val_effect,
                    kind,
                ));
                len += line.len() - val_effect.len();
                line.push_str(&fmt!(
                    "{}{}{}",
                    " ".repeat(try_range!(self.cfg.col[4] - len, 0, self.cfg.col[6])),
                    Self::normalise(help_txt),
                    reset,
                ));
                lines.push(line);
            }
            // Message arguments.
            for (_k, v) in &syntax.args {
                if let Recursive::Val(arg) = v {
                    lines.append(&mut res!(self.arg_lines(arg, true)));
                }
            }
        }
        lines.push(String::from("CMD:"));
        for (_k, v) in &syntax.cmds {
            if let Recursive::Val(cmd) = v {
                let mut line = String::new();
                let mut len = 0;
                // Command name.
                line.push_str(&fmt!(
                    "{}{}{}{}",
                    self.cfg.col_str[0],
                    cmd_effect,
                    cmd.config().name,
                    reset,
                ));
                len += line.len() - cmd_effect.len() - reset.len();
                // Command description.
                if let Some(help_txt) = &cmd.config().help {
                    line.push_str(&fmt!(
                        "{}{}{}{}",
                        " ".repeat(try_range!(self.cfg.col[3] - len, 0, self.cfg.col[6])),
                        cmd_effect,
                        Self::normalise(help_txt),
                        reset,
                    ));
                }
                lines.push(line);
                // Command values.
                for (kind, help_txt) in &cmd.config().vals {
                    let mut line = String::new();
                    let mut len = 0;
                    line.push_str(&fmt!(
                        "{}{}[{}]",
                        self.cfg.col_str[1],
                        val_effect,
                        kind,
                    ));
                    len += line.len() - val_effect.len();
                    line.push_str(&fmt!(
                        "{}{}{}",
                        " ".repeat(try_range!(self.cfg.col[4] - len, 0, self.cfg.col[6])),
                        Self::normalise(help_txt),
                        reset,
                    ));
                    lines.push(line);
                }
                // Command arguments.
                for (_k, v) in &cmd.args {
                    if let Recursive::Val(arg) = v {
                        lines.append(&mut res!(self.arg_lines(arg, false)));
                    }
                }
            }
        }
        Ok(lines)
    }

    pub fn arg_lines(
        &self,
        arg:        &Arg,
        for_msg:    bool,
    )
        -> Outcome<Vec<String>>
    {
        let first_col = if for_msg { 0 } else { 1 };
        let mut lines = Vec::new();
        let mut line = String::new();
        let arg_effect = if self.cfg.colour { &self.cfg.arg_effect }  else { "" };
        let val_effect = if self.cfg.colour { &self.cfg.val_effect }  else { "" };
        let reset = if self.cfg.colour { &Term::RESET }  else { "" };
        let mut len = 0;
        // Short argument switch.
        line.push_str(&fmt!(
            "{}{}-{}",
            self.cfg.col_str[first_col],
            arg_effect,
            arg.config().hyph1,
        ));
        // Long argument switch.
        if let Some(hyph2_txt) = &arg.config().hyph2 {
            line.push_str(&fmt!(
                " --{}",
                hyph2_txt,
            ));
        }
        len += line.len() - arg_effect.len();
        // Argument description.
        if let Some(help_txt) = &arg.config().help {
            line.push_str(&fmt!(
                "{}{}{}",
                " ".repeat(try_range!(self.cfg.col[first_col + 3] - len, 0, self.cfg.col[6])),
                Self::normalise(help_txt),
                if self.cfg.colour { &reset }                else { "" },
            ));
        }
        lines.push(line);
        if arg.config().vals.len() > 0 {
            for (kind, help_txt) in &arg.config().vals {
                let mut line = String::new();
                let mut len = 0;
                // Argument values.
                line.push_str(&fmt!(
                    "{}{}[{}]",
                    self.cfg.col_str[first_col + 1],
                    val_effect,
                    kind,
                ));
                len += line.len() - val_effect.len();
                line.push_str(&fmt!(
                    "{}{}{}",
                    " ".repeat(try_range!(self.cfg.col[first_col + 4] - len, 0, self.cfg.col[6])),
                    Self::normalise(help_txt),
                    reset,
                ));
                //len += line.len() - reset.len();
                // TODO Argument value description.
                {


                }
                lines.push(line);
            }
        }
        Ok(lines)
    }

    pub fn normalise(s: &str) -> String {
        if s.ends_with('.') {
            s.to_string()
        } else {
            fmt!("{}.", s)
        }
    }
}

