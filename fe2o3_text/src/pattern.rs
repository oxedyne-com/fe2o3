use oxedize_fe2o3_core::prelude::*;

#[derive(Clone, Debug)]
pub enum BoolOp {
    And,
    Or,
    Not,
}

#[derive(Clone, Debug)]
pub enum SacssOp {
    StartsWith(String),
    EndsWith(String),
    Contains(String),
}

#[derive(Clone, Debug)]
pub struct SacssNode {
    pub op: Option<BoolOp>,
    pub matcher_op: Option<SacssOp>,
    pub children: Vec<usize>,
    pub watching: bool,
    pub matching_indices: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Sacss {
    pub nodes: Vec<SacssNode>,
    pub buffer: String,
}

impl SacssNode {
    pub fn new_leaf(matcher_op: SacssOp) -> Self {
        SacssNode {
            op: None,
            matcher_op: Some(matcher_op),
            children: Vec::new(),
            watching: false,
            matching_indices: Vec::new(),
        }
    }

    pub fn new_branch(op: BoolOp, children: Vec<usize>) -> Self {
        SacssNode {
            op: Some(op),
            matcher_op: None,
            children,
            watching: false,
            matching_indices: Vec::new(),
        }
    }
}

/// Sacss implements the Stateful Algorithm for Composable, Streaming Search (SACSS).
impl Sacss {
    pub fn new(root: SacssNode) -> Self {
        Sacss {
            nodes: vec![root],
            buffer: String::new(),
        }
    }

    pub fn process_char(&mut self, c: char) -> Vec<(usize, usize)> {
        self.buffer.push(c);
        self.update_and_match()
    }

    fn update_and_match(&mut self) -> Vec<(usize, usize)> {
        let mut results = Vec::new();
        self.update_node(0, &mut results);
        results
    }

    fn update_node(&mut self, node_index: usize, results: &mut Vec<(usize, usize)>) {
        let buffer_len = self.buffer.len();

        if let Some(matcher_op) = &self.nodes[node_index].matcher_op.clone() {
            match matcher_op {
                SacssOp::StartsWith(pattern) => {
                    self.update_starts_with(node_index, &pattern, buffer_len, results);
                }
                // Implement other cases as needed
                _ => {}
            }
        }

        // Traverse children if it's a branch node
        let children = self.nodes[node_index].children.clone();
        for &child_index in &children {
            self.update_node(child_index, results);
        }
    }

    fn update_starts_with(&mut self, node_index: usize, pattern: &str, buffer_len: usize, results: &mut Vec<(usize, usize)>) {
        let node = &mut self.nodes[node_index];

        // Check if we need to start watching
        if !node.watching && self.buffer.ends_with(&pattern[0..1]) {
            node.watching = true;
        }

        // Check for completed matches
        if node.watching {
            if self.buffer.ends_with(pattern) {
                let start = buffer_len - pattern.len();
                results.push((start, buffer_len));
                node.matching_indices.push(start);
                node.watching = false;
            } else if !pattern.starts_with(&self.buffer[buffer_len - 1..]) {
                node.watching = false;
            }
        }

        // Check existing matching indices for potential new results
        let new_matches: Vec<(usize, usize)> = node.matching_indices
            .iter()
            .filter(|&&start| start + pattern.len() == buffer_len)
            .map(|&start| (start, buffer_len))
            .collect();
        results.extend(new_matches);
    }
}
