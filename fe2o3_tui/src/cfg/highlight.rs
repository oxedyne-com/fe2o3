use crate::{
    cfg::style::StyleLibrary,
    style::{
        Colour,
        Style,
    },
};


impl StyleLibrary {

    pub fn basic_highlight_styles(&self) -> Vec<Style> {
        vec![
            Style::new(Some(Colour::White),     Some(Colour::Green),    None),
            Style::new(Some(Colour::White),     Some(Colour::Red),      None),
            Style::new(Some(Colour::White),     Some(Colour::Blue),     None),
            Style::new(Some(Colour::Black),     Some(Colour::Yellow),   None),
            Style::new(Some(Colour::LightRed),  Some(Colour::Black),    None),
        ]
    }
}
