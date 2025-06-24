use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedyne_fe2o3_text::string::Stringer;


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

    Ok(())
}
