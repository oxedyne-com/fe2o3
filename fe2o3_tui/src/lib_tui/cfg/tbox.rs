use crate::lib_tui::{
    cfg::style::StyleLibrary,
    draw::{
        tbox::{
            TextBox,
            TextBoxConfig,
        },
    },
    text::nav::PositionCursor,
    render::CursorStyle,
    text::{
        //highlight::{
        //    BuilderType,
        //    StyledHighlighter,
        //},
        nav::Navigator,
        typ::{
            ContentType,
            TextViewType,
        },
        view::TextView,
    },
    style::{
        Colour,
        Style,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_file::tree::FileTree;
use oxedize_fe2o3_text::{
    access::AccessibleText,
    lines::TextLines,
};

use std::{
    rc::Rc,
    sync::RwLock,
};


#[derive(Clone, Debug, Default)]
pub struct TextBoxLibrary;

impl StyleLibrary {

    pub fn navigable_text_box(&self) -> TextBoxConfig {
        TextBoxConfig {
            cursor_style:       Some(CursorStyle::BlinkingBlock),
            cursor_position:    PositionCursor::UserControlled,
            scrollbars:         Some(self.standard_scrollbars()),
            empty_line:         Some((Style::new(Some(Colour::Red), None, None), fmt!("~"))),
            highlight_styles:   self.basic_highlight_styles(),
        }
    }

    pub fn menu_text_box(&self) -> TextBoxConfig {
        TextBoxConfig {
            cursor_style:       None,
            cursor_position:    PositionCursor::UserControlled,
            scrollbars:         Some(self.standard_scrollbars()),
            empty_line:         None,
            highlight_styles:   self.basic_highlight_styles(),
        }
    }

    pub fn passive_text_box(&self) -> TextBoxConfig {
        TextBoxConfig {
            cursor_style:       None,
            cursor_position:    PositionCursor::LatestLine(false),
            scrollbars:         None,
            empty_line:         None,
            highlight_styles:   self.basic_highlight_styles(),
        }
    }

    pub fn file_tree_text_box(&self) -> Outcome<TextBox> {
        let cfg = TextBoxConfig {
            cursor_style:       None,
            cursor_position:    PositionCursor::UserControlled,
            scrollbars:         Some(self.standard_scrollbars()),
            empty_line:         None,
            highlight_styles:   self.basic_highlight_styles(),
        };
        let tree = res!(FileTree::new("."));
        let lines = res!(tree.display(true));
        let mut tlines = TextLines::new(Vec::new(), None);
        for line in lines {
            debug!("{}",line);
            tlines.append_string(line);
        }
        TextBox::new(
            cfg,
            res!(TextView::new(
                ContentType::Menu,
                TextViewType::FileTree(
                    tree,
                    Navigator::new(None),
                ),
                AccessibleText::Shared(Rc::new(RwLock::new(tlines))),
            )),
        )
    }
}
