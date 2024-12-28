use oxedize_fe2o3_file::{
    tree::{
        FileTree,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    test::test_it,
};


pub fn test_tree(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Display simple expanded directory test tree 000", "all", "tree", "display"], || {
        let mut tree = res!(FileTree::new("./test_tree"));
        //for line in Stringer::new(fmt!("{:?}", tree)).to_lines("  ") {
        //    debug!("{}", line);
        //}
        tree.for_all(|entry| {
            entry.set_expanded(true);
        });

        let expected = vec![
            "├── >>> A",
            "│   ├── B.txt",
            "│   ├── C.txt",
            "│   └── D.txt",
            "├── E",
            "│   ├── F.txt",
            "│   └── G",
            "│       ├── H",
            "│       │   ├── I.txt",
            "│       │   ├── J.txt",
            "│       │   └── K.txt",
            "│       └── L.txt",
            "├── M",
            "│   ├── N.txt",
            "│   └── P.txt",
            "└── Q.txt",
        ];

        let output = res!(tree.display(true));
        req!(output.len(), expected.len(), "L: output, R: expected"); 
        let mut i = 0;
        for line in res!(tree.display(true)) {
            test!("{}", line);
            req!(output[i], expected[i], "L: output, R: expected");
            i += 1;
        }

        Ok(())
    }));

    res!(test_it(filter, &["Increment, decrement focus over entire test tree 000", "all", "tree", "focus"], || {
        let mut tree = res!(FileTree::new("./test_tree"));
        //for line in Stringer::new(fmt!("{:?}", tree)).to_lines("  ") {
        //    debug!("{}", line);
        //}
        tree.for_all(|entry| {
            entry.set_expanded(true);
        });

        let expected = vec![
            "A",
            "B.txt",
            "C.txt",
            "D.txt",
            "E",
            "F.txt",
            "G",
            "H",
            "I.txt",
            "J.txt",
            "K.txt",
            "L.txt",
            "M",
            "N.txt",
            "P.txt",
            "Q.txt",
            "Q.txt",
            "Q.txt",
        ];

        for i in 0..expected.len() {
            test!(">>>>>>>>>>>>>>>>>>>");
            for line in res!(tree.display(true)) {
                test!("{}", line);
            }
            if let Some(focal_node) = tree.get_focal_node() {
                req!(focal_node.name().as_str(), expected[i], "L: result, R: expected");
            } else {
                return Err(err!(
                    "Could not obtain focal node.";
                Test, Data, Missing));
            }
            res!(tree.inc_focus());
        }

        let expected = vec![
            "A",
            "A",
            "A",
            "B.txt",
            "C.txt",
            "D.txt",
            "E",
            "F.txt",
            "G",
            "H",
            "I.txt",
            "J.txt",
            "K.txt",
            "L.txt",
            "M",
            "N.txt",
            "P.txt",
            "Q.txt",
        ];

        for i in (0..expected.len()).rev() {
            test!("<<<<<<<<<<<<<<<<<<<");
            for line in res!(tree.display(true)) {
                test!("{}", line);
            }
            if let Some(focal_node) = tree.get_focal_node() {
                req!(focal_node.name().as_str(), expected[i], "L: result, R: expected");
            } else {
                return Err(err!(
                    "Could not obtain focal node.";
                Test, Data, Missing));
            }
            res!(tree.dec_focus());
        }

        Ok(())
    }));

    Ok(())
}
