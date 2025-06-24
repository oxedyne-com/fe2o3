use crate::lib_tui::{
    cfg::style::StyleLibrary,
    draw::status::{
        StatusStripConfig,
        StatusStripContent,
        StatusStripLeft,
        StatusStripRight,
        StatusStripType,
    },
    style::{
        Colour,
        Style,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_geom::dim::Coord;


#[derive(Clone, Debug, Default)]
pub struct StatusStripLibrary;

impl StatusStripLibrary {}

impl StyleLibrary {

    pub fn standard_header_config(
        &self,
        fore: Option<Colour>,
        back: Option<Colour>,
    )
        -> StatusStripConfig
    {
        StatusStripConfig {
            style: Style::new(fore, back, None),
            typ: StatusStripType::Header,
            ..Default::default()
        }
    }

    pub fn standard_footer_config(
        &self,
        fore: Option<Colour>,
        back: Option<Colour>,
    )
        -> StatusStripConfig
    {
        StatusStripConfig {
            style: Style::new(fore, back, None),
            typ: StatusStripType::Footer,
            ..Default::default()
        }
    }

    pub fn status_strip_labels<S: Into<String>>(
        &self,
        left:   Option<S>,
        right:  S,
    )
        -> StatusStripContent
    {
        StatusStripContent {
            left: match left {
                Some(left) => Some(StatusStripLeft::Origin(left.into())),
                None => None,
            },
            right: Some(StatusStripRight::Label(right.into())),
            ..Default::default()
        }
    }

    pub fn status_strip_labels_with_mode<S: Into<String>>(
        &self,
        left:   Option<S>,
        right:  S,
    )
        -> StatusStripContent
    {
        StatusStripContent {
            left: match left {
                Some(left) => Some(StatusStripLeft::Origin(left.into())),
                None => None,
            },
            right: Some(StatusStripRight::ModeLabel(None, right.into())),
            ..Default::default()
        }
    }

    pub fn status_strip_cursor<S: Into<String>>(
        &self,
        left: Option<S>,
    )
        -> StatusStripContent
    {
        StatusStripContent {
            left: match left {
                Some(left) => Some(StatusStripLeft::Origin(left.into())),
                None => None,
            },
            right: Some(StatusStripRight::Cursor(Coord::default())),
            ..Default::default()
        }
    }
}
