use crate::lib_tui::{
    text::{
        nav::PositionCursor,
        typ::{
            ContentType,
            HighlightType,
            TextType,
            TextViewType,
        },
    },
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_geom::{
    dim::Coord,
    rect::{
        AbsRect,
        AbsSize,
    },
};
use oxedyne_fe2o3_text::{
    Text,
    access::AccessibleText,
};


/// Contains access to the source text and keeps track of view metrics.  The text extent is
/// represented by a rectangle of `AbsSize` where the height is the current number of lines and the
/// width is the character length of the longest line.
#[derive(Clone, Debug, Default)]
pub struct TextView {
    pub ctyp:           ContentType,
    pub vtyp:           TextViewType,
    pub atext:          AccessibleText<TextType, HighlightType>,
    pub extent:         AbsSize,    // A dynamic rectangle representing the extent of the text.
    pub term_view:      AbsRect,    // Current terminal view.
    pub text_view:      AbsRect,    // Current text view.
    pub term_cursor:    Coord,      // Cursor position in terminal coordinates.
}

impl TextView {

    pub fn new(
        ctyp:   ContentType,
        vtyp:   TextViewType,
        //atext:  Option<AccessibleText<TextType, HighlightType>>,
        atext:  AccessibleText<TextType, HighlightType>,
    )
        -> Outcome<Self>
    {
        let text_lines = res!(atext.get_text_lines());
        //if let Some(highlighter) = text_lines.get_highlighter_mut() {
        //    highlighter.build(text_lines);
        //}
        let extent = AbsSize::from((text_lines.max_width(), text_lines.len()));
        //match vtyp {
        //    TextViewType::FileTree(ftree, _nav) => {
        //        if atext.is_none() {
        //            let txt = ftree.display(true)
        //                        AccessibleText::Shared(Rc::new(RwLock::new(
        //                            TextLines::new(
        //                                vec![], 
        //                                Some(Highlighter::default()),
        //                            ),
        //                        ))),
        //        }
        //    }
        //    _ => {}
        //}
        Ok(Self {
            ctyp,
            vtyp,
            atext: atext.clone(),
            extent,
            term_view:      AbsRect::default(),
            text_view:      AbsRect::default(),
            term_cursor:    Coord::default(),
            //editor,
            //line_focus_mgr,
        })
    }

    //pub fn get_highlighter<'a>(&'a self) -> Outcome<Option<&'a Highlighter<HighlightType>>> {
    //    let text_lines = res!(self.atext.get_text_lines());
    //    let result = text_lines.get_highlighter();
    //    Ok(text_lines.get_highlighter())
    //    //match self.atext {
    //    //    AccessibleText::ThreadShared(ref locked) => {
    //    //        let text_lines = lock_read!(locked);
    //    //        Ok(text_lines.get_highlighter())
    //    //    }
    //    //    AccessibleText::Shared(ref locked) => {
    //    //        let text_lines = lock_read!(locked);
    //    //        Ok(text_lines.get_highlighter())
    //    //    }
    //    //}
    //}

    //pub fn get_highlighter_mut(&mut self) -> Outcome<Option<&mut Highlighter<HighlightType>>> {
    //    let text_lines = res!(self.atext.get_text_lines_mut());
    //    Ok(text_lines.get_highlighter_mut())
    //    //match self.atext {
    //    //    AccessibleText::ThreadShared(ref locked) => {
    //    //        let text_lines = lock_write!(locked);
    //    //        Ok(text_lines.get_highlighter_mut())
    //    //    }
    //    //    AccessibleText::Shared(ref locked) => {
    //    //        let text_lines = lock_write!(locked);
    //    //        Ok(text_lines.get_highlighter_mut())
    //    //    }
    //    //}
    //}

    /// Extract the text view from the text.
    ///
    ///  +----------------------------+
    ///  |                            | text
    ///  |                            |
    ///  |                            |
    ///  |        x       w     x+w   |
    ///  |        +--------------+    |
    ///  |    len |              |    |
    ///  |<-------|--->| line    |    |
    ///  |        |              |    |
    ///  |        |              |    |
    ///  |        +--------------+    |
    ///  |                   text     |
    ///  |                   view     |
    ///  |                            |
    ///  |                            |
    ///  +----------------------------+
    ///
    pub fn extract_view<'a>(
        lines:      &'a [Text<TextType>],
        text_view:  &AbsRect,
    )
        -> Vec<&'a str>
    {
        let (x, y, w, h) = text_view.tup();
        let mut result = Vec::new();
        let len = lines.len();
        let rng = if len < y {
            return result;
        } else if len > y + h {
            y.as_index()..(y + h).as_index()
        } else {
            y.as_index()..len
        };
    
        for line in &lines[rng] {
            let chars: Vec<char> = line.txt.chars().collect();
            let len = chars.len();
            if len <= x {
                result.push("");
            } else if len > x + w {
                let start = line.txt.char_indices().nth(x.as_index()).map(|(i, _)| i).unwrap_or(0);
                let end = line.txt.char_indices().nth((x + w).as_index())
                    .map(|(i, _)| i).unwrap_or(line.txt.len());
                result.push(&line.txt[start..end]);
            } else {
                let start = line.txt.char_indices().nth(x.as_index()).map(|(i, _)| i).unwrap_or(0);
                result.push(&line.txt[start..]);
            }
        }
    
        result
    }

    pub fn update_view(
        &mut self,
        outer:              &AbsRect,
        position_cursor:    &PositionCursor,
    )
        -> Outcome<()>
    {
        // Update the extent.
        {
            let result = self.atext.get_text_lines_mut();
            let text_lines = res!(result);
            self.extent = AbsSize::new((text_lines.max_width(), text_lines.len()));
        }
        // The terminal view may have resized.
        self.term_view = outer.clone();
        self.text_view = AbsRect::new(self.text_view.top_left, self.term_view.size);
        match position_cursor {
            PositionCursor::LatestLine(_) => {
                if let Some(cursor) = self.vtyp.get_cursor_mut() {
                    cursor.y = self.extent.y;
                }
            }
            _ => {}
        }
        self.keep_cursor_in_view();
        Ok(())
    }

    /// If for some reason, such as a large text insertion, the cursor finds itself outside the
    /// text view rectangle, this method normalises the situation by moving the text view.
    pub fn keep_cursor_in_view(&mut self) {
        let (mut xt, mut yt, wt, ht) = self.text_view.tup();
        if let Some(cursor) = self.vtyp.get_cursor() {
            if cursor.y < yt {
                yt = cursor.y;  
            } else if cursor.y > yt + ht {
                yt = cursor.y - ht + 1;  
            }
            if cursor.x < xt {
                xt = cursor.x;  
            } else if cursor.x > xt + wt {
                xt = cursor.x - wt;  
            }
            self.text_view.top_left = Coord::new((xt, yt));
            self.term_cursor = self.term_view.top_left
                + (*cursor - self.text_view.top_left);
        }
    }

    pub fn update_cursor(&mut self) {
        if let Some(cursor) = self.vtyp.get_cursor() {
            self.term_cursor = self.term_view.top_left
                + (*cursor - self.text_view.top_left);
        }
    }

    pub fn max_width(&self) -> Outcome<usize> {
        let text_lines = res!(self.atext.get_text_lines());
        Ok(text_lines.max_width())
    }
}
