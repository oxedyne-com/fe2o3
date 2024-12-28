use oxedize_fe2o3_text::string::Stringer;

use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};


pub fn test_string(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["To lines 000", "all", "string"], || {
        let input = Stringer::new(r#"[1, "two", {3: [4, "five", 6]}, [7, 8, 9]]"#);
        let expected =
r#"[
  1,
  "two",
  {
    3: [
      4,
      "five",
      6
    ]
  },
  [
    7,
    8,
    9
  ]
]"#;
        let expected_lines: Vec<_> = expected.split('\n').collect();
        //for line in &expected_lines {
        //    test!("{}", line);
        //}
        let lines = input.to_lines("  ");
        req!(lines.len(), expected_lines.len());
        for (i, line) in lines.iter().enumerate() {
            debug!("{}", line);
            req!(line, expected_lines[i]);
        }
        Ok(())
    }));

    res!(test_it(filter, &["To lines 010", "all", "string"], || {
        let input = Stringer::new(r#"(LIST|[(I32|1), (STR|"two"), (OMAP|{(I32|3): (LIST|[(I32|4), (STR|"five"), (I32|6), ]), }), (LIST|[(I32|7), (I32|8), (I32|9), ]), ])"#);
        let expected =
r#"(LIST|[
  (I32|1),
  (STR|"two"),
  (OMAP|{
    (I32|3): (LIST|[
      (I32|4),
      (STR|"five"),
      (I32|6),
    ]),
  }),
  (LIST|[
    (I32|7),
    (I32|8),
    (I32|9),
  ]),
])"#;
        let expected_lines: Vec<_> = expected.split('\n').collect();
        //for line in &expected_lines {
        //    test!("{}", line);
        //}
        let lines = input.to_lines("  ");
        req!(lines.len(), expected_lines.len());
        for (i, line) in lines.iter().enumerate() {
            debug!("{}", line);
            req!(line, expected_lines[i]);
        }
        Ok(())
    }));

    res!(test_it(filter, &["Wrap text 000", "all", "string", "wrap"], || {
        let text =
r#"He who has seen everything, I will make known (?) to the lands.
I will teach (?) about him who experienced all things,
... alike,
Anu granted him the totality of knowledge of all.
He saw the Secret, discovered the Hidden,
he brought information of (the time) before the Flood.
He went on a distant journey, pushing himself to exhaustion,
but then was brought to peace.
He carved on a stone stela all of his toils,
and built the wall of Uruk-Haven,
the wall of the sacred Eanna Temple, the holy sanctuary.
Look at its wall which gleams like copper(?),
inspect its inner wall, the likes of which no one can equal!
Take hold of the threshold stone--it dates from ancient times!
Go close to the Eanna Temple, the residence of Ishtar,
such as no later king or man ever equaled!
Go up on the wall of Uruk and walk around,
examine its foundation, inspect its brickwork thoroughly.
Is not (even the core of) the brick structure made of kiln-fired brick,
and did not the Seven Sages themselves lay out its plans?
One league city, one league palm gardens, one league lowlands, the open area(?) of the Ishtar Temple,
three leagues and the open area(?) of Uruk it (the wall) encloses.
Find the copper tablet box,
open the ... of its lock of bronze,
undo the fastening of its secret opening.
Take and read out from the lapis lazuli tablet
how Gilgamesh went through every hardship.
"#;
        let expected =
r#"He who has seen
 everything, I will
 make known (?) to
 the lands.
I will teach (?)
 about him who
 experienced all
 things,
... alike,
Anu granted him the
 totality of
 knowledge of all.
He saw the Secret,
 discovered the
 Hidden,
he brought
 information of (the
 time) before the
 Flood.
He went on a distant
 journey, pushing
 himself to
 exhaustion,
but then was brought
 to peace.
He carved on a stone
 stela all of his
 toils,
and built the wall
 of Uruk-Haven,
the wall of the
 sacred Eanna
 Temple, the holy
 sanctuary.
Look at its wall
 which gleams like
 copper(?),
inspect its inner
 wall, the likes of
 which no one can
 equal!
Take hold of the
 threshold stone--it
 dates from ancient
 times!
Go close to the
 Eanna Temple, the
 residence of
 Ishtar,
such as no later
 king or man ever
 equaled!
Go up on the wall of
 Uruk and walk
 around,
examine its
 foundation, inspect
 its brickwork
 thoroughly.
Is not (even the
 core of) the brick
 structure made of
 kiln-fired brick,
and did not the
 Seven Sages
 themselves lay out
 its plans?
One league city, one
 league palm
 gardens, one league
 lowlands, the open
 area(?) of the
 Ishtar Temple,
three leagues and
 the open area(?) of
 Uruk it (the wall)
 encloses.
Find the copper
 tablet box,
open the ... of its
 lock of bronze,
undo the fastening
 of its secret
 opening.
Take and read out
 from the lapis
 lazuli tablet
how Gilgamesh went
 through every
 hardship.
"#;
        let expected: Vec<String> = expected.lines().map(String::from).collect();
        test!("Original text:");
        for line in text.lines() {
            test!("{}", line);
        }
        let width = 20; // > 1
        let wrapped = Stringer::new(text).wrap_lines(width, Some(" "));
        test!("Wrapped to width {}:", width);
        test!("|{}|", "-".repeat(width - 2));
        for (i, line) in wrapped.iter().enumerate() {
            test!("{}", line);
            let len = line.chars().count();
            if len > width {
                return Err(err!(
                    "'{}' is {} characters wide, more than the maximum {}.",
                    line, len, width;
                Test, String, TooBig));
            }
            req!(expected[i], *line, "(L: expected, R: wrapped)");
        }
        test!("|{}|", "-".repeat(width - 2));
        Ok(())
    }));

    res!(test_it(filter, &["Truncate string 000", "all", "string", "truncate"], || {
        let tests = [
            ("abcdefg", 9, "abcdefg"),
            ("abcdefg", 8, "abcdefg"),
            ("abcdefg", 7, "abcdefg"),
            ("abcdefg", 6, "abc..."),
            ("abcdefg", 5, "ab..."),
            ("abcdefg", 4, "a..."),
            ("abcdefg", 3, ""),
            ("abcdefg", 2, ""),
            ("abcdefg", 1, ""),
        ];
        for (input, new_len, expected) in tests {
            let mut result = Stringer::new(input);
            result.fit_into(new_len, "...");
            req!(result, Stringer::new(expected), "(L: result, R: expected), new_len = {}", new_len);
        }
        Ok(())
    }));

    Ok(())
}
