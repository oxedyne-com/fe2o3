use crate::{
    action::Action,
    cfg::style::StyleLibrary,
    draw::{
        outline::{
            Outline,
            OutlineConfig,
        },
        tab::TabbedTextBox,
        tbox::TextBox,
        window::{
            TextBoxesState,
            Window,
            WindowConfig,
            WindowId,
            WindowStateInit,
        },
    },
    event::KeyState,
    text::{
        nav::PositionCursor,
        highlight::HighlightBuilder,
        typ::{
            HighlightType,
            TextType,
        },
    },
    render::{
        Drawer,
        Renderer,
        Sink,
        When,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    count::{
        Counter,
        CycleCounter,
    },
};
use oxedize_fe2o3_geom::{
    rect::{
        AbsSize,
        RectView,
    },
};
use oxedize_fe2o3_text::{
    Text,
    lines::{
        LineRange,
        TextLines,
    },
    highlight::{
        Highlight,
        Highlighter,
    },
};

use std::{
    collections::{
        BTreeMap,
        HashMap,
    },
    rc::Rc,
    sync::{
        Arc,
        RwLock,
    },
};


#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum WindowMode {
    #[default]
    Navigation,
    Interaction,
    WindowManagement,
    BorderManagement,
}

#[derive(Clone, Debug, Default)]
pub struct MenuItem {
    pub text:   Text<TextType>,
    pub action: Option<Action>,
}

new_type!(MenuList, Vec<MenuItem>, Clone, Debug, Default);

impl From<Vec<(Text<TextType>, Option<Action>)>> for MenuList {
    fn from(v: Vec<(Text<TextType>, Option<Action>)>) -> Self {
        let mut result = Vec::new();
        for (text, action) in v {
            result.push(MenuItem {
                text,
                action,
            });
        }
        Self(result)
    }
}

impl MenuList {
    pub fn text_lines(self) -> Outcome<TextLines<TextType, HighlightType>> {
        let lines = self.iter().map(|item| item.text.clone());
        let actions = self.iter().map(|item| match item.action {
            Some(action) => Some(HighlightType::Menu(action)),
            None => None,
        });
        let pairs = lines.clone().zip(actions).collect();
        // Zip the highlight associated data into the highlights.
        let highlights = res!(HighlightBuilder::IgnoreWhiteSpace.build(pairs));
        let highlighter = Highlighter::new(highlights, None);
        Ok(TextLines::new(lines.collect(), Some(highlighter)))
    }
}

impl WindowMode {

    pub fn build_menus() -> Outcome<HashMap<Self, TextLines<TextType, HighlightType>>> {
        let mut map = HashMap::new();
        let menu_list = MenuList::from(vec![
            (
                TextType::new_menu_item("Move to next window (Tab)                 "),
                Some(Action::MoveToNextWindow),   
            ),
            (
                TextType::new_menu_item("Enter window (Enter)                      "),
                Some(Action::EnterWindow),   
            ),
            (
                TextType::new_menu_item("Enter window management mode (Space)      "),
                Some(Action::EnterWindowManagementMode),
            ),
            (
                TextType::new_menu_item("Create new window                         "),
                Some(Action::CreateNewWindow),
            ),
        ]);
        let text_lines = res!(menu_list.text_lines());
        map.insert(
            Self::Navigation,
            text_lines,
        );
        let menu_list = MenuList::from(vec![
            (
                TextType::new_menu_item(    "Move to next tabbed text box (Tab)        "),
                Some(Action::MoveToNextTab),   
            ),
            (
                TextType::new_menu_heading( "Move cursor                               "),
                None,
            ),
            (
                TextType::new_menu_item(    " Move cursor up (Up arrow)                "),
                Some(Action::MoveCursorUp),   
            ),
            (
                TextType::new_menu_item(    " Move cursor down (Down arrow)            "),
                Some(Action::MoveCursorDown),   
            ),
            (
                TextType::new_menu_item(    " Move cursor right (Right arrow)          "),
                Some(Action::MoveCursorRight),   
            ),
            (
                TextType::new_menu_item(    " Move cursor left (Left arrow)            "),
                Some(Action::MoveCursorLeft),   
            ),
            (
                TextType::new_menu_heading( "Pan view                                  "),
                None,
            ),
            (
                TextType::new_menu_item(    " Pan view up (Shift + Up arrow)           "),
                Some(Action::PanTextViewUp),
            ),
            (
                TextType::new_menu_item(    " Pan view down (Shift + Down arrow)       "),
                Some(Action::PanTextViewDown),
            ),
            (
                TextType::new_menu_item(    " Pan view right (Shift + Right arrow)     "),
                Some(Action::PanTextViewRight),
            ),
            (
                TextType::new_menu_item(    " Pan view left (Shift + Left arrow)       "),
                Some(Action::PanTextViewLeft),
            ),
            (
                TextType::new_menu_heading( "Editor modes                              "),
                None,
            ),
            (
                TextType::new_menu_item(    " Enter editor insert mode (i)             "),
                Some(Action::EnterEditorInsertMode),
            ),
            (
                TextType::new_menu_item(    " Enter editor replace mode (r)            "),
                Some(Action::EnterEditorReplaceMode),
            ),
            (
                TextType::new_menu_item(    " Enter editor modify mode (m)             "),
                Some(Action::EnterEditorModifyMode),
            ),
            (
                TextType::new_menu_item(    "Return to editor navigation mode (Esc)    "),
                Some(Action::ReturnToEditorNavigationMode),
            ),
            (
                TextType::new_menu_item(    "Return to window navigation mode (Esc)    "),
                Some(Action::ReturnToWindowNavigationMode),
            ),
        ]);
        let text_lines = res!(menu_list.text_lines());
        map.insert(
            Self::Interaction,
            text_lines,
        );
        let menu_list = MenuList::from(vec![
            (
                TextType::new_menu_item(    "Enter border management mode (Space)      "),
                Some(Action::EnterBorderManagementMode),   
            ),
            (
                TextType::new_menu_heading( "Slide window                              "),
                None,
            ),
            (
                TextType::new_menu_item(    " Slide window up (Up arrow)               "),
                Some(Action::SlideWindowUp),   
            ),
            (
                TextType::new_menu_item(    " Slide window down (Down arrow)           "),
                Some(Action::SlideWindowDown),   
            ),
            (
                TextType::new_menu_item(    " Slide window right (Right arrow)         "),
                Some(Action::SlideWindowRight),   
            ),
            (
                TextType::new_menu_item(    " Slide window left (Left arrow)           "),
                Some(Action::SlideWindowLeft),   
            ),
            (
                TextType::new_menu_item(    "Create new window (+)                     "),
                Some(Action::CreateWindow),
            ),
            (
                TextType::new_menu_item(    "Delete window (Del)                       "),
                Some(Action::DeleteWindow),
            ),
            (
                TextType::new_menu_item(    "Return to navigation mode (Esc)           "),
                Some(Action::ReturnToWindowNavigationMode),
            ),
        ]);
        let text_lines = res!(menu_list.text_lines());
        map.insert(
            Self::WindowManagement,
            text_lines,
        );
        let menu_list = MenuList::from(vec![
            (
                TextType::new_menu_item(    "Move to next border (Space)               "),
                Some(Action::MoveToNextBorder),
            ),
            (
                TextType::new_menu_heading( "Drag border                               "),
                None,
            ),
            (
                TextType::new_menu_item(    " Drag border up (Up arrow)                "),
                Some(Action::DragBorderUp),   
            ),
            (
                TextType::new_menu_item(    " Drag border down (Down arrow)            "),
                Some(Action::DragBorderDown),   
            ),
            (
                TextType::new_menu_item(    " Drag border right (Right arrow)          "),
                Some(Action::DragBorderRight),   
            ),
            (
                TextType::new_menu_item(    " Drag border left (Left arrow)            "),
                Some(Action::DragBorderLeft),   
            ),
            (
                TextType::new_menu_item(    "Return to window management mode (Esc)    "),
                Some(Action::ReturnToWindowManagementMode),
            ),
        ]);
        let text_lines = res!(menu_list.text_lines());
        map.insert(
            Self::BorderManagement,
            text_lines,
        );
        Ok(map)
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowManagerConfig {
    pub tab_string:         String,
    pub window_count_limit: u8,
    pub window_min_size_x:  u8,
    pub window_min_size_y:  u8,
    pub selection_envelope: OutlineConfig,
}

impl WindowManagerConfig {
    pub fn window_min_size(&self) -> AbsSize {
        AbsSize::from((
            self.window_min_size_x,
            self.window_min_size_y,
        ))
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowManagerState<'a> {
    pub selection_envelope: Option<Outline<'a>>,
    pub window_list:        Rc<RwLock<TextLines<TextType, HighlightType>>>,
    pub menu_text:          Arc<RwLock<TextLines<TextType, HighlightType>>>,
    //pub menu_actions:       Vec<Action>,
    pub next_label:         Counter<u8>,
    pub new_win_pos:        (CycleCounter<usize>, CycleCounter<usize>),
}

#[derive(Clone, Debug, Default)]
pub struct WindowManager<'a> {
    pub cfg:        WindowManagerConfig,
    pub windows:    BTreeMap<WindowId, Window>,
    pub mode:       WindowMode,
    pub menus:      HashMap<WindowMode, TextLines<TextType, HighlightType>>, 
    pub focus:      WindowId,
    pub state:      WindowManagerState<'a>,
    pub key_state:  KeyState,
    pub style_lib:  StyleLibrary,
}

impl<'a> WindowManager<'a> {

    pub fn new(
        cfg: WindowManagerConfig,
    )
        -> Outcome<Self>
    {
        let next_label_counter = res!(Counter::new(3, cfg.window_count_limit, 1));
        let mut new = Self {
            cfg,
            menus: res!(WindowMode::build_menus()),
            ..Default::default()
        };
        new.state.next_label = next_label_counter;
        new.state.new_win_pos = (
            res!(CycleCounter::new(30, 100, 2)), // x
            res!(CycleCounter::new(2, 20, 2)),   // y
        );
        res!(new.set_mode_update_menu(WindowMode::Navigation, None));
        Ok(new)
    }

    pub fn next_id(&mut self) -> Outcome<WindowId> {
        if let Some(next_label) = self.state.next_label.next() {
            Ok(WindowId::User(next_label))
        } else {
            Err(err!(
                "Window count limit of {} has been reached.", self.cfg.window_count_limit;
            Excessive))
        }
    }

    pub fn add_window(
        &mut self,
        id:     Option<WindowId>,
        cfg:    WindowConfig,
        view:   RectView,
    )
        -> Outcome<(WindowId, bool)>
    {
        let id = match id {
            Some(id) => id,
            None => res!(self.next_id()),
        };
        let new_window = res!(Window::new(
            id.clone(),
            cfg,
            view,
            None,
        ));
        let old = self.windows.insert(id.clone(), new_window);
        res!(self.update_shared_window_list());
        Ok((id, old.is_some()))
    }

    pub fn set_state(
        &mut self,
        id:     &WindowId,
        state:  WindowStateInit,
    )
        -> Outcome<()>
    {
        let result = self.get_window_by_id_mut(&id);
        let win = res!(result);
        win.set_state(state);
        Ok(())
    }

    pub fn add_text_box(
        &mut self,
        id:     &WindowId,
        ttbox:  TabbedTextBox,
    )
        -> Outcome<()>
    {
        let result = self.get_window_by_id_mut(&id);
        let win = res!(result);
        match &mut win.state.text_boxes {
            TextBoxesState::Single(_tbox) => {
                return Err(err!(
                    "Cannot add text box to an existing single, untabbed container.";
                Input, Invalid, Mismatch));
            }
            TextBoxesState::Tabbed(tmgr) => {
                tmgr.tboxes.push(ttbox);
            }
        }
        Ok(())
    }

    pub fn update_shared_window_list(&mut self) -> Outcome<()> {
        // 1. Update the shared window list.
        let list = self.list_windows();
        let highlights = res!(HighlightBuilder::FullLine.build(list.clone()));
        let mut window_list_text_lines = lock_write!(self.state.window_list);
        let current_focus = if let Some(highlighter) = window_list_text_lines.get_highlighter() {
            Some(highlighter.get_focus())  
        } else {
            None
        };
        let highlighter = Highlighter::new(highlights, current_focus);
        *window_list_text_lines = TextLines::new(
            list.into_iter().map(|pair| pair.0).collect(),
            Some(highlighter),
        );
        Ok(())
    }

    /// Returns a mutating iterator over the windows in reverse order, setting
    /// the focus flag and mode for each as it goes.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Window> {
        let focus = &self.focus;
        let mode = self.mode;
        self.windows.iter_mut().rev().map(move |(id, window)| {
            window.focus = *id == *focus;
            window.mode = mode;
            window
        })
    }

    pub fn set_mode_update_menu(
        &mut self,
        mode:           WindowMode,
        menu_list_opt:  Option<MenuList>,
    )
        -> Outcome<()>
    {
        self.mode = mode;
        {
            let mut text = lock_write!(self.state.menu_text);
            if let Some(menu_list) = menu_list_opt {
                *text = res!(menu_list.text_lines());
                text.changed = true;
            } else if let Some(text_lines) = self.menus.get(&mode) {
                // Get the menu commands for the new mode, if not provided.
                *text = text_lines.clone();
                text.changed = true;
            } else {
                *text = TextLines::default();
                text.changed = true;
            }
        }
        if !self.windows.is_empty() {
            let result = self.get_focal_window();
            let win = res!(result);
            let menu_text = win.cfg.menu_text.clone();
            if let Some(menu_text) = menu_text {
                // Add commands from the focal window to the menu.
                let mut text = lock_write!(self.state.menu_text);
                (*text).append_text_lines(menu_text);
            }
        }
        // Add these commands to the end of all menu lists.
        let mut text = lock_write!(self.state.menu_text);
        let txt = "Exit Ironic (Ctlr + c)                    ";
        (*text).append_text_line(
            Text::new(txt, Some(TextType::MenuItem)),
            Some(Highlight::new(
                LineRange::new(0usize, (0usize, txt.len() - 1)),
                0,
                Some(HighlightType::Menu(Action::ExitApp)),
            )),
        );
        Ok(())
    }

    pub fn next_focus(&mut self) {
        let focus = &self.focus;
        let mut range = self.windows.range(focus..);
    
        if let Some((next_id, _)) = range.next() {
            if next_id == focus {
                if let Some((next_next_id, _)) = range.next() {
                    self.focus = next_next_id.clone();
                } else if let Some((first_id, _)) = self.windows.iter().next() {
                    self.focus = first_id.clone();
                }
            } else {
                self.focus = next_id.clone();
            }
        } else if let Some((first_id, _)) = self.windows.iter().next() {
            self.focus = first_id.clone();
        }
    }

    pub fn get_focus_id(&self) -> &WindowId {
        &self.focus
    }

    /// Set the window focus by id.
    pub fn set_focus_by_id(&mut self, id: &WindowId) -> Outcome<()> {
        if self.windows.contains_key(id) {
            self.focus = id.clone();
            Ok(())
        } else {
            Err(err!(
                "There is no window with id {:?}.", id;
            Key, NotFound))
        }
    }

    /// Set the window focus by label.
    pub fn set_focus_by_label(&mut self, label: &str) -> Outcome<()> {
        for (id, _window) in &self.windows {
            if id.label() == label {
                self.focus = id.clone();
                return Ok(());
            }
        }
        Err(err!(
            "There is no window with label '{}'.", label;
        Data, NotFound))
    }

    //pub fn list_windows(&self) -> Vec<(String, String)> {
    //    let mut result = Vec::new();
    //    for id in self.windows.keys() {
    //        result.push((id.title().to_string(), id.label().to_string()));
    //    }
    //    result
    //}

    pub fn list_windows(&self) -> Vec<(Text<TextType>, Option<HighlightType>)> {
        let mut result = Vec::new();
        for id in self.windows.keys() {
            result.push((
                Text::new(
                    fmt!("{:<10}{:20}", id.title(), id.label()),
                    Some(TextType::MenuItem),
                ),
                Some(HighlightType::Window(id.clone())),
            ));
        }
        result
    }

    pub fn get_focal_window(&self) -> Outcome<&Window> {
        if !self.windows.is_empty() {
            if let Some(window) = self.windows.get(&self.focus) {
                Ok(window)
            } else {
                Err(err!(
                    "There is no window corresponding to the focus id {:?}.", self.focus;
                Bug, Key, NotFound))
            }
        } else {
            Err(err!("There are no windows yet."; Bug, Data, Missing))
        }
    }

    pub fn get_focal_window_mut(&mut self) -> Outcome<&mut Window> {
        if !self.windows.is_empty() {
            if let Some(window) = self.windows.get_mut(&self.focus) {
                Ok(window)
            } else {
                Err(err!(
                    "There is no window corresponding to the focus id {:?}.", self.focus;
                Bug, Key, NotFound))
            }
        } else {
            Err(err!("There are no windows yet."; Bug, Data, Missing))
        }
    }

    pub fn get_window_by_id(&self, id: &WindowId) -> Outcome<&Window> {
        if let Some(window) = self.windows.get(id) {
            Ok(window)
        } else {
            Err(err!(
                "There is no window with id {:?}.", id;
            Key, NotFound))
        }
    }

    pub fn get_window_by_id_mut(&mut self, id: &WindowId) -> Outcome<&mut Window> {
        if let Some(window) = self.windows.get_mut(id) {
            Ok(window)
        } else {
            Err(err!(
                "There is no window with id {:?}.", id;
            Key, NotFound))
        }
    }

    pub fn get_focal_window_text_box_mut(&mut self) -> Outcome<Option<&mut TextBox>> {
        let result = self.get_focal_window_mut();
        let window = res!(result);
        Ok(window.get_focal_text_box_mut())
    }

    pub fn draw_cursor<S: Sink, R: Renderer<S>>(
        &mut self,
        drawer: &mut Drawer<S, R>,
        when:   When,
    )
        -> Outcome<()>
    {
        let result = self.get_focal_window_text_box_mut();
        if let Some(tbox) = res!(result) {
            match tbox.cfg.cursor_position {
                PositionCursor::UserControlled | PositionCursor::LatestLine(true) => {
                    res!(drawer.rend.set_cursor(tbox.tview.term_cursor, when));
                    if let Some(cursor_style) = tbox.cfg.cursor_style {
                        res!(drawer.rend.show_cursor(when));
                        res!(drawer.rend.set_cursor_style(cursor_style, When::Later));
                    }
                }
                _ => {
                    res!(drawer.rend.hide_cursor(when));
                }
            }
        }
        Ok(())
    }
}
