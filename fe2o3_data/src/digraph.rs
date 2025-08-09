//! Directed graph data structure with typed nodes and links.
//! 
//! This module provides a generic directed graph implementation where nodes
//! and links can carry arbitrary data. The graph supports efficient lookups
//! and traversals through HashMap-based storage.

use std::{
    collections::HashMap,
    fmt,
    hash::Hash,
};

use rayon::prelude::*;

/// Trait for types that can serve as node identifiers in the graph.
/// 
/// Implementors must be cloneable, debuggable, equatable, and hashable
/// to work as HashMap keys.
pub trait NodeId: Clone + fmt::Debug + Eq + Hash + Send + Sync {}

/// Trait for data stored within graph nodes.
/// 
/// Node data must be cloneable and debuggable for graph operations.
pub trait NodeData: Clone + fmt::Debug + Send + Sync {}

/// Trait for data stored on graph links (edges).
/// 
/// Link data must be cloneable and debuggable for graph operations.
pub trait LinkData: Clone + fmt::Debug + Send + Sync {}

/// A node in the directed graph.
/// 
/// Each node contains its associated data and a list of outgoing links
/// to other nodes in the graph.
#[derive(Clone, Debug)]
pub struct Node<ID: NodeId, ND: NodeData, LD: LinkData> {
    /// Outgoing links from this node.
    links:  Vec<Link<ID, LD>>,
    /// Data associated with this node.
    data:   ND,
}

/// A directed link (edge) between nodes.
/// 
/// Each link carries optional data and points to a target node.
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct Link<ID: NodeId, LD: LinkData> {
    /// Data associated with this link.
    data:   LD,
    /// Target node identifier.
    to:     ID,
}

/// A directed graph with typed nodes and links.
/// 
/// The graph uses a HashMap for O(1) node lookups by identifier.
/// Nodes can have arbitrary data attached, as can the links between them.
/// 
/// # Type Parameters
/// 
/// * `ID` - The type used for node identifiers.
/// * `ND` - The type of data stored in nodes.
/// * `LD` - The type of data stored on links.
#[derive(Clone, Debug)]
pub struct DiGraph<ID: NodeId, ND: NodeData, LD: LinkData> {
    /// HashMap storing all nodes indexed by their identifiers.
    nodes:  HashMap<ID, Node<ID, ND, LD>>,
}

impl<ID: NodeId, ND: NodeData, LD: LinkData> DiGraph<ID, ND, LD> {

    /// Creates a new, empty directed graph.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use oxedyne_fe2o3_data::digraph::DiGraph;
    /// 
    /// let graph: DiGraph<u32, String, ()> = DiGraph::new();
    /// ```
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Inserts a new node into the graph.
    /// 
    /// Returns the previous node if one existed with the same identifier.
    /// 
    /// # Arguments
    /// 
    /// * `id` - The unique identifier for the node.
    /// * `data` - The data to store in the node.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use oxedyne_fe2o3_data::digraph::DiGraph;
    /// use oxedyne_fe2o3_core::prelude::*;
    /// 
    /// let mut graph = DiGraph::new();
    /// graph.insert(1, fmt!("Node A"));
    /// graph.insert(2, fmt!("Node B"));
    /// ```
    pub fn insert(&mut self, id: ID, data: ND) -> Option<Node<ID, ND, LD>> {
        let node = Node {
            links: Vec::new(),
            data,
        };
        self.nodes.insert(id, node)
    }
    
    /// Creates a directed link between two nodes.
    /// 
    /// If the source node doesn't exist, this operation is silently ignored.
    /// The target node doesn't need to exist for the link to be created.
    /// 
    /// # Arguments
    /// 
    /// * `from` - The identifier of the source node.
    /// * `to` - The identifier of the target node.
    /// * `data` - The data to associate with the link.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use oxedyne_fe2o3_data::digraph::DiGraph;
    /// use oxedyne_fe2o3_core::prelude::*;
    /// 
    /// let mut graph = DiGraph::new();
    /// graph.insert(1, fmt!("Node A"));
    /// graph.insert(2, fmt!("Node B"));
    /// graph.link(&1, &2, fmt!("Edge A->B"));
    /// ```
    pub fn link(&mut self, from: &ID, to: &ID, data: LD) {
        if let Some(from_node) = self.nodes.get_mut(from) {
            from_node.links.push(Link { data, to: to.clone() });
        }
    }

    /// Finds all nodes matching a predicate on their data.
    /// 
    /// Returns a vector of node identifiers that match the given criteria.
    /// This is the most memory-efficient option when you only need the identifiers.
    /// 
    /// # Arguments
    /// 
    /// * `predicate` - A closure that returns `true` for matching node data.
    /// 
    /// # Returns
    /// 
    /// A vector containing the identifiers of all matching nodes.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use oxedyne_fe2o3_data::digraph::DiGraph;
    /// 
    /// let mut graph = DiGraph::new();
    /// graph.insert(1, 100);
    /// graph.insert(2, 200);
    /// graph.insert(3, 150);
    /// 
    /// // Find all nodes with values greater than 120.
    /// let large_nodes = graph.find_nodes(|&value| value > 120);
    /// assert_eq!(large_nodes.len(), 2);
    /// ```
    pub fn find_nodes<F>(&self, predicate: F) -> Vec<ID>
    where
        F: Fn(&ND) -> bool,
    {
        self.nodes
            .iter()
            .filter_map(|(id, node)| {
                if predicate(&node.data) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Finds all nodes matching a predicate, returning both identifiers and data.
    /// 
    /// This method is more efficient than `find_nodes` when you need both the
    /// identifier and the data, as it avoids a second lookup operation.
    /// 
    /// # Arguments
    /// 
    /// * `predicate` - A closure that returns `true` for matching node data.
    /// 
    /// # Returns
    /// 
    /// A vector of tuples containing the identifier and a reference to the data
    /// for each matching node.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use oxedyne_fe2o3_data::digraph::DiGraph;
    /// use oxedyne_fe2o3_core::prelude::*;
    /// 
    /// #[derive(Clone, Debug)]
    /// struct Person {
    ///     name: String,
    ///     age: u32,
    /// }
    /// 
    /// let mut graph = DiGraph::new();
    /// graph.insert(1, Person { name: fmt!("Alice"), age: 30 });
    /// graph.insert(2, Person { name: fmt!("Bob"), age: 25 });
    /// graph.insert(3, Person { name: fmt!("Charlie"), age: 35 });
    /// 
    /// // Find all people aged 30 or over.
    /// let adults = graph.find_nodes_with_data(|person| person.age >= 30);
    /// for (id, person) in adults {
    ///     println!("Person {}: {} is {} years old", id, person.name, person.age);
    /// }
    /// ```
    pub fn find_nodes_with_data<F>(&self, predicate: F) -> Vec<(ID, &ND)>
    where
        F: Fn(&ND) -> bool,
    {
        self.nodes
            .iter()
            .filter_map(|(id, node)| {
                if predicate(&node.data) {
                    Some((id.clone(), &node.data))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Finds all nodes matching a predicate using parallel processing.
    /// 
    /// This method parallelises the search across CPU cores for better performance
    /// with large graphs.
    /// 
    /// # Arguments
    /// 
    /// * `predicate` - A closure that returns `true` for matching node data.
    /// 
    /// # Returns
    /// 
    /// A vector containing the identifiers of all matching nodes.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use oxedyne_fe2o3_data::digraph::DiGraph;
    /// 
    /// let mut graph = DiGraph::new();
    /// for i in 0..10000 {
    ///     graph.insert(i, i * 2);
    /// }
    /// 
    /// // Find all nodes with values greater than 5000 in parallel.
    /// let large_nodes = graph.find_nodes_par(|&value| value > 5000);
    /// ```
    pub fn find_nodes_par<F>(&self, predicate: F) -> Vec<ID>
    where
        F: Fn(&ND) -> bool + Sync + Send,
    {
        self.nodes
            .iter()
            .collect::<Vec<_>>()
            .par_iter()
            .filter_map(|(id, node)| {
                if predicate(&node.data) {
                    Some((*id).clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Finds all nodes matching a predicate using parallel processing, returning both identifiers and data.
    /// 
    /// This method parallelises the search and returns both the identifier and
    /// data reference for matched nodes.
    /// 
    /// # Arguments
    /// 
    /// * `predicate` - A closure that returns `true` for matching node data.
    /// 
    /// # Returns
    /// 
    /// A vector of tuples containing the identifier and a reference to the data
    /// for each matching node.
    /// 
    /// # Examples
    /// 
    /// ```
    /// use oxedyne_fe2o3_data::digraph::DiGraph;
    /// use oxedyne_fe2o3_core::prelude::*;
    /// 
    /// #[derive(Clone, Debug)]
    /// struct Sensor {
    ///     name: String,
    ///     temperature: f64,
    /// }
    /// 
    /// let mut graph = DiGraph::new();
    /// for i in 0..10000 {
    ///     graph.insert(i, Sensor { 
    ///         name: fmt!("Sensor_{}", i), 
    ///         temperature: (i as f64) * 0.1 
    ///     });
    /// }
    /// 
    /// // Find all sensors with high temperature readings in parallel.
    /// let hot_sensors = graph.find_nodes_with_data_par(|sensor| sensor.temperature > 500.0);
    /// println!("Found {} sensors with high temperature", hot_sensors.len());
    /// ```
    pub fn find_nodes_with_data_par<F>(&self, predicate: F) -> Vec<(ID, &ND)>
    where
        F: Fn(&ND) -> bool + Sync + Send,
    {
        self.nodes
            .iter()
            .collect::<Vec<_>>()
            .par_iter()
            .filter_map(|(id, node)| {
                if predicate(&node.data) {
                    Some(((*id).clone(), &node.data))
                } else {
                    None
                }
            })
            .collect()
    }
}
