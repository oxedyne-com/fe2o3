use oxedize_fe2o3_core::{
    prelude::*,
};
use oxedize_fe2o3_data::tree::{
    Branch,
    Leaf,
    Node,
    NodeData,
    Tree,
};

use std::{
    fs,
    path::{
        Path,
        PathBuf,
    },
    time::SystemTime,
};


#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Attributes {
    path:       PathBuf,
    mod_time:   SystemTime,
    size:       u64,
}

impl NodeData for Attributes {}

new_type!(FileTree, Tree<PathBuf, Attributes>, Clone, Debug);

impl FileTree {

    pub fn new<P: AsRef<Path>>(path: P) -> Outcome<Self> {
        let root = path.as_ref().to_path_buf();
        let nodes = res!(Self::read_nodes(&root));
        // Max theoretical directory depth estimates resulting from path length limits:
        // Linux: 2047,
        // Windows: 129 (default),
        // MacOS: 507
        let mut tree = Tree::new(
            root,
            nodes,
            2048,
        );
        tree.sort_by_name();
        res!(tree.set_node_focus(true));
        Ok(Self(tree))
    }

    pub fn read_nodes(
        path: &Path,
    )
        -> Outcome<Vec<Node<Attributes>>>
    {
        let mut nodes = Vec::new();

        for entry_result in res!(fs::read_dir(path), IO, File) {
            let entry = res!(entry_result, IO, File);
            let node_type = res!(entry.file_type(), IO, File);
            let node_path = entry.path();
            let metadata = res!(entry.metadata(), IO, File);
            let size = metadata.len();
            let mod_time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

            let node_name = match entry.file_name().to_str() {
                Some(name) => name.to_string(),
                None => String::from(""),
            };

            let data = Attributes {
                path: node_path.clone(),
                size,
                mod_time,
            };
            if node_type.is_file() {
                nodes.push(Node::Leaf(Leaf {
                    name:       node_name,
                    data,
                    focus:      false,
                    selected:   false,
                }));
            } else if node_type.is_dir() {
                let subdir_nodes = res!(Self::read_nodes(&node_path));
                nodes.push(Node::Branch(Branch {
                    name:       node_name,
                    data,
                    nodes:      subdir_nodes,
                    expanded:   false,
                    focus:      false,
                    selected:   false,
                }));
            }
        }

        Ok(nodes)
    }
}
