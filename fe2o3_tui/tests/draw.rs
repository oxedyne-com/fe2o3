use oxedyne_fe2o3_tui::{
    dim::{
        AbsRect,
        Dim,
        Span,
    },
    draw::text::{
        LineRange,
        TextView, 
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};


pub fn test_text(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Extract view", "all", "text", "extract"], || {
        let text = vec![
            fmt!("The Epic of Gilgamesh"),
            fmt!("Tablet 1"),
            fmt!("He who has seen everything, I will make known (?) to the lands."),
            fmt!("I will teach (?) about him who experienced all things,"),
            fmt!("... alike,"),
            fmt!("Anu granted him the totality of knowledge of all."),
            fmt!("He saw the Secret, discovered the Hidden,"),
            fmt!("he brought information of (the time) before the Flood."),
            fmt!("He went on a distant journey, pushing himself to exhaustion,"),
            fmt!("but then was brought to peace."),
            fmt!("He carved on a stone stela all of his toils,"),
            fmt!("and built the wall of Uruk-Haven,"),
            fmt!("the wall of the sacred Eanna Temple, the holy sanctuary."),
            fmt!("Look at its wall which gleams like copper(?),"),
            fmt!("inspect its inner wall, the likes of which no one can equal!"),
            fmt!("Take hold of the threshold stone--it dates from ancient times!"),
            fmt!("Go close to the Eanna Temple, the residence of Ishtar,"),
            fmt!("such as no later king or man ever equaled!"),
            fmt!("Go up on the wall of Uruk and walk around,"),
            fmt!("examine its foundation, inspect its brickwork thoroughly."),
            fmt!("Is not (even the core of) the brick structure made of kiln-fired brick,"),
            fmt!("and did not the Seven Sages themselves lay out its plans?"),
            fmt!("One league city, one league palm gardens, one league lowlands, the open area(?) of the Ishtar Temple,"),
            fmt!("three leagues and the open area(?) of Uruk it (the wall) encloses."),
            fmt!("Find the copper tablet box,"),
            fmt!("open the ... of its lock of bronze,"),
            fmt!("undo the fastening of its secret opening."),
            fmt!("Take and read out from the lapis lazuli tablet"),
            fmt!("how Gilgamesh went through every hardship."),
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
            range: Span::new((Dim(3), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(3), Dim(4), Dim(5), Dim(1)));
        req!(expected, result, "L: expected, R: result");
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
            range: Span::new((Dim(8), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(8), Dim(4), Dim(4), Dim(1)));
        req!(expected, result, "L: expected, R: result");
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
            range: Span::new((Dim(11), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(11), Dim(4), Dim(1), Dim(1)));
        req!(expected, result, "L: expected, R: result");
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
            range: Span::new((Dim(12), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(12), Dim(4), Dim(0), Dim(1)));
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
            range: Span::new((Dim(13), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(12), Dim(4), Dim(0), Dim(1)));
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
            range: Span::new((Dim(2), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(2), Dim(4), Dim(5), Dim(1)));
        req!(expected, result, "L: expected, R: result");
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
            range: Span::new((Dim(1), Dim(5))),
        };
        let view = AbsRect::from((Dim(2), Dim(1), Dim(10), Dim(7)));
        let result = view.clip(range.to_abs_rect());
        let expected = AbsRect::from((Dim(2), Dim(4), Dim(4), Dim(1)));
        req!(expected, result, "L: expected, R: result");
        Ok(())
    }));

    Ok(())
}
