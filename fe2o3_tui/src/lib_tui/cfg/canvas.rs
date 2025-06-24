use crate::lib_tui::{
    cfg::style::StyleLibrary,
    draw::canvas::CanvasConfig,
    style::{
        Colour,
        Style,
    },
};

use oxedyne_fe2o3_core::prelude::*;


#[derive(Clone, Debug, Default)]
pub struct CanvasLibrary;

impl CanvasLibrary {}

impl StyleLibrary {

    pub fn canvas_colour(
        &self,
        fore: Option<Colour>,
        back: Option<Colour>,
    )
        -> CanvasConfig
    {
        CanvasConfig {
            style: Style::new(fore, back, None),
            ..Default::default()
        }
    }
}
