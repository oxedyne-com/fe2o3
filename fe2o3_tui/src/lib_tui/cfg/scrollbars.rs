use crate::lib_tui::{
    cfg::style::StyleLibrary,
    draw::scrollbars::ScrollBarsConfig,
    style::{
        Colour,
        Style,
    },
};

use oxedize_fe2o3_core::prelude::*;


#[derive(Clone, Debug, Default)]
pub struct ScrollBarsLibrary;

impl StyleLibrary {

    pub fn standard_scrollbars(&self) -> ScrollBarsConfig {
        ScrollBarsConfig {
            style: Style::new(Some(Colour::Gray), Some(Colour::DarkGray), None),
            horiz:  '\u{2501}',
            vert:   '\u{2503}',
            corner: Some(' '),
            always: true,
        }
    }
}
