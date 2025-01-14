use crate::lib_tui::{
    cfg::{
        line::LineType,
        style::StyleLibrary,
    },
    draw::outline::{
        OutlineConfig,
        OutlineConfigs,
    },
    style::{
        Colour,
        Style,
    },
};

use oxedize_fe2o3_core::prelude::*;


impl StyleLibrary {

    pub fn default_window_outlines(&self) -> OutlineConfigs {
        OutlineConfigs {
            normal: OutlineConfig {
                style: Style::new(None, None, None),
                typ: LineType::Blank,
            },
            focus: OutlineConfig {
                style: Style::new(Some(Colour::White), None, None),
                typ: LineType::ThickSingleSharp,
            },
            interact: OutlineConfig {
                style: Style::new(Some(Colour::Green), None, None),
                typ: LineType::ThickSingleSharp,
            },
            mgmt_mode: OutlineConfig {
                style: Style::new(Some(Colour::Red), None, None),
                typ: LineType::ThickSingleSharp,
            },
            mgmt_tint: OutlineConfig {
                style: Style::new(Some(Colour::Gray), None, None),
                typ: LineType::ThickSingleSharp,
            },
            mgmt_focus: OutlineConfig {
                style: Style::new(Some(Colour::White), Some(Colour::Green), None),
                typ: LineType::ThickSingleSharp,
            },
        }
    }
    
}
