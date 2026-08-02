use oxedyne_fe2o3_tui::lib_tui::text::{
    typ::TextType,
    view::TextView,
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedyne_fe2o3_geom::{
    dim::{
        Dim,
        Span,
    },
    rect::AbsRect,
};
use oxedyne_fe2o3_text::{
    Text,
    lines::LineRange,
};


pub fn test_text(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Extract view", "all", "text", "extract"], || {
        let text: Vec<Text<TextType>> = vec![
            Text::from("The Epic of Gilgamesh"),
            Text::from("Tablet 1"),
            Text::from("He who has seen everything, I will make known (?) to the lands."),
            Text::from("I will teach (?) about him who experienced all things,"),
            Text::from("... alike,"),
            Text::from("Anu granted him the totality of knowledge of all."),
            Text::from("He saw the Secret, discovered the Hidden,"),
            Text::from("he brought information of (the time) before the Flood."),
            Text::from("He went on a distant journey, pushing himself to exhaustion,"),
            Text::from("but then was brought to peace."),
            Text::from("He carved on a stone stela all of his toils,"),
            Text::from("and built the wall of Uruk-Haven,"),
            Text::from("the wall of the sacred Eanna Temple, the holy sanctuary."),
            Text::from("Look at its wall which gleams like copper(?),"),
            Text::from("inspect its inner wall, the likes of which no one can equal!"),
            Text::from("Take hold of the threshold stone--it dates from ancient times!"),
            Text::from("Go close to the Eanna Temple, the residence of Ishtar,"),
            Text::from("such as no later king or man ever equaled!"),
            Text::from("Go up on the wall of Uruk and walk around,"),
            Text::from("examine its foundation, inspect its brickwork thoroughly."),
            Text::from("Is not (even the core of) the brick structure made of kiln-fired brick,"),
            Text::from("and did not the Seven Sages themselves lay out its plans?"),
            Text::from("One league city, one league palm gardens, one league lowlands, the open area(?) of the Ishtar Temple,"),
            Text::from("three leagues and the open area(?) of Uruk it (the wall) encloses."),
            Text::from("Find the copper tablet box,"),
            Text::from("open the ... of its lock of bronze,"),
            Text::from("undo the fastening of its secret opening."),
            Text::from("Take and read out from the lapis lazuli tablet"),
            Text::from("how Gilgamesh went through every hardship."),
        ];
        let expected = vec![
            fmt!("who has see"),
            fmt!("ill teach ("), 
            fmt!(" alike,"),
            fmt!(" granted hi"),
            fmt!("saw the Sec"),
        ];
        let text_view = AbsRect::from((Dim(3), Dim(2), Dim(11), Dim(5)));
        debug!("text_view = {:?}", text_view);
        let lines = TextView::extract_view(
            &text,
            &text_view,
        );
        for (i, line) in lines.iter().enumerate() {
            debug!("line {} = '{}'", i, line);
        }
        req!(lines.len(), expected.len(), "(L: result len, R: expected len)");
        for (i, line) in lines.iter().enumerate() {
            req!(*line, expected[i], "(L: result line, R: expected line)");
        }
        Ok(())
    }));

    // Test the clipping of a rectangle of height 1 by a view.
    //
    // Not to scale.
    // +---------------------------------------+
    // |  (2,1)              (12,1)            |
    // |    +------------------+               |
    // |    |      5           |               |
    // |    |  +=======+       |               |
    // |    | (3,4)            |               |
    // |    |                  | 7             |
    // |    |        10        |               |
    // |    +------------------+               |
    // |                   view                |
    // |                                       |
    // |                                       |
    // |                                       |
    // +---------------------------------------+
    //
    res!(test_it(filter, &["Relative text range 000", "all", "text", "range"], || {
        let range = LineRange {
            line: Dim(4),
            span: Span::new((Dim(3), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(3), Dim(4), Dim(5), Dim(1)));
        req!(Some(expected), result, "L: expected, R: result");
        Ok(())
    }));

    // Not to scale.
    // +---------------------------------------+
    // |  (2,1)              (12,1)            |
    // |    +------------------+               |
    // |    |               4  |               |
    // |    |           +======|x              |
    // |    |         (8,4)    |               |
    // |    |                  | 7             |
    // |    |        10        |               |
    // |    +------------------+               |
    // |                   view                |
    // |                                       |
    // |                                       |
    // |                                       |
    // +---------------------------------------+
    //
    res!(test_it(filter, &["Relative text range 010", "all", "text", "range"], || {
        let range = LineRange {
            line: Dim(4),
            span: Span::new((Dim(8), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(8), Dim(4), Dim(4), Dim(1)));
        req!(Some(expected), result, "L: expected, R: result");
        Ok(())
    }));

    // Not to scale.
    // +---------------------------------------+
    // |  (2,1)              (12,1)            |
    // |    +------------------+               |
    // |    |                1 |               |
    // |    |                 +|xxxxxxx        |
    // |    |            (11,4)|               |
    // |    |                  | 7             |
    // |    |        10        |               |
    // |    +------------------+               |
    // |                   view                |
    // |                                       |
    // |                                       |
    // |                                       |
    // +---------------------------------------+
    //
    res!(test_it(filter, &["Relative text range 020", "all", "text", "range"], || {
        let range = LineRange {
            line: Dim(4),
            span: Span::new((Dim(11), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(11), Dim(4), Dim(1), Dim(1)));
        req!(Some(expected), result, "L: expected, R: result");
        Ok(())
    }));

    // Not to scale.
    // +---------------------------------------+
    // |  (2,1)              (12,1)            |
    // |    +------------------+               |
    // |    |                0 |               |
    // |    |                  +xxxxxxxx       |
    // |    |            (12,4)|               |
    // |    |                  | 7             |
    // |    |        10        |               |
    // |    +------------------+               |
    // |                   view                |
    // |                                       |
    // |                                       |
    // |                                       |
    // +---------------------------------------+
    //
    res!(test_it(filter, &["Relative text range 030", "all", "text", "range"], || {
        let range = LineRange {
            line: Dim(4),
            span: Span::new((Dim(12), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        // A range that starts at the far edge of the view has nothing inside it. An earlier
        // fe2o3_geom answered with a rectangle of zero width; `Span::clip` now answers `None`,
        // which is what the caller in `draw/tbox.rs` matches on when it decides whether there is
        // anything to paint.
        let expected: Option<AbsRect> = None;
        req!(expected, result, "L: expected, R: result");
        Ok(())
    }));

    // Not to scale.
    // +---------------------------------------+
    // |  (2,1)              (12,1)            |
    // |    +------------------+               |
    // |    |                0 |               |
    // |    |                  |xxxxxxxxx      |
    // |    |            (12,4)|               |
    // |    |                  | 7             |
    // |    |        10        |               |
    // |    +------------------+               |
    // |                   view                |
    // |                                       |
    // |                                       |
    // |                                       |
    // +---------------------------------------+
    //
    res!(test_it(filter, &["Relative text range 040", "all", "text", "range"], || {
        let range = LineRange {
            line: Dim(4),
            span: Span::new((Dim(13), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        // A range that starts at the far edge of the view has nothing inside it. An earlier
        // fe2o3_geom answered with a rectangle of zero width; `Span::clip` now answers `None`,
        // which is what the caller in `draw/tbox.rs` matches on when it decides whether there is
        // anything to paint.
        let expected: Option<AbsRect> = None;
        req!(expected, result, "L: expected, R: result");
        Ok(())
    }));

    // Not to scale.
    // +---------------------------------------+
    // |  (2,1)              (12,1)            |
    // |    +------------------+               |
    // |    |   5              |               |
    // |    +=======+          |               |
    // |    |(2,4)             |               |
    // |    |                  | 7             |
    // |    |        10        |               |
    // |    +------------------+               |
    // |                   view                |
    // |                                       |
    // |                                       |
    // |                                       |
    // +---------------------------------------+
    //
    res!(test_it(filter, &["Relative text range 050", "all", "text", "range"], || {
        let range = LineRange {
            line: Dim(4),
            span: Span::new((Dim(2), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(2), Dim(4), Dim(5), Dim(1)));
        req!(Some(expected), result, "L: expected, R: result");
        Ok(())
    }));

    // Not to scale.
    // +---------------------------------------+
    // |  (2,1)              (12,1)            |
    // |    +------------------+               |
    // |    |  4               |               |
    // |  xx+=====+            |               |
    // |    |(2,4)             |               |
    // |    |                  | 7             |
    // |    |        10        |               |
    // |    +------------------+               |
    // |                   view                |
    // |                                       |
    // |                                       |
    // |                                       |
    // +---------------------------------------+
    //
    res!(test_it(filter, &["Relative text range 060", "all", "text", "range"], || {
        let range = LineRange {
            line: Dim(4),
            span: Span::new((Dim(1), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(2), Dim(4), Dim(4), Dim(1)));
        req!(Some(expected), result, "L: expected, R: result");
        Ok(())
    }));

    Ok(())
}
