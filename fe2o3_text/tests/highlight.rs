use oxedyne_fe2o3_text::{
    Text,
    highlight::{
        Highlight,
    },
    lines::{
        LineRange,
        TextLines,
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedyne_fe2o3_geom::{
    dim::Coord,
};


#[derive(Clone, Debug, Default)]
enum TextType {
    #[default]
    A,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
enum HighlightData<'a> {
    X(&'a str),
}

impl<'a> Default for HighlightData<'a> {
    fn default() -> Self {
        Self::X("")
    }
}

pub fn test_highlight(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Highlights 000", "all", "highlight"], || {
        let original_text = [
        //   00000000001111111111222  
        //   01234567890123456789012
            "This is the first line.",
            "Make your selection:",
            "      a. Do this",
            "      c. Do that other",
            "      d. Do nothing",
        ];
        let mut tlines = TextLines::<TextType, HighlightData>::default();
        tlines.append_text_line(
            Text::new(original_text[0], None),
            Some(Highlight::new(
                LineRange::new(0usize, (12usize, 16)),
                13,
                None,
            )),
        );
        tlines.append_text_line(
            Text::new(original_text[1], None),
            None,
        );
        tlines.append_text_line(
            Text::new(original_text[2], Some(TextType::A)),
            Some(Highlight::new(
                LineRange::new(0usize, (9usize, 15)),
                0,
                Some(HighlightData::X("this")),
            )),
        );
        tlines.append_text_line(
            Text::new(original_text[3], Some(TextType::A)),
            Some(Highlight::new(
                LineRange::new(0usize, (9usize, 21)),
                0,
                Some(HighlightData::X("that other")),
            )),
        );
        tlines.append_text_line(
            Text::new(original_text[4], Some(TextType::A)),
            Some(Highlight::new(
                LineRange::new(0usize, (9usize, 18)),
                0,
                Some(HighlightData::X("nothing")),
            )),
        );

        // When each text was added, the associated highlight line should have incremented.
        let expected_text = original_text.clone();
        let expected_highlight_lines = [0usize, 2, 3, 4];
        test!("Original text lines:");
        req!(expected_text.len(), tlines.len(), "(L: expected, R: actual)");
        for (i, line) in tlines.lines.iter().enumerate() {
            test!("{:02}: '{}'", i, line);
            req!(expected_text[i], line.txt.as_str());
        }
        test!("Original highlights:");
        if let Some(highlighter) = &tlines.highlighter {
            req!(expected_highlight_lines.len(), highlighter.len(), "(L: expected, R: actual)");
            for (i, highlight) in highlighter.ranges.iter().enumerate() {
                test!("{:?}", highlight);
                req!(expected_highlight_lines[i], highlight.range.line, "(L: expected, R: actual)");
            }
        }

        // Break first line inside the associated highlight.  This should delete the highlight and
        // increment the line for the remainder.
        tlines.enter_new_line(&mut Coord::from((14usize, 0)));
        let expected_text = [
            "This is the fi",
            "rst line.",
            "Make your selection:",
            "      a. Do this",
            "      c. Do that other",
            "      d. Do nothing",
        ];
        let expected_highlight_lines = [3, 4, 5];
        test!("Break first line:");
        req!(expected_text.len(), tlines.len(), "(L: expected, R: actual)");
        for (i, line) in tlines.lines.iter().enumerate() {
            test!("{:02}: '{}'", i, line);
            req!(expected_text[i], line.txt.as_str());
        }
        test!("Resulting highlights:");
        if let Some(highlighter) = &tlines.highlighter {
            req!(expected_highlight_lines.len(), highlighter.len(), "(L: expected, R: actual)");
            for (i, highlight) in highlighter.ranges.iter().enumerate() {
                test!("{:?}", highlight);
                req!(expected_highlight_lines[i], highlight.range.line, "(L: expected, R: actual)");
            }
        }

        // Rejoin the first line inside the associated highlight.  This should delete the highlight and
        // increment the line for the remainder.
        tlines.delete_char(&mut Coord::from((14usize, 0)));
        let expected_text = original_text.clone();
        let expected_highlight_lines = [2, 3, 4];
        test!("Rejoin first line:");
        req!(expected_text.len(), tlines.len(), "(L: expected, R: actual)");
        for (i, line) in tlines.lines.iter().enumerate() {
            test!("{:02}: '{}'", i, line);
            req!(expected_text[i], line.txt.as_str());
        }
        test!("Resulting highlights:");
        if let Some(highlighter) = &tlines.highlighter {
            req!(expected_highlight_lines.len(), highlighter.len(), "(L: expected, R: actual)");
            for (i, highlight) in highlighter.ranges.iter().enumerate() {
                test!("{:?}", highlight);
                req!(expected_highlight_lines[i], highlight.range.line, "(L: expected, R: actual)");
            }
        }

        // Insert a new line of text.
        //             00000000001111111111222  
        //             01234567890123456789012
        let new_txt = "      b. Forgot this";
        tlines.insert_text_line(
            2,
            Text::new(new_txt, Some(TextType::A)),
            Some(Highlight::new(
                LineRange::new(0usize, (9usize, 19)),
                0,
                Some(HighlightData::X("forgot this")),
            )),
        );
        let expected_text = [
            "This is the first line.",
            "Make your selection:",
            "      a. Do this",
            new_txt,
            "      c. Do that other",
            "      d. Do nothing",
        ];
        let expected_highlight_lines = [2, 3, 4, 5];
        test!("Insert new line:");
        req!(expected_text.len(), tlines.len(), "(L: expected, R: actual)");
        for (i, line) in tlines.lines.iter().enumerate() {
            test!("{:02}: '{}'", i, line);
            req!(expected_text[i], line.txt.as_str());
        }
        test!("Resulting highlights:");
        if let Some(highlighter) = &tlines.highlighter {
            req!(expected_highlight_lines.len(), highlighter.len(), "(L: expected, R: actual)");
            for (i, highlight) in highlighter.ranges.iter().enumerate() {
                test!("{:?}", highlight);
                req!(expected_highlight_lines[i], highlight.range.line, "(L: expected, R: actual)");
            }
        }

        // Delete the inserted line.
        tlines.remove_line(3);
        let expected_text = original_text;
        let expected_highlight_lines = [2, 3, 4];
        test!("Delete inserted line:");
        req!(expected_text.len(), tlines.len(), "(L: expected, R: actual)");
        for (i, line) in tlines.lines.iter().enumerate() {
            test!("{:02}: '{}'", i, line);
            req!(expected_text[i], line.txt.as_str());
        }
        test!("Resulting highlights:");
        if let Some(highlighter) = &tlines.highlighter {
            req!(expected_highlight_lines.len(), highlighter.len(), "(L: expected, R: actual)");
            for (i, highlight) in highlighter.ranges.iter().enumerate() {
                test!("{:?}", highlight);
                req!(expected_highlight_lines[i], highlight.range.line, "(L: expected, R: actual)");
            }
        }

        // Append another TextLines.
        let mut tlines2 = TextLines::<TextType, HighlightData>::default();
        let original_text2 = [
        //   00000000001111111111222  
        //   01234567890123456789012
            "      e. One more thing",
            "      f. And another",
        ];
        tlines2.append_text_line(
            Text::new(original_text2[0], None),
            Some(Highlight::new(
                LineRange::new(0usize, (9usize, 22)),
                0,
                Some(HighlightData::X("one more thing")),
            )),
        );
        tlines2.append_text_line(
            Text::new(original_text2[1], None),
            Some(Highlight::new(
                LineRange::new(0usize, (9usize, 19)),
                0,
                Some(HighlightData::X("and another")),
            )),
        );
        tlines.append_text_lines(tlines2);
        let mut expected_text = original_text.to_vec();
        for line in original_text2 {
            expected_text.push(line);
        }
        let expected_highlight_lines = [2, 3, 4, 5, 6];
        test!("Append another TextLines:");
        req!(expected_text.len(), tlines.len(), "(L: expected, R: actual)");
        for (i, line) in tlines.lines.iter().enumerate() {
            test!("{:02}: '{}'", i, line);
            req!(expected_text[i], line.txt.as_str());
        }
        test!("Resulting highlights:");
        if let Some(highlighter) = &tlines.highlighter {
            req!(expected_highlight_lines.len(), highlighter.len(), "(L: expected, R: actual)");
            for (i, highlight) in highlighter.ranges.iter().enumerate() {
                test!("{:?}", highlight);
                req!(expected_highlight_lines[i], highlight.range.line, "(L: expected, R: actual)");
            }
        }

        Ok(())
    }));


    Ok(())
}
