use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_stds::chars::Term;
use oxedize_fe2o3_text::{
    split::StringSplitter,
};

use std::{
    fs::OpenOptions,
    io::{
        stdout,
        BufRead,
        BufReader,
        Write,
    },
    path::Path,
};

use crossterm::{
    cursor,
    event::{
        read,
        Event,
        KeyCode,
        KeyModifiers,
    },
    execute,
    terminal::{
        disable_raw_mode,
        enable_raw_mode,
        Clear,
        ClearType,
    },
};


pub trait ShellContext {
    fn eval(
        &mut self,
        input:      &String,
        cfg:        &ShellConfig,
        splitters:  &Splitters,
    )
        -> Outcome<Vec<Evaluation>>;
}

#[derive(Clone)]
pub struct ShellConfig {
    pub greeting_msg:   String,
    pub norm_prompt:    String,
    pub alert_prompt:   String,
    pub history_name:   String,
    pub exit_msg:       String,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            greeting_msg:   fmt!("Welcome!"),
            norm_prompt:    fmt!(">"),
            alert_prompt:   fmt!("!"),
            history_name:   fmt!("history.txt"),
            exit_msg:       fmt!("Goodbye!"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Splitters {
    pub command:    StringSplitter,
    pub assignment: StringSplitter,
    pub word:       StringSplitter,
}

impl Default for Splitters {
    fn default() -> Self {
        Self {
            command:    StringSplitter::new().add_separators(Box::new([';'])),
            assignment: StringSplitter::new().add_separators(Box::new(['='])),
            word:       StringSplitter::default(),
        }
    }
}

pub enum Evaluation {
    Exit,
    Error(String),
    None,
    Output(String),
}

pub struct Shell<C: ShellContext> {
    cfg:        ShellConfig,
    context:    C,
    splitters:  Splitters,
    hist:       Vec<String>,
    hist_ind:   usize,
}

impl<C: ShellContext> Shell<C> {

    pub fn new(
        cfg:        ShellConfig,
        context:    C,
    )
        -> Outcome<Self>
    {
        let hist_path = "history.txt";

        let file = res!(OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(hist_path)
        );

        let reader = BufReader::new(file);
        let hist: Vec<String> = reader.lines().filter_map(Result::ok).collect();
        let hist_ind = hist.len();

        Ok(Shell {
            cfg,
            context,
            splitters: Splitters::default(),
            hist,
            hist_ind,
        })
    }

    fn read_line(&mut self) -> Outcome<Option<String>> {
        res!(enable_raw_mode());
        let mut input = String::new();
        let mut cursor_pos = 0;
    
        loop {
            if let Event::Key(key_event) = res!(read()) {
                match key_event.code {
                    KeyCode::Char(c) if key_event.modifiers.contains(KeyModifiers::CONTROL)
                        && (c == 'c' || c == 'd') =>
                    {
                        res!(disable_raw_mode());
                        println!("\n\r{}", self.cfg.exit_msg);
                        return Ok(None);
                    }
                    KeyCode::Char(c) => {
                        input.insert(cursor_pos, c);
                        cursor_pos += 1;
                        print!("{}", c);
                        if cursor_pos < input.len() {
                            // Print the rest of the input after the insertion point
                            print!("{}", &input[cursor_pos..]);
                            // Move the cursor back to the right position after reprinting the rest of the input
                            res!(execute!(stdout(), cursor::MoveLeft((input.len() - cursor_pos) as u16)));
                        }
                        res!(stdout().flush());
                    }
                    KeyCode::Enter => {
                        println!("\r"); // println! behaves differently in crossterm raw mode.
                        res!(disable_raw_mode());
                        return Ok(Some(input));
                    }
                    KeyCode::Left => {
                        if cursor_pos > 0 {
                            cursor_pos -= 1;
                            res!(execute!(stdout(), cursor::MoveLeft(1)));
                        }
                    }
                    KeyCode::Right => {
                        if cursor_pos < input.len() {
                            cursor_pos += 1;
                            res!(execute!(stdout(), cursor::MoveRight(1)));
                            res!(stdout().flush());
                        }
                    }
                    KeyCode::Up => {
                        if self.hist_ind > 0 {
                            self.hist_ind -= 1;
                            input = self.hist[self.hist_ind].clone();
                            cursor_pos = input.len();
                        } else {
                            continue;
                        }
    
                        res!(execute!(stdout(), Clear(ClearType::CurrentLine)));
                        print!("\r{} {}", self.cfg.norm_prompt, input);
                        res!(stdout().flush());
                    }
                    KeyCode::Down => {
                        if self.hist_ind < self.hist.len() - 1 {
                            self.hist_ind += 1;
                            input = self.hist[self.hist_ind].clone();
                            cursor_pos = input.len();
                        } else {
                            continue;
                        }
    
                        res!(execute!(stdout(), Clear(ClearType::CurrentLine)));
                        print!("\r{} {}", self.cfg.norm_prompt, input);
                        res!(stdout().flush());
                    }
                    KeyCode::Backspace => {
                        if cursor_pos > 0 && !input.is_empty() {
                            // Correctly remove the character at the cursor position
                            input.remove(cursor_pos - 1);
                            cursor_pos -= 1;
                    
                            // Move cursor back by one to reflect the character removal
                            res!(execute!(stdout(), cursor::MoveLeft(1)));
                    
                            // Clear from the cursor to the end of the line to prepare for redraw
                            res!(execute!(stdout(), Clear(ClearType::UntilNewLine)));
                    
                            // Print the rest of the string after the cursor
                            print!("{}", &input[cursor_pos..]);
                    
                            // Move the cursor back to its correct position after redraw.
                            // This step might not be necessary if the cursor is at the end of the input,
                            // but it's required if the cursor is in the middle of the input.
                            let chars_to_move_left = input.len().saturating_sub(cursor_pos);
                            if chars_to_move_left > 0 {
                                res!(execute!(stdout(), cursor::MoveLeft(chars_to_move_left as u16)));
                            }
                    
                            res!(stdout().flush());
                        }
                    }
                    KeyCode::Delete => {
                        if cursor_pos < input.len() {
                            // Remove the character right after the cursor position
                            input.remove(cursor_pos);
                    
                            // Clear from the cursor to the end of the line to prepare for redraw
                            res!(execute!(stdout(), Clear(ClearType::UntilNewLine)));
                    
                            // Print the rest of the string after the cursor
                            print!("{}", &input[cursor_pos..]);
                    
                            // Move the cursor back to its correct position after redrawing.
                            // Calculate the number of characters to move left based on the length of the input
                            // minus the current cursor position, and then move the cursor left that many spaces.
                            let chars_to_move_left = input.len().saturating_sub(cursor_pos);
                            if chars_to_move_left > 0 {
                                res!(execute!(stdout(), cursor::MoveLeft(chars_to_move_left as u16)));
                            }
                    
                            res!(stdout().flush());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn start(&mut self) -> Outcome<()> {
        println!("{}", self.cfg.greeting_msg);
        let hist_path = Path::new(".").join(self.cfg.history_name.clone());
        let mut hist_file = res!(OpenOptions::new()
            .append(true)
            .create(true)
            .open(hist_path)
        );

        print!("{} ", self.cfg.norm_prompt);
        res!(stdout().flush());

        let mut exit_flag = false;

        while let Some(input) = res!(self.read_line()) {
            if !input.trim().is_empty() {
                res!(writeln!(hist_file, "{}", input));
                res!(hist_file.flush());

                match self.context.eval(
                    &input,
                    &self.cfg,
                    &self.splitters,
                ) {
                    Ok(evals) => for eval in evals {
                        match eval {
                            Evaluation::Output(s) => println!("{} {}", self.cfg.norm_prompt, s),
                            Evaluation::Error(s) => println!(
                                "{}{} {}{}",
                                Term::SET_BRIGHT_FORE_RED,
                                self.cfg.alert_prompt,
                                s,
                                Term::RESET,
                            ),
                            Evaluation::Exit => exit_flag = true,
                            _ => (),
                        }
                    }
                    Err(e) => println!("Error: {}", e),
                }

                self.hist.push(input);
                self.hist_ind = self.hist.len();
                if exit_flag {
                    break;
                }
            }

            res!(execute!(stdout(), Clear(ClearType::CurrentLine)));
            print!("\r{} ", self.cfg.norm_prompt);
            res!(stdout().flush());
        }

        Ok(())
    }
}
