use oxedize_fe2o3_core::{
    prelude::*,
    count::ErrorWhen,
};

use std::{
    cmp::Ordering,
    fmt,
};


#[derive(Clone, Copy, Debug, Default)]
pub enum SortBy {
    #[default]
    Name,
    Path,
    ModifiedTime,
    Size,
}

pub trait NodeData: Clone + fmt::Debug + Eq + PartialEq + Ord + PartialOrd {}

#[derive(Clone, Debug)]
pub struct Leaf<D: NodeData> {
    pub name:       String,
    pub data:       D,
    pub focus:      bool,
    pub selected:   bool,
}

impl<D: NodeData> Leaf<D> {
    pub fn name(&self) -> &String {
        &self.name
    }
    pub fn data(&self) -> &D {
        &self.data
    }
}

#[derive(Clone, Debug)]
pub struct Branch<D: NodeData> {
    pub name:       String,
    pub data:       D,
    pub nodes:      Vec<Node<D>>,
    pub expanded:   bool,
    pub focus:      bool,
    pub selected:   bool,
}

impl<D: NodeData> Branch<D> {
    pub fn name(&self) -> &String {
        &self.name
    }
    pub fn data(&self) -> &D {
        &self.data
    }
}

#[derive(Clone, Debug, Default)]
pub struct NodeProperties {
    is_branch:      bool,
    has_parent:     bool,
    has_children:   bool,
    num_children:   usize,
    num_siblings:   usize,
}

#[derive(Clone, Debug)]
pub enum Node<D: NodeData> {
    Leaf(Leaf<D>),
    Branch(Branch<D>),
}

impl<D: NodeData> Node<D> {

    pub fn name(&self) -> &String {
        match self {
            Node::Leaf(Leaf { name, .. }) => &name,
            Node::Branch(Branch { name, .. }) => &name,
        }
    }

    pub fn data(&self) -> &D {
        match self {
            Node::Leaf(Leaf { data, .. }) => &data,
            Node::Branch(Branch { data, .. }) => &data,
        }
    }

    pub fn is_expanded(&self) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Branch(branch) => branch.expanded,
        }
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        if let Node::Branch(branch) = self {
            branch.expanded = expanded;
        }
    }

    pub fn set_node_focus(&mut self, focus: bool) {
        match self {
            Node::Leaf(leaf) => leaf.focus = focus,
            Node::Branch(branch) => branch.focus = focus,
        }
    }

    pub fn set_selected(&mut self, selected: bool) {
        match self {
            Node::Leaf(leaf) => leaf.selected = selected,
            Node::Branch(branch) => branch.selected = selected,
        }
    }

    pub fn is_selected(&self) -> bool {
        match self {
            Node::Leaf(leaf) => leaf.selected,
            Node::Branch(branch) => branch.selected,
        }
    }

    pub fn is_focused(&self) -> bool {
        match self {
            Node::Leaf(leaf) => leaf.focus,
            Node::Branch(branch) => branch.focus,
        }
    }
}

pub fn sort_nodes<
    D: NodeData,
    F: Fn(&D, &D) -> Ordering,
>(
    nodes:      &mut Vec<Node<D>>,
    compare:    &F,
) {
    nodes.sort_by(|a, b| compare(a.data(), b.data()))
}

pub fn sort_nodes_by_name<D: NodeData>(nodes: &mut Vec<Node<D>>) {
    nodes.sort_by(|a, b| a.name().cmp(b.name()))
}

#[derive(Clone, Debug, Default)]
pub struct Tree<R, D: NodeData> {
    pub root:       R,
    pub focus_path: Vec<usize>,
    pub nodes:      Vec<Node<D>>,
    pub max_depth:  usize,
}

impl<R, D: NodeData> Tree<R, D> {

    pub fn new(
        root:       R,
        nodes:      Vec<Node<D>>,
        max_depth:  usize,
    )
        -> Self
    {
        Self {
            root,
            focus_path: if !nodes.is_empty() {
                vec![0]
            } else {
                Vec::new()
            },
            nodes,
            max_depth,
        }
    }

    pub fn for_all<F>(&mut self, mut callback: F)
    where
        F: FnMut(&mut Node<D>),
    {
        Self::for_all_recursive(&mut self.nodes, &mut callback);
    }

    fn for_all_recursive<F>(nodes: &mut [Node<D>], callback: &mut F)
    where
        F: FnMut(&mut Node<D>),
    {
        for node in nodes.iter_mut() {
            callback(node);
            if let Node::Branch(branch) = node {
                Self::for_all_recursive(&mut branch.nodes, callback);
            }
        }
    }

    pub fn get_node<'a>(
        &'a self,
        path: &'a [usize],
    )
        -> Option<&'a Node<D>>
    {
        let mut nodes = &self.nodes;
        let imax = path.len().saturating_sub(1);
        for (i, &index) in path.iter().enumerate() {
            if i >= imax {
                return nodes.get(index);
            } else {
                nodes = match nodes.get(index) {
                    Some(Node::Branch(branch)) => &branch.nodes,
                    Some(node) => return Some(node),
                    None => return None,
                };
            }
        }
        nodes.get(path.last().cloned().unwrap_or(0))
    }
    
    pub fn get_focal_node<'a>(&'a self) -> Option<&'a Node<D>> {
        let mut nodes = &self.nodes;
        let imax = self.focus_path.len().saturating_sub(1);
        for (i, &index) in self.focus_path.iter().enumerate() {
            if i >= imax {
                return nodes.get(index);
            } else {
                nodes = match nodes.get(index) {
                    Some(Node::Branch(branch)) => &branch.nodes,
                    Some(node) => return Some(node),
                    None => return None,
                };
            }
        }
        nodes.get(self.focus_path.last().cloned().unwrap_or(0))
    }
    
    pub fn get_node_mut<'a>(
        &'a mut self,
        path: &'a [usize],
    )
        -> Option<&'a mut Node<D>>
    {
        let mut nodes = &mut self.nodes;
        let imax = path.len().saturating_sub(1);
        for (i, &index) in path.iter().enumerate() {
            if i >= imax {
                return nodes.get_mut(index);
            } else {
                nodes = match nodes.get_mut(index) {
                    Some(Node::Branch(branch)) => &mut branch.nodes,
                    Some(node) => return Some(node),
                    None => return None,
                };
            }
        }
        nodes.get_mut(path.last().cloned().unwrap_or(0))
    }
    
    fn get_focal_node_mut(&mut self) -> Option<&mut Node<D>> {
        let mut nodes = &mut self.nodes;
        let imax = self.focus_path.len().saturating_sub(1);
        for (i, &index) in self.focus_path.iter().enumerate() {
            if i >= imax {
                return nodes.get_mut(index);
            } else {
                nodes = match nodes.get_mut(index) {
                    Some(Node::Branch(branch)) => &mut branch.nodes,
                    Some(node) => return Some(node),
                    None => return None,
                };
            }
        }
        nodes.get_mut(self.focus_path.last().cloned().unwrap_or(0))
    }

    pub fn get_sibling_count(&self, path: &[usize]) -> Option<usize> {
        if path.is_empty() {
            Some(self.nodes.len())
        } else {
            let parent_path = &path[..path.len() - 1];
            if let Some(parent_node) = self.get_node(parent_path) {
                match parent_node {
                    Node::Branch(branch) => Some(branch.nodes.len() - 1),
                    Node::Leaf(_) => None,
                }
            } else {
                None
            }
        }
    }

    pub fn has_next_sibling(&self, path: &[usize], index: usize) -> bool {
        if let Some(Node::Branch(branch)) = self.get_node(&path) {
            if index >= branch.nodes.len() {
                false
            } else {
                true
            }
        } else {
            false
        }
    }

    pub fn has_children(&self, path: &[usize]) -> bool {
        if let Some(Node::Branch(branch)) = self.get_node(&path) {
            if branch.nodes.is_empty() {
                false
            } else {
                true
            }
        } else {
            false
        }
    }

    pub fn is_branch(&self, path: &[usize]) -> bool {
        if let Some(Node::Branch(_branch)) = self.get_node(&path) {
            true
        } else {
            false
        }
    }

    pub fn get_properties(&self, path: &[usize]) -> NodeProperties {

        let mut is_branch = false;
        let mut has_parent = false;
        let mut has_children = false;
        let mut num_children = 0;
        let mut num_siblings = 0;

        if let Some(Node::Branch(branch)) = self.get_node(&path) {
            is_branch = true;
            if branch.nodes.len() > 0 {
                has_children = true;
            }
            num_children = branch.nodes.len();
        }

        if path.len() == 1 {
            num_siblings = self.nodes.len().saturating_sub(1); // Don't count the focus node.
        } else if path.len() > 1 {
            if let Some(node) = self.get_node(&path[..(path.len() - 1)]) {
                has_parent = true;
                if let Node::Branch(branch) = node {
                    num_siblings = branch.nodes.len().saturating_sub(1); // Don't count the focus node.
                }
            }
        }

        NodeProperties {
            is_branch,
            has_parent,
            has_children,
            num_children,
            num_siblings,
        }
    }

    // Sort by provided closure.
    
    pub fn sort<F: Fn(&D, &D) -> Ordering>(&mut self, compare: F) {
        self.sort_nodes(&compare);
        self.sort_child_nodes(compare);
    }

    fn sort_nodes<F: Fn(&D, &D) -> Ordering>(&mut self, compare: F) {
        sort_nodes(&mut self.nodes, &compare);
    }

    fn sort_child_nodes<F: Fn(&D, &D) -> Ordering>(&mut self, compare: F) {
        for node in &mut self.nodes {
            if let Node::Branch(branch) = node {
                sort_nodes(&mut branch.nodes, &compare);
                Self::sort_child_nodes_recursive(&mut branch.nodes, &compare);
            }
        }
    }

    fn sort_child_nodes_recursive<
        F: Fn(&D, &D) -> Ordering
    >(
        nodes:      &mut Vec<Node<D>>,
        compare:    &F,
    ) {
        for node in nodes {
            if let Node::Branch(branch) = node {
                sort_nodes(&mut branch.nodes, compare);
                Self::sort_child_nodes_recursive(&mut branch.nodes, compare);
            }
        }
    }

    // Sort by name.
    
    pub fn sort_by_name(&mut self) {
        self.sort_nodes_by_name();
        self.sort_child_nodes_by_name();
    }

    fn sort_nodes_by_name(&mut self) {
        sort_nodes_by_name(&mut self.nodes);
    }

    fn sort_child_nodes_by_name(&mut self) {
        for node in &mut self.nodes {
            if let Node::Branch(branch) = node {
                sort_nodes_by_name(&mut branch.nodes);
                Self::sort_child_nodes_by_name_recursive(&mut branch.nodes);
            }
        }
    }

    fn sort_child_nodes_by_name_recursive(nodes: &mut Vec<Node<D>>) {
        for node in nodes {
            if let Node::Branch(branch) = node {
                sort_nodes_by_name(&mut branch.nodes);
                Self::sort_child_nodes_by_name_recursive(&mut branch.nodes);
            }
        }
    }

    pub fn display(&self, lines: bool) -> Outcome<Vec<String>> {
        let mut output = Vec::new();
        res!(Self::display_nodes(
            &self.nodes,
            0,
            lines,
            &mut output,
            &mut Vec::new(),
        ));
        Ok(output)
    }

    pub fn display_nodes(
        nodes:              &[Node<D>],
        depth:              usize,
        lines:              bool,
        output:             &mut Vec<String>,
        is_last_at_level:   &mut Vec<bool>,
    )
        -> Outcome<()>
    {
        for (index, node) in nodes.iter().enumerate() {
            let is_last = index == nodes.len() - 1;
            let prefix = if lines {
                if is_last {
                    "└── "
                } else {
                    "├── "
                }
            } else {
                "  "
            };
    
            let mut line_prefix = String::new();
            for i in 0..depth {
                if is_last_at_level[i] {
                    line_prefix.push_str("    ");
                } else {
                    line_prefix.push_str("│   ");
                }
            }
    
            match node {
                Node::Leaf(leaf) => {
                    let focus_prefix = if leaf.focus { ">>> " } else { "" };
                    let selected_prefix = if leaf.selected { "[*] " } else { "" };
                    output.push(format!(
                        "{}{}{}{}{}",
                        line_prefix,
                        prefix,
                        focus_prefix,
                        selected_prefix,
                        leaf.name(),
                    ));
                }
                Node::Branch(branch) => {
                    let focus_prefix = if branch.focus { ">>> " } else { "" };
                    let selected_prefix = if branch.selected { "[*] " } else { "" };
                    output.push(format!(
                        "{}{}{}{}{}",
                        line_prefix,
                        prefix,
                        focus_prefix,
                        selected_prefix,
                        branch.name(),
                    ));
                    if branch.expanded {
                        if is_last_at_level.len() <= depth {
                            is_last_at_level.push(is_last);
                        } else {
                            is_last_at_level[depth] = is_last;
                        }
                        res!(Self::display_nodes(
                            &branch.nodes,
                            depth + 1,
                            lines,
                            output,
                            is_last_at_level,
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn inc_focus(&mut self) -> Outcome<()> {

        let mut path = self.focus_path.clone();
        if self.get_node(&path).is_none() {
            return Ok(());
        }

        let mut ascending = false;
        let mut safety = ErrorWhen::new(self.max_depth);
        loop {
            res!(safety.inc());
            let len = path.len();
            if len > 0 {
                let last = len - 1;
                let props = self.get_properties(&path);
                let index = path[last];
                if props.is_branch {
                    if props.has_children && !ascending {
                        // Focus goes to first child.
                        path.push(0);
                        break;
                    } else {
                        if index < props.num_siblings {
                            // Focus goes to next sibling.
                            path[last] += 1;
                            break;
                        } else {
                            // There is no next sibiling.
                            if props.has_parent {
                                path = path[..last].to_vec();
                                ascending = true;
                                continue;
                            } else {
                                return Ok(());
                            }
                        }
                    }
                } else {
                    // Node is a leaf.
                    if index < props.num_siblings {
                        // Focus goes to next sibling.
                        path[last] += 1;
                        break;
                    } else {
                        // There is no next sibling. Begin ascent back up branch.
                        if props.has_parent {
                            path = path[..last].to_vec();
                            ascending = true;
                            continue;
                        } else {
                            return Ok(());
                        }
                    }
                }
            } else {
                break;   
            }
        }
    
        res!(self.set_node_focus(false));
        self.focus_path = path;
        res!(self.set_node_focus(true));
    
        Ok(())
    }
    
    pub fn dec_focus(&mut self) -> Outcome<()> {

        let mut path = self.focus_path.clone();
        if self.get_node(&path).is_none() {
            return Ok(());
        }

        let mut descending = false;
        let mut safety = ErrorWhen::new(self.max_depth);
        loop {
            res!(safety.inc());
            let len = path.len();
            if len > 0 {
                let last = len - 1;
                let props = self.get_properties(&path);
                let index = path[last];
                if descending {
                    // Slippery slide to the bottom.
                    if props.has_children {
                        // Descend to bottom of the level and try to keep going.
                        path.push(props.num_children - 1);
                        continue;
                    } else {
                        // We've reached the bottom, a branch with no children.  Make this the
                        // focus.
                        break;
                    }
                } else {
                    if props.is_branch {
                        if index > 0 {
                            // Focus goes to previous sibling.
                            path[last] -= 1;
                            descending = true;
                            continue;
                        } else {
                            // There is no previous sibiling.
                            if props.has_parent {
                                path = path[..last].to_vec();
                                break;
                            } else {
                                return Ok(());
                            }
                        }
                    } else {
                        // Node is a leaf.
                        if index > 0 {
                            // Focus on previous sibling.
                            path[last] -= 1;
                            descending = true;
                            continue;
                        } else {
                            // There is no previous sibling.
                            if props.has_parent {
                                path = path[..last].to_vec();
                                break;
                            } else {
                                return Ok(());
                            }
                        }
                    }
                }
            } else {
                break;   
            }
        }
    
        res!(self.set_node_focus(false));
        self.focus_path = path;
        res!(self.set_node_focus(true));
    
        Ok(())
    }

    pub fn set_node_focus(&mut self, focus: bool) -> Outcome<()> {

        if let Some(current) = self.get_focal_node_mut() {
            current.set_node_focus(focus);
        } else {
            return Err(err!(
                "Could not find a tree node matching the current focus \
                with path {:?}.", self.focus_path;
            Data, Missing));
        }

        Ok(())
    }

    pub fn toggle_selection(&mut self) -> Outcome<()> {
        if let Some(node) = self.nodes.iter_mut().find(|node| node.is_focused()) {
            node.set_selected(!node.is_selected());
        }
        Ok(())
    }
}
