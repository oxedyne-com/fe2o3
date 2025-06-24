use crate::lib_tui::{
    render::{
        Drawable,
        Drawer,
        Renderer,
        Sink,
        When,
    },
};

use oxedyne_fe2o3_core::prelude::*;

use std::fmt;


#[derive(Clone, Debug, Default)]
pub struct Style {
    pub cols: Colours,
    pub attr: Vec<Attribute>,
}

impl Style {
    pub fn new(
        fore: Option<Colour>,
        back: Option<Colour>,
        attr: Option<Vec<Attribute>>,
    )
        -> Self
    {
        Self {
            cols: Colours::new(fore, back),
            attr: match attr {
                Some(list) => list,
                None => Vec::new(),
            },
        }
    }
}

impl Drawable for Style {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        res!(drawer.rend.set_style(self, when));
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Colours {
    pub fore: Option<Colour>,
    pub back: Option<Colour>,
}

impl Drawable for Colours {
    fn render<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        if let Some(col) = self.back {
            res!(drawer.rend.set_back_colour(&col, when));
        }
        if let Some(col) = self.fore {
            res!(drawer.rend.set_fore_colour(&col, when));
        }
        Ok(())
    }
}

impl Colours {
    pub fn new(fore: Option<Colour>, back: Option<Colour>) -> Self {
        Self {
            fore,
            back,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Colour {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightBlue,
    LightYellow,
    LightMagenta,
    LightCyan,
    White,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<Colour> for crossterm::style::Color {
    fn from(colour: Colour) -> Self {
        match colour {
            Colour::Reset        => crossterm::style::Color::Reset,
            Colour::Black        => crossterm::style::Color::Black,
            Colour::Red          => crossterm::style::Color::DarkRed,
            Colour::Green        => crossterm::style::Color::DarkGreen,
            Colour::Yellow       => crossterm::style::Color::DarkYellow,
            Colour::Blue         => crossterm::style::Color::DarkBlue,
            Colour::Magenta      => crossterm::style::Color::DarkMagenta,
            Colour::Cyan         => crossterm::style::Color::DarkCyan,
            Colour::Gray         => crossterm::style::Color::Grey,
            Colour::DarkGray     => crossterm::style::Color::DarkGrey,
            Colour::LightRed     => crossterm::style::Color::Red,
            Colour::LightGreen   => crossterm::style::Color::Green,
            Colour::LightBlue    => crossterm::style::Color::Blue,
            Colour::LightYellow  => crossterm::style::Color::Yellow,
            Colour::LightMagenta => crossterm::style::Color::Magenta,
            Colour::LightCyan    => crossterm::style::Color::Cyan,
            Colour::White        => crossterm::style::Color::White,
            Colour::Indexed(i)   => crossterm::style::Color::AnsiValue(i),
            Colour::Rgb(r, g, b) => crossterm::style::Color::Rgb { r, g, b },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Attribute {
    Reset,
    Bold,
    Dim,
    Italic,
    Underlined,
    DoubleUnderlined,
    Undercurled,
    Underdotted,
    Underdashed,
    SlowBlink,
    RapidBlink,
    Reverse,
    Hidden,
    CrossedOut,
    Fraktur,
    NoBold,
    NormalIntensity,
    NoItalic,
    NoUnderline,
    NoBlink,
    NoReverse,
    NoHidden,
    NotCrossedOut,
    Framed,
    Encircled,
    OverLined,
    NotFramedOrEncircled,
    NotOverLined,
}

impl From<Attribute> for crossterm::style::Attribute {
    fn from(attr: Attribute) -> Self {
        match attr {
            Attribute::Reset                => crossterm::style::Attribute::Reset,               
            Attribute::Bold                 => crossterm::style::Attribute::Bold,                
            Attribute::Dim                  => crossterm::style::Attribute::Dim,                 
            Attribute::Italic               => crossterm::style::Attribute::Italic,              
            Attribute::Underlined           => crossterm::style::Attribute::Underlined,          
            Attribute::DoubleUnderlined     => crossterm::style::Attribute::DoubleUnderlined,    
            Attribute::Undercurled          => crossterm::style::Attribute::Undercurled,         
            Attribute::Underdotted          => crossterm::style::Attribute::Underdotted,         
            Attribute::Underdashed          => crossterm::style::Attribute::Underdashed,         
            Attribute::SlowBlink            => crossterm::style::Attribute::SlowBlink,           
            Attribute::RapidBlink           => crossterm::style::Attribute::RapidBlink,          
            Attribute::Reverse              => crossterm::style::Attribute::Reverse,             
            Attribute::Hidden               => crossterm::style::Attribute::Hidden,              
            Attribute::CrossedOut           => crossterm::style::Attribute::CrossedOut,          
            Attribute::Fraktur              => crossterm::style::Attribute::Fraktur,             
            Attribute::NoBold               => crossterm::style::Attribute::NoBold,              
            Attribute::NormalIntensity      => crossterm::style::Attribute::NormalIntensity,     
            Attribute::NoItalic             => crossterm::style::Attribute::NoItalic,            
            Attribute::NoUnderline          => crossterm::style::Attribute::NoUnderline,         
            Attribute::NoBlink              => crossterm::style::Attribute::NoBlink,             
            Attribute::NoReverse            => crossterm::style::Attribute::NoReverse,           
            Attribute::NoHidden             => crossterm::style::Attribute::NoHidden,            
            Attribute::NotCrossedOut        => crossterm::style::Attribute::NotCrossedOut,       
            Attribute::Framed               => crossterm::style::Attribute::Framed,              
            Attribute::Encircled            => crossterm::style::Attribute::Encircled,           
            Attribute::OverLined            => crossterm::style::Attribute::OverLined,           
            Attribute::NotFramedOrEncircled => crossterm::style::Attribute::NotFramedOrEncircled,
            Attribute::NotOverLined         => crossterm::style::Attribute::NotOverLined,        
        }
    }
}

#[derive(Clone, Debug)]
pub enum Symbol {
    Buffer,
    Database,
    File,
    FileTree,
    Info,
    Keyboard,
    Menu,
    Shell,
    Text,
    Windows,
}

/// Note that width calculation seems to be defective when multiple emoji characters are used with
/// the GNOME terminal, hence sticking to only a single character.
impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Buffer     => write!(f, "\u{2700}"),
            Self::Database   => write!(f, "\u{1f5ae}"),
            Self::File       => write!(f, "\u{1f4c2}"),
            Self::FileTree   => write!(f, "\u{1f332}"),
            Self::Info       => write!(f, "\u{02139}"),
            Self::Keyboard   => write!(f, "\u{1f5ae}"),
            Self::Menu       => write!(f, "\u{2630}"),
            Self::Shell      => write!(f, "\u{2630}"),
            Self::Text       => write!(f, "\u{1f4c2}"),
            Self::Windows    => write!(f, "\u{1fa9f}"),
        }
    }
}
