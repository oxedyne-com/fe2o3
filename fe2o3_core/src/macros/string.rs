#[macro_export]
/// Print a line to the console including the source file and line info.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
///
/// let trooper = 421;
/// msg!("TK{} why aren't you at your post?", trooper);
///```
macro_rules! msg {
    () => (println!("{}:{}\n",file!(),line!()));
    ($($arg:tt)*) => ({
        print!("{}:{}: ",file!(),line!());
        println!($($arg)*);
    })
}

#[macro_export]
/// A three letter alias for `std::format!`.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
///
/// let s = fmt!("The meaning is {}", 42);
///```
macro_rules! fmt {
    () => (String::from(""));
    ($($arg:tt)*) => (format!($($arg)*));
}

#[macro_export]
/// Return the type of the given expression as a `String`.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
///
/// let typ = fmt_typ!(42u8);
///```
macro_rules! fmt_typ {
    () => (String::from(""));
    ($v:expr) => (oxedize_fe2o3_core::format_type($v));
}

#[macro_export]
/// A three letter alias for wrapping a string in a `Stringer`.
///
///```
/// use oxedize_fe2o3_core::prelude::*;
///
/// let s = str!("The meaning is {}", 42);
///```
macro_rules! str {
    () => (Stringer::from(""));
    ($($arg:tt)*) => (Stringer::new($($arg)*));
}

#[macro_export]
/// Dump of a byte slice to `String`s.
///
/// Convert a byte slice to text values in a vector of `String` lines.  The format string is
/// applied without modiciation to each byte, with no change to the first and last lines.
///
/// Arguments:
///
/// * format string including placeholder,
/// * byte slice,
/// * maximum number of bytes per line,
///
/// # Example
///```
/// use oxedize_fe2o3_core::dump;
///
/// let b = [0_u16; 10];
/// let lines: Vec<String> = dump!(" {:04x}", &b, 4);
///```
macro_rules! dump {
    ($f:tt, $b:expr, $c:expr) => {
        {
            let mut lines: Vec<String> = Vec::new();
            let mut line = String::new();
            let mut i: usize = 1;
            for e in $b {
                line.push_str(&format!($f, e));
                if i % $c == 0 {
                    lines.push(line);
                    line = String::new();
                }
                i += 1;
            }
            if line.len() > 0 {
                lines.push(line);
            }
            lines
        }
    };
    ($f:tt, $b:expr) => {
        dump!($f, $b, 8);
    };
}
