use crate::lib_tui::{
    cfg::style::StyleLibrary,
    draw::{
        tab::{
            Tab,
            TabbedTextBox,
            TabStripConfig,
        },
        tbox::TextBox,
    },    
    style::{
        Colour,
        Style,
        Symbol,
    },
    text::{
        edit::Editor,
        typ::{
            ContentType,
            TextViewType,
        },
        view::TextView,
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_text::{
    Text,
    access::AccessibleText,
    lines::TextLines,
};

use std::{
    rc::Rc,
    sync::{
        RwLock,
    },
};


#[derive(Clone, Debug, Default)]
pub struct TabLibrary;

impl TabLibrary {

    pub fn tab_label(&self, symbol: Option<Symbol>, label: &str) -> String {
        fmt!("{}{}",
            match symbol {
                Some(symbol) => fmt!("{} ", symbol),
                None => String::new(),
            },
            label,
        )
    }
}

impl StyleLibrary {

    pub fn tab_with_label(&self, symbol: Option<Symbol>, label: &str) -> Tab {
        Tab {
            label: self.tab.tab_label(symbol, label),
            ..Default::default()
        }
    }

    pub fn standard_tab_styles(&self) -> Outcome<TabStripConfig> {
        TabStripConfig::new(vec![
            Style::new(Some(Colour::White),     None,  None),
            Style::new(Some(Colour::White),     Some(Colour::Green),    None),
            Style::new(Some(Colour::White),     Some(Colour::Red),      None),
            Style::new(Some(Colour::White),     Some(Colour::Blue),     None),
            Style::new(Some(Colour::Black),     Some(Colour::Yellow),   None),
            Style::new(Some(Colour::LightRed),  Some(Colour::Black),    None),
        ])
    }

    pub fn tabbed_text_box(&self, ctyp: ContentType) -> Outcome<TabbedTextBox> {
        Ok(TabbedTextBox {
            tbox: match ctyp {
                ContentType::FileTree => {
                    res!(self.file_tree_text_box())
                }
                _ => {
                    res!(TextBox::new(
                        self.navigable_text_box(),
                        res!(TextView::new(
                            ContentType::Text,
                            TextViewType::Editable(Editor::default()),
                            AccessibleText::Shared(
                                Rc::new(RwLock::new(
                                    TextLines::new(
                                        vec![Text::new("", None)],
                                        None,
                                    ),
                                )),
                            ),
                        )),
                    ))
                }
            },
            tab: self.tab_with_label(ctyp.symbol(), ctyp.label()),
        })
    } 
}
