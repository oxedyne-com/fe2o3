use crate::lib_tui::{
    action::Action,
    cfg::style::StyleLibrary,
    draw::{
        tab::{
            TabStripConfig,
            TabbedTextBox,
            TabbedTextManager,
            TabbedTile,
        },
        tbox::TextBox,
        window::{
            TextBoxesState,
            WindowConfig,
            WindowId,
            WindowStateInit,
            WindowType,
        },
    },
    style::{
        Colour,
        Style,
        Symbol,
    },
    text::{
        nav::Navigator,
        typ::{
            ContentType,
            HighlightType,
            TextType,
            TextViewType,
        },
        view::TextView,
    },
    window::{
        MenuList,
        WindowManagerConfig,
    },
};

use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_file::tree::FileTree;
use oxedize_fe2o3_geom::{
    dim::{
        Coord,
        Dim,
        FlexDim,
    },
    rect::{
        AbsSize,
        Position,
        RelativePosition,
        RelRect,
        RelSize,
        RectView,
    },
};
use oxedize_fe2o3_text::{
    Text,
    access::AccessibleText,
    highlight::Highlighter,
    lines::TextLines,
};

use std::{
    rc::Rc,
    sync::{
        Arc,
        RwLock,
    },
};


impl StyleLibrary {

    pub fn default_help_window(
        &self,
        cfg: &WindowManagerConfig,
    )
        -> Outcome<(
            WindowId,
            WindowConfig,
            RectView,
            WindowStateInit,
        )>
    {
        let id = WindowId::Help;
        let view = RectView::InitiallyRelative(RelRect::FixSize {
            top_left:   Position::Relative(RelativePosition::Centre),
            size:       AbsSize::new((Dim(80), Dim(50))),
        });
        let cfg = WindowConfig {
            typ:        WindowType::Fixed,
            canvas:     self.canvas_colour(None, None),
            outlines:   self.default_window_outlines(),
            header:     Some(self.standard_header_config(Some(Colour::White), Some(Colour::Magenta))),
            footer:     Some(self.standard_footer_config(Some(Colour::White), Some(Colour::Green))),
            min_size:   cfg.window_min_size(),
            menu_text:  None,
        };
        let help_txt = [
            Text::from("Ironic is a modal text user interface.  Initially you begin in Window Navigation mode."),
            Text::from("  Esc:            Exit the current mode.  Repeat to return to Window Navigation mode."),
            Text::from("WINDOW NAVIGATION MODE"),
            Text::from("  Tab:            Move the window focus."),
            Text::from("  Space:          Enter Window Management mode for the chosen window."),
            Text::from("  Enter:          Enter Window Interaction mode for the chosen window."),
            Text::from("WINDOW MANAGEMENT MODE"),
            Text::from("  Arrow keys:     Translate the window around the screen."),
            Text::from("  Space again:    Enter the window size editor.  Pressing the space bar again moves"),
            Text::from("                  the focus to the next window border.  Use the arrow keys to resize."),
            Text::from("WINDOW INTERACTION MODE"),
            Text::from("  Tab:            Move the tab focus."),
            Text::from("  i:              Enter text Insert mode."),
            Text::from("  r:              Enter text Replace mode."),
            Text::from("  m:              Enter text Modify mode."),
            Text::from("  Arrow keys:     Move the cursor."),
            Text::from("  Shift + Arrow:  Pan across the text."),
        ];
        let state = WindowStateInit {
            header: Some(self.status_strip_labels(Some(id.title()), &id.label())),
            footer: None,
            text_boxes: Some(TextBoxesState::Tabbed(res!(TabbedTextManager::new(
                res!(self.standard_tab_styles()),
                vec![
                    TabbedTextBox {
                        tbox: res!(TextBox::new(
                            self.navigable_text_box(),
                            res!(TextView::new(
                                ContentType::Text,
                                TextViewType::Static(Navigator::default()),
                                AccessibleText::Shared(Rc::new(RwLock::new(
                                    TextLines::new(
                                        Self::DEFAULT_SPLASH.lines()
                                            .map(|s|
                                                 Text::new(fmt!("{}{}", " ".repeat(10), s), None)
                                            ).collect::<Vec<_>>(),
                                        None,
                                    ),
                                ))),
                            )),
                        )),
                        tab: self.tab_with_label(Some(Symbol::Info), "About"),
                    },
                    TabbedTextBox {
                        tbox: res!(TextBox::new(
                            self.navigable_text_box(),
                            res!(TextView::new(
                                ContentType::Text,
                                TextViewType::Static(Navigator::default()),
                                AccessibleText::Shared(Rc::new(RwLock::new(
                                    TextLines::from(help_txt)
                                ))),
                            )),
                        )),
                        tab: self.tab_with_label(Some(Symbol::Keyboard), "Keys"),
                    },
                    TabbedTextBox {
                        tbox: res!(TextBox::new(
                            self.navigable_text_box(),
                            res!(TextView::new(
                                ContentType::FileTree,
                                TextViewType::FileTree(res!(FileTree::new(".")), Navigator::default()),
                                AccessibleText::Shared(Rc::new(RwLock::new(
                                    TextLines::new(
                                        vec![], 
                                        Some(Highlighter::default()),
                                    ),
                                ))),
                            )),
                        )),
                        tab: self.tab_with_label(Some(Symbol::FileTree), "File Tree"),
                    },
                ],
                Some(vec![
                    TabbedTile {
                        index: 0,
                        ..Default::default()
                    },
                    TabbedTile {
                        index: 2,
                        ..Default::default()
                    },
                ]),
                Some(0),
            )))),
        };
        Ok((
            id,
            cfg,
            view,
            state,
        ))
    }
    
    pub fn default_log_window(
        &self,
        cfg: &WindowManagerConfig,
        log: Arc<RwLock<TextLines<TextType, HighlightType>>>,
    )
        -> Outcome<(
            WindowId,
            WindowConfig,
            RectView,
            WindowStateInit,
        )>
    {
        let id = WindowId::Log;
        let view = RectView::AlwaysRelative(RelRect::RelSize(RelSize::new((
            FlexDim::PadFillPad(Dim(0), Dim(0)),
            FlexDim::FillFixedPad(Dim(20), Dim(0)),
        ))));
        let cfg = WindowConfig {
            typ:        WindowType::Fixed,
            canvas:     self.canvas_colour(Some(Colour::White), None),
            outlines:   self.default_window_outlines(),
            header:     Some(self.standard_header_config(Some(Colour::Black), Some(Colour::LightBlue))),
            footer:     None,
            min_size:   cfg.window_min_size(),
            menu_text:  None,
        };
        let state = WindowStateInit {
            header: Some(self.status_strip_labels(Some(id.title()), &id.label())),
            footer: None,
            text_boxes: Some(TextBoxesState::Single(res!(TextBox::new(
                self.passive_text_box(),
                res!(TextView::new(
                    ContentType::Log,
                    TextViewType::Static(Navigator::default()),
                    AccessibleText::ThreadShared(log),
                )),
            )))),
        };
        Ok((
            id,
            cfg,
            view,
            state,
        ))
    }

    pub fn default_menu_window(
        &self,
        cfg:            &WindowManagerConfig,
        window_list:    Rc<RwLock<TextLines<TextType, HighlightType>>>,
        menu_text:      Arc<RwLock<TextLines<TextType, HighlightType>>>,
    )
        -> Outcome<(
            WindowId,
            WindowConfig,
            RectView,
            WindowStateInit,
        )>
    {
        let id = WindowId::Menu;
        let view = RectView::InitiallyRelative(RelRect::FixSize {
            top_left:   Position::Relative(RelativePosition::TopLeft),
            size:       AbsSize::new((Dim(50), Dim(50))),
        });
        let cfg = WindowConfig {
            typ:        WindowType::Fixed,
            canvas:     self.canvas_colour(Some(Colour::White), Some(Colour::LightBlue)),
            outlines:   self.default_window_outlines(),
            header:     Some(self.standard_header_config(Some(Colour::Blue), Some(Colour::Cyan))),
            footer:     Some(self.standard_footer_config(Some(Colour::White), Some(Colour::Blue))),
            min_size:   cfg.window_min_size(),
            menu_text:  None,
        };
        let state = WindowStateInit {
            header: Some(self.status_strip_labels(Some(id.title()), &id.label())),
            footer: None,
            text_boxes: Some(TextBoxesState::Tabbed(res!(TabbedTextManager::new(
                res!(TabStripConfig::new(vec![
                    Style::new(Some(Colour::White),     Some(Colour::Blue),     None),
                    Style::new(Some(Colour::White),     Some(Colour::Green),    None),
                    Style::new(Some(Colour::White),     Some(Colour::Red),      None),
                    Style::new(Some(Colour::Black),     Some(Colour::Yellow),   None),
                    Style::new(Some(Colour::LightRed),  Some(Colour::Black),    None),
                ])),
                vec![
                    TabbedTextBox {
                        tbox: res!(TextBox::new(
                            self.menu_text_box(),
                            res!(TextView::new(
                                ContentType::Menu,
                                TextViewType::Menu(Navigator::new(None)),
                                AccessibleText::ThreadShared(menu_text),
                            )),
                        )),
                        tab: self.tab_with_label(Some(Symbol::Menu), "Menu"),
                    },
                    TabbedTextBox {
                        tbox: res!(TextBox::new(
                            self.menu_text_box(),
                            res!(TextView::new(
                                ContentType::Menu,
                                TextViewType::WindowList(Navigator::new(None)),
                                AccessibleText::Shared(window_list),
                            )),
                        )),
                        tab: self.tab_with_label(Some(Symbol::Windows), "Windows"),
                    },
                ],
                None,
                None,
            )))),
        };
        Ok((
            id,
            cfg,
            view,
            state,
        ))
    }

    pub fn default_new_window(
        &self,
        cfg:        &WindowManagerConfig,
        id:         &WindowId,
        top_left:   Coord,
    )
        -> Outcome<(
            WindowConfig,
            RectView,
            WindowStateInit,
        )>
    {
        let view = RectView::InitiallyRelative(RelRect::RelSize(RelSize::new((
            FlexDim::PadFixedFill(top_left.x, Dim(60)),
            FlexDim::PadFixedFill(top_left.y, Dim(40)),
        ))));
        let cfg = WindowConfig {
            typ:        WindowType::Fixed,
            canvas:     self.canvas_colour(None, None),
            outlines:   self.default_window_outlines(),
            header:     Some(self.standard_header_config(
                Some(Colour::White),
                Some(Colour::Magenta),
            )),
            footer:     Some(self.standard_footer_config(
                Some(Colour::White),
                Some(Colour::Blue),
            )),
            min_size:   cfg.window_min_size(),
            menu_text:  Some(res!(MenuList::from(vec![
                (
                    TextType::new_menu_heading( "Create new tab                            "),
                    None,
                ),
                (
                    TextType::new_menu_item(    " Create new text tab                      "),
                    Some(Action::CreateNewTextTab),
                ),
            ]).text_lines())),
        };
        let ctyp = ContentType::FileTree;
        let state = WindowStateInit {
            header: Some(self.status_strip_labels_with_mode(
                Some(id.title()),
                &id.label(),
            )),
            footer: Some(self.status_strip_cursor(None::<&str>)),
            text_boxes: Some(TextBoxesState::Tabbed(res!(TabbedTextManager::new(
                res!(self.standard_tab_styles()),
                vec![
                    res!(self.tabbed_text_box(ctyp)),
                ],
                None,
                None,
            )))),
        };
        Ok((
            cfg,
            view,
            state,
        ))
    }
}
