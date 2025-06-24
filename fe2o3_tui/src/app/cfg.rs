use crate::{
    App,
    lib_tui::{
        action::Action,
        cfg::style::StyleLibrary,
        draw::{
            tab::{
                TabbedTextBox,
                TabbedTextManager,
            },
            tbox::TextBox,
            window::{
                TextBoxesState,
                WindowConfig,
                WindowStateInit,
                WindowType,
            },
        },
        style::{
            Colour,
            Symbol,
        },
        text::{
            edit::Editor,
            typ::{
                ContentType,
                TextType,
                TextViewType,
            },
            view::TextView,
        },
        window::{
            MenuList,
            WindowManagerConfig,
        },
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_geom::{
    dim::{
        Dim,
        FlexDim,
    },
    rect::{
        AbsSize,
        RectView,
        RelSize,
        RelRect,
    },
};
use oxedyne_fe2o3_text::{
    Text,
    access::AccessibleText,
    lines::TextLines,
};

use std::{
    rc::Rc,
    sync::RwLock,
};


#[derive(Clone, Debug)]
pub struct AppConfig {
    pub tab_string:         String,
    pub window_count_limit: u8,
    pub window_min_size_x:  u8,
    pub window_min_size_y:  u8,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            tab_string:         fmt!("    "),
            window_count_limit: 100,
            window_min_size_x:  6,
            window_min_size_y:  6,
        }
    }
}

impl From<AppConfig> for WindowManagerConfig {
    fn from(cfg: AppConfig) -> Self {
        Self {
            tab_string:         cfg.tab_string.clone(),
            window_count_limit: cfg.window_count_limit,
            window_min_size_x:  cfg.window_min_size_x,
            window_min_size_y:  cfg.window_min_size_y,
            ..Default::default()
        }
    }
}

impl App<'_> {

    pub fn make_windows(&mut self, _term: AbsSize) -> Outcome<()> {

        let min_size = AbsSize::new((Dim(6), Dim(6)));
        let lib = ConfigLibrary::default();

        let main_win = RectView::InitiallyRelative(RelRect::RelSize(RelSize::new((
            FlexDim::PadFixedFill(Dim(50), Dim(70)),
            FlexDim::PadFixedFill(Dim(2), Dim(50)),
        ))));
        let main_win_cfg = WindowConfig {
            typ:        WindowType::Fixed,
            canvas:     lib.fe2o3.canvas_colour(None, None),
            outlines:   lib.fe2o3.default_window_outlines(),
            header:     Some(lib.fe2o3.standard_header_config(Some(Colour::White), Some(Colour::Magenta))),
            footer:     Some(lib.fe2o3.standard_footer_config(Some(Colour::White), Some(Colour::Blue))),
            min_size:   min_size.clone(),
            menu_text:  Some(res!(MenuList::from(vec![
                (
                    TextType::new_menu_item("Create new text tab                       "),
                    Some(Action::CreateNewTextTab),
                ),
            ]).text_lines())),
        };

        let (id, cfg, view, state) = res!(lib.fe2o3.default_log_window(&self.win_mgr.cfg, self.log.clone()));
        res!(self.win_mgr.add_window(
            Some(id.clone()),
            cfg,
            view,
        ));
        res!(self.win_mgr.set_state(&id, state));

        let (id, cfg, view, state) = res!(lib.fe2o3.default_menu_window(
            &self.win_mgr.cfg,
            self.win_mgr.state.window_list.clone(),
            self.win_mgr.state.menu_text.clone(),
        ));
        res!(self.win_mgr.add_window(
            Some(id.clone()),
            cfg,
            view,
        ));
        res!(self.win_mgr.set_state(&id, state));
        let (id, cfg, view, state) = res!(lib.fe2o3.default_help_window(&self.win_mgr.cfg));
        res!(self.win_mgr.add_window(
            Some(id.clone()),
            cfg,
            view,
        ));
        res!(self.win_mgr.set_state(&id, state));

        let (id, _) = res!(self.win_mgr.add_window(
            None,
            main_win_cfg,
            main_win,
        ));
        res!(self.win_mgr.set_state(
            &id,
            WindowStateInit {
                header: Some(lib.fe2o3.status_strip_labels_with_mode(
                    Some("/a/really/long/file/path/to/a/veryveryveryvery/loong/file/name"),
                    &id.label(),
                )),
                footer: Some(lib.fe2o3.status_strip_cursor(None::<&str>)),
                text_boxes: Some(TextBoxesState::Tabbed(res!(TabbedTextManager::new(
                    res!(lib.fe2o3.standard_tab_styles()),
                    vec![],
                    None,
                    None,
                )))),
            },
        ));
        let test_txt = [
            Text::from("1. The procrastinator's meeting has been postponed."),
            Text::from("2. The mechanic's car is always the last to get fixed."),
            Text::from("3. My Wi-Fi went down for five minutes, so I had to talk to my family. They seem like nice people."),
            Text::from("4. The fire station burned down last night."),
            Text::from("5. I always arrive late at the office, but I make up for it by leaving early."),
            Text::from("6. The plumber's house has leaky taps."),
            Text::from("7. My diet plan includes eating donuts for breakfast."),
            Text::from("8. The marriage counselor is getting a divorce."),
            Text::from("9. I told my boss I was late because I was stuck in traffic. He replied, 'At home?'"),
            Text::from("10. The vegetarian worked in a butcher shop."),
            Text::from("11. My calendar says I have no time to procrastinate today."),
            Text::from("12. The lifeguard can't swim."),
            Text::from("13. My phone bill was so high, I had to call the phone company to complain."),
            Text::from("14. I keep forgetting to go to my memory improvement class."),
            Text::from("15. The dentist has a sweet tooth."),
            Text::from("16. I quit my job to pursue my passion for unemployment."),
            Text::from("17. The tailor's clothes are always too loose or too tight."),
            Text::from("18. The librarian lost her book."),
            Text::from("19. My therapist told me to write a letter to someone who hurt me and then burn it. Now I don't know what to do with the letter!"),
            Text::from("20. The meteorologist said it would be sunny all day, but I got drenched in the rain."),
        ];
        res!(self.win_mgr.add_text_box(
            &id,
            TabbedTextBox {
                tbox: res!(TextBox::new(
                    lib.fe2o3.navigable_text_box(),
                    res!(TextView::new(
                        ContentType::Text,
                        TextViewType::Editable(Editor::default()),
                        AccessibleText::Shared(Rc::new(RwLock::new(
                            TextLines::from(test_txt)
                        ))),
                    )),
                )),
                tab: lib.fe2o3.tab_with_label(Some(Symbol::File), "File"),
            },
        ));
        res!(self.win_mgr.add_text_box(
            &id,
            TabbedTextBox {
                tbox: res!(TextBox::new(
                    lib.fe2o3.navigable_text_box(),
                    res!(TextView::new(
                        ContentType::Text,
                        TextViewType::Editable(Editor::default()),
                        AccessibleText::Shared(Rc::new(RwLock::new(
                            TextLines::from(vec![
                                Text::from("Hello World."),
                            ]),
                        ))),
                    )),
                )),
                tab: lib.fe2o3.tab_with_label(Some(Symbol::Buffer), "Buffer"),
            },
        ));

        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct ConfigLibrary {
    pub fe2o3: StyleLibrary,
}

impl ConfigLibrary {}
