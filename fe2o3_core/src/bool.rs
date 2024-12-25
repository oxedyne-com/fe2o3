use std::fmt;

#[derive(Clone, Debug)]
pub enum BoolFormatter {
    TrueFalse(bool),
    OnOff(bool),
    YesNo(bool),
}

impl fmt::Display for BoolFormatter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::TrueFalse(b) => if *b {
                write!(f, "true")
            } else {
                write!(f, "false")
            },
            Self::OnOff(b) => if *b {
                write!(f, "on")
            } else {
                write!(f, "off")
            },
            Self::YesNo(b) => if *b {
                write!(f, "yes")
            } else {
                write!(f, "no")
            },
        }
    }
}
