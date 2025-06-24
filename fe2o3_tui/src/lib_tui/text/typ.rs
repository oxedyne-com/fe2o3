use crate::lib_tui::{
    action::Action,
    draw::window::WindowId,
    style::Symbol,
    text::{
        edit::Editor,
        //highlight::StyledHighlighter,
        nav::Navigator,
    },
};

use oxedyne_fe2o3_geom::dim::Coord;
use oxedyne_fe2o3_text::Text;

use std::{
    path::PathBuf,
};


#[derive(Clone, Debug, Default)]
pub enum HighlightType {
    #[default]
    Plain,
    File(PathBuf),
    Menu(Action),
    Window(WindowId),
}

#[derive(Clone, Debug, Default)]
pub enum TextType {
    #[default]
    Plain,
    MenuItem,
    MenuHeading,
}

impl TextType {
    pub fn new_menu_item<S: Into<String>>(s: S) -> Text<Self> {
        Text::new(s, Some(Self::MenuItem))
    }
    pub fn new_menu_heading<S: Into<String>>(s: S) -> Text<Self> {
        Text::new(s, Some(Self::MenuHeading))
    }
}

#[derive(Clone, Debug, Default)]
pub enum ContentType {
    Buffer,
    Database,
    File,
    FileTree,
    Log,
    Menu,
    Shell,
    #[default]
    Text,
}

impl ContentType {

    pub fn symbol(&self) -> Option<Symbol> {
        match self {
            Self::Buffer    => Some(Symbol::Buffer),
            Self::Database  => Some(Symbol::Database),
            Self::File      => Some(Symbol::File),
            Self::FileTree  => Some(Symbol::FileTree),
            Self::Log       => None,
            Self::Menu      => Some(Symbol::Menu),
            Self::Shell     => Some(Symbol::Shell),
            Self::Text      => Some(Symbol::Text),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Buffer    => "Buffer",
            Self::Database  => "Database",
            Self::File      => "File",
            Self::FileTree  => "File Tree",
            Self::Log       => "Log",
            Self::Menu      => "Menu",
            Self::Shell     => "Shell",
            Self::Text      => "Text",
        }
    }
}

#[derive(Clone, Debug)]
pub enum TextViewType {
    Editable(Editor),
    Static(Navigator),
    Menu(Navigator),
    WindowList(Navigator),
    FileTree(Navigator),
}

impl Default for TextViewType {
    fn default() -> Self {
        Self::Static(Navigator::default())
    }
}

impl TextViewType {

    pub fn get_cursor(&self) -> Option<&Coord> {
        Some(match self {
            Self::Editable(editor) => &editor.nav.cursor,
            Self::Static(nav)       |
            Self::Menu(nav)         |
            Self::FileTree(nav)     |
            Self::WindowList(nav)   => &nav.cursor,
        })
    }

    pub fn get_cursor_mut(&mut self) -> Option<&mut Coord> {
        Some(match self {
            Self::Editable(editor) => &mut editor.nav.cursor,
            Self::Static(nav)       |
            Self::Menu(nav)         |
            Self::FileTree(nav)     |
            Self::WindowList(nav)   => &mut nav.cursor,
        })
    }

    pub fn get_editor(&self) -> Option<&Editor> {
        match self {
            Self::Editable(editor) => Some(editor),
            _ => None,
        }
    }

    pub fn get_editor_mut(&mut self) -> Option<&mut Editor> {
        match self {
            Self::Editable(editor) => Some(editor),
            _ => None,
        }
    }

    //pub fn get_highlighter(&self) -> Option<&StyledHighlighter> {
    //    match self {
    //        Self::Editable(editor) => editor.nav.highlighter.as_ref(),
    //        Self::Static(nav)           |
    //        Self::Menu(nav)             |
    //        Self::FileTree(_, nav)      |
    //        Self::WindowList(nav)       => nav.highlighter.as_ref(),
    //    }
    //}

    //pub fn get_highlighter_mut(&mut self) -> Option<&mut StyledHighlighter> {
    //    match self {
    //        Self::Editable(editor) => editor.nav.highlighter.as_mut(),
    //        Self::Static(nav)           |
    //        Self::Menu(nav)             |
    //        Self::FileTree(_, nav)      |
    //        Self::WindowList(nav)       => nav.highlighter.as_mut(),
    //    }
    //}
}
