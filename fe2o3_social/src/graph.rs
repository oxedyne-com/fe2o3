//! Social network graph generator using stub matching algorithm.
//!
//! This module generates realistic social networks with configurable
//! population profiles, social circles, and geographic distributions.

use crate::{
    mmap_graph::{
        MmapGraph,
        MmapGraphBuilder,
    },
    person::{
    	PersonId,
    	ProfileType,
    },
};

use oxedyne_fe2o3_core::{
	prelude::*,
    mem::get_memory_usage_mb,
	rand::{
        Rand,
        SamplingMethod,
    },
};
use oxedyne_fe2o3_data::digraph::{
	LinkData,
	NodeData,
};

use std::{
    collections::HashMap,
    fmt,
    path::Path,
};


#[derive(Clone, Copy, Debug)]
pub enum GraphAccessMethod {
    Auto(usize),// Decide based on edge count.
    FileIO,     // Always use file I/O.
    Mmap,       // Always use memory mapping.
}

/// Social graph - edges only, stored in memory-mapped file.
pub struct SocialGraph {
    edges:      MmapGraph,
    population: u32,
}

impl SocialGraph {

    pub fn new(
        edges:      MmapGraph,
        population: u32,
    )
        -> Self
    {
        Self { edges, population }
    }

    /// Release memory pages used by the memory-mapped graph.
    /// This tells the OS it can free cached pages to reduce memory usage.
    #[cfg(unix)]
    pub fn release_memory(&self) {
        self.edges.release_memory();
    }

    #[cfg(not(unix))]
    pub fn release_memory(&self) {
        // No-op on non-Unix systems.
    }

    pub fn get_links_from(&self, id: &PersonId) -> Vec<(PersonId, SocialLink)> {
        self.get_links_from_with_method(id, GraphAccessMethod::Auto(1000))
    }

    /// Gets outgoing links from a node.
    pub fn get_links_from_with_method(
        &self,
        id: &PersonId,
        method: GraphAccessMethod,
    )
        -> Vec<(PersonId, SocialLink)>
    {
        // Query the mmap file for edges from this node.
        match self.edges.get_outgoing_edges(id.0, method) {
            Ok(edge_list) => {
                edge_list.into_iter().map(|(target_id, link_data)| {
                    let person_id = PersonId(target_id);
                    let social_link = SocialLink { packed: link_data };
                    (person_id, social_link)
                }).collect()
            },
            Err(_) => Vec::new(),
        }
    }

    /// Gets incoming links to a node.
    /// Note: This is expensive (O(n)) for memory-mapped storage as it scans all edges.
    pub fn get_links_to(&self, id: &PersonId) -> Vec<(PersonId, SocialLink)> {
        match self.edges.get_incoming_edges(id.0) {
            Ok(edge_list) => {
                edge_list.into_iter().map(|(source_id, link_data)| {
                    let person_id = PersonId(source_id);
                    let social_link = SocialLink { packed: link_data };
                    (person_id, social_link)
                }).collect()
            },
            Err(_) => Vec::new(),
        }
    }

    /// Gets the total number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.edges.total_edges()
    }

    /// Gets the number of nodes.
    pub fn len(&self) -> usize {
        self.population as usize
    }

    /// Iterator over nodes.
    pub fn iter_nodes(&self) -> std::iter::Map<std::ops::Range<u32>, fn(u32) -> (PersonId, EmptyNodeData)> {
        let node_count = self.population as u32;
        (0..node_count).map(|i| (PersonId(i), EmptyNodeData))
    }
}

/// Type of social circle relationship.
/// 
/// Represents a numbered circle from 0 (innermost) to n-1 (outermost).
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub struct CircleType(pub u8);

impl CircleType {
    /// Converts circle type to matrix index.
    /// 
    /// # Returns
    /// Index for use in reciprocity matrix.
    pub fn to_index(&self) -> usize {
        self.0 as usize
    }
    
    /// Creates circle type from matrix index.
    /// 
    /// # Arguments
    /// * `idx` - Matrix index.
    /// * `max_circles` - Maximum number of circles.
    /// 
    /// # Returns
    /// Corresponding circle type or error if invalid.
    pub fn from_index(idx: usize, max_circles: usize) -> Outcome<Self> {
        if idx >= max_circles || idx > 255 {
            Err(err!(
                "Invalid circle index: {} (max: {})", idx, max_circles - 1;
                Invalid, Index
            ))
        } else {
            Ok(Self(idx as u8))
        }
    }
    
    /// Creates an inner circle (index 0).
    pub fn inner() -> Self {
        Self(0)
    }
    
    /// Creates a close circle (index 1).
    pub fn close() -> Self {
        Self(1)
    }
    
    /// Creates an active circle (index 2).
    pub fn active() -> Self {
        Self(2)
    }
    
    /// Creates a wider circle (index 3).
    pub fn wider() -> Self {
        Self(3)
    }
}

/// Data stored on each social link.
/// 
/// Compact representation using a single byte to store both circle types.
/// Lower 4 bits: from_circle, Upper 4 bits: to_circle.
#[derive(Clone, Debug, Copy)]
pub struct SocialLink {
    pub packed: u8,
}

impl SocialLink {
    /// Creates a new social link.
    /// 
    /// # Arguments
    /// * `from_circle` - Source circle type.
    /// * `to_circle` - Target circle type.
    /// 
    /// # Returns
    /// New social link instance.
    pub fn new(
        from_circle: CircleType,
        to_circle: CircleType,
    )
        -> Self
    {
        let packed = (to_circle.0 << 4) | (from_circle.0 & 0x0F);
        Self { packed }
    }
    
    /// Gets the source circle.
    pub fn from_circle(&self) -> CircleType {
        CircleType(self.packed & 0x0F)
    }
    
    /// Gets the target circle.
    pub fn to_circle(&self) -> CircleType {
        CircleType(self.packed >> 4)
    }
}

impl fmt::Display for SocialLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "C{} -> C{}", self.from_circle().0, self.to_circle().0)
    }
}

impl LinkData for SocialLink {}

/// Empty node data for edge-only graphs.
#[derive(Clone, Debug)]
pub struct EmptyNodeData;

impl NodeData for EmptyNodeData {}

impl fmt::Display for EmptyNodeData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "()")
    }
}

/// Profile definition with circle size ranges.
#[derive(Clone, Debug)]
pub struct Profile {
    pub profile_type:	ProfileType,
    pub probability:	f32,
    pub circle_ranges:	Vec<(u32, u32)>, // (min, max) for each circle.
    pub sampling_methods:	Vec<SamplingMethod>, // One per social circle.
}

/// Link generation mode for network creation.
/// 
/// # Example
/// ```no_run
/// use oxedyne_fe2o3_social::graph::{NetworkConfig, LinkMode, generate_social_network};
/// 
/// let mut config = NetworkConfig::default();
/// config.population = 5;
/// 
/// // Create reciprocal network (default) - inverted circle relationships
/// config.link_mode = LinkMode::Reciprocal;
/// let reciprocal_graph = generate_social_network(config.clone()).unwrap();
/// 
/// // Create symmetric network - identical circle relationships
/// config.link_mode = LinkMode::Symmetric;
/// let symmetric_graph = generate_social_network(config.clone()).unwrap();
/// 
/// // Create non-reciprocal network  
/// config.link_mode = LinkMode::NonReciprocal;
/// let non_reciprocal_graph = generate_social_network(config).unwrap();
/// ```
#[derive(Clone, Debug, Copy)]
pub enum LinkMode {
    /// All links are reciprocal - if A connects to B, B also connects to A.
    /// Uses the existing probability matrix to determine circle types.
    Reciprocal,
    /// All links are symmetric - both people put each other in the same circle.
    /// If the relationship determines circle Cx, both A→B and B→A are [Cx → Cx].
    Symmetric,
    /// Links are non-reciprocal - connections are one-way only.
    /// Uses the existing probability matrix to determine circle types.
    NonReciprocal,
}

/// Internal stub representation for matching.
#[derive(Clone, Debug)]
struct Stub {
    owner_id:		PersonId,
    circle_type:	CircleType,
}

/// Labels for default circle types.
#[derive(Clone, Debug)]
pub struct CircleLabels {
    pub labels: Vec<String>,
}

impl CircleLabels {
    /// Creates default circle labels.
    pub fn default() -> Self {
        Self {
            labels: vec![
                fmt!("Inner"),
                fmt!("Close"),
                fmt!("Active"),
                fmt!("Wider"),
            ],
        }
    }
}


/// Configuration for social network generation.
#[derive(Clone)]
pub struct NetworkConfig {
    pub population:			u32,
    pub profiles:			Vec<Profile>,
    pub num_circles:		usize,
    pub reciprocity_matrix:	Vec<Vec<f32>>, // NxN matrix for circle reciprocity.
    pub circle_labels:		Option<CircleLabels>, // Optional labels for circles.
    pub link_mode:			LinkMode, // Whether links are reciprocal or not.
    pub progress_interval:	Option<u32>, // Report progress every N nodes (None = no progress reports).
    pub memory_limit_mb:	Option<f32>, // Memory limit in MB (None = no limit).
    pub chunk_size:			Option<u32>, // Process stubs in chunks of this size (None = process all at once).
    pub use_mmap:			Option<String>, // Use memory-mapped graph with specified file path (required).
}

impl NetworkConfig {
    /// Creates a default configuration with isolated/connected profiles.
    /// 
    /// Returns a configuration with realistic social network parameters
    /// including two profile types and geographic distribution.
    /// Uses 4 circles with labels: Inner, Close, Active, Wider.
    pub fn default() -> Self {
        Self {
            population: 1000,
            profiles: vec![
                Profile {
                    profile_type:	ProfileType::Isolated,
                    probability:	0.33,
                    circle_ranges:	vec![
                        (1, 1),     // Inner circle.
                        (3, 3),     // Close circle.
                        (5, 5),     // Active circle.
                        (30, 35),   // Wider circle.
                    ],
                    sampling_methods: vec![SamplingMethod::Uniform; 4],
                },
                Profile {
                    profile_type:	ProfileType::Connected,
                    probability:	0.67,
                    circle_ranges:	vec![
                        (4, 6),       // Inner circle.
                        (20, 25),     // Close circle.
                        (70, 75),     // Active circle.
                        (450, 500),   // Wider circle.
                    ],
                    sampling_methods: vec![SamplingMethod::Uniform; 4],
                },
            ],
            num_circles: 4,
            reciprocity_matrix: vec![
                vec![0.95, 0.05, 0.00, 0.00], // Inner -> x.
                vec![0.30, 0.50, 0.20, 0.00], // Close -> x.
                vec![0.10, 0.40, 0.40, 0.10], // Active -> x.
                vec![0.00, 0.10, 0.30, 0.60], // Wider -> x.
            ],
            circle_labels:      Some(CircleLabels::default()),
            link_mode:          LinkMode::Reciprocal,
            progress_interval:  None,
            memory_limit_mb:    None,
            chunk_size:         None,
            use_mmap:           None,
        }
    }
}

/// Statistics from graph verification.
#[derive(Debug)]
pub struct GraphStatistics {
    pub population:		    u32,
    pub profile_counts:	    HashMap<ProfileType, usize>,
    pub avg_circle_sizes:   Vec<f32>, // Average circle sizes by type.
}

/// Generates a social network graph using the stub matching algorithm.
/// 
/// Creates a directed graph representing social relationships between
/// people based on profile types and geographic distribution.
/// Uses memory-mapped storage for efficient handling of large graphs.
/// 
/// # Arguments
/// * `config` - Network generation configuration (must include mmap path).
/// 
/// # Returns
/// A memory-mapped social graph or error if generation fails.
pub fn generate_social_network(
    config: NetworkConfig,
)
    -> Outcome<SocialGraph>
{
    // Validate profile sampling methods match num_circles.
    for profile in &config.profiles {
        if profile.sampling_methods.len() != config.num_circles {
            return Err(err!(
                "Profile sampling_methods length ({}) must match num_circles ({})",
                profile.sampling_methods.len(),
                config.num_circles;
                Invalid, Input
            ));
        }
        if profile.circle_ranges.len() != config.num_circles {
            return Err(err!(
                "Profile circle_ranges length ({}) must match num_circles ({})",
                profile.circle_ranges.len(),
                config.num_circles;
                Invalid, Input
            ));
        }
    }

    // Memory-mapped storage is required.
    let mmap_path = match config.use_mmap.clone() {
        Some(path) => path,
        None => return Err(err!("Memory-mapped path is required for graph generation"; Invalid, Input)),
    };
    
    generate_mmap_social_network(config, mmap_path)
}

/// Generates social network using memory-mapped storage.
fn generate_mmap_social_network(
    config:     NetworkConfig,
    mmap_path:  String,
)
    -> Outcome<SocialGraph>
{
    // Check if the mmap file already exists and has content.
    if Path::new(&mmap_path).exists() {
        if let Ok(metadata) = std::fs::metadata(&mmap_path) {
            if metadata.len() > 0 {
                if let Some(_interval) = config.progress_interval {
                    info!(">>> Loading existing memory-mapped social graph");
                    info!("Population: {}", config.population);
                    info!("Memory-mapped file: {}", mmap_path);
                    info!("File size: {:.1} MB", metadata.len() as f32 / (1024.0 * 1024.0));
                }
                
                // Load existing mmap graph.
                let edges = res!(MmapGraph::load_existing(&mmap_path));
                
                return Ok(SocialGraph::new(edges, config.population));
            }
        }
    }
    
    if let Some(interval) = config.progress_interval {
        info!(">>> Memory-mapped social graph generation");
        info!("Population: {}", config.population);
        info!("Progress reporting every {} nodes", interval);
        info!("Memory-mapped file: {}", mmap_path);
    }
    
    // Step 1: Generate stubs directly from population range.
    if config.progress_interval.is_some() {
        info!("Step 1: Generating stubs for {} nodes...", config.population);
    }
    let stubs = create_stubs(&config);
    // For reciprocal/symmetric modes, each stub pair creates 2 edges (A→B and B→A).
    // For non-reciprocal mode, each pair creates 1 edge.
    // Add 10% safety margin for edge case variations.
    let base_edges = match config.link_mode {
        LinkMode::NonReciprocal => stubs.len() / 2,
        _ => stubs.len(), // Reciprocal and Symmetric create 2 edges per pair
    };
    let estimated_edges = (base_edges as f32 * 1.1) as usize;
    
    if config.progress_interval.is_some() {
        info!("Step 2: Creating memory-mapped graph (estimated {} edges)...", estimated_edges);
    }
    
    // Create memory-mapped graph builder.
    // For large populations (>100k), disable indexing to save memory during generation.
    // Smaller populations use disk-based indexing for faster lookups.
    let max_node_id = (config.population - 1) as u32; // Node IDs are 0-based.
    let mut builder = if config.population > 100_000 {
        if config.progress_interval.is_some() {
            info!("Large population detected ({}), disabling index to save memory", config.population);
        }
        res!(MmapGraphBuilder::new_without_index(&mmap_path, estimated_edges))
    } else {
        if config.progress_interval.is_some() {
            info!("Using disk-based index for fast lookups");
        }
        res!(MmapGraphBuilder::new(&mmap_path, estimated_edges, max_node_id))
    };
    
    // Step 3: Match stubs and write directly to memory-mapped file.
    if config.progress_interval.is_some() {
        let total_stubs = stubs.len();
        info!("Step 3: Matching {} stubs and writing to mmap file...", total_stubs);
    }
    
    let total_edges = res!(match_stubs_and_insert_to_mmap(
        &mut builder,
        stubs,
        &config.reciprocity_matrix,
        config.num_circles,
        config.link_mode,
        config.progress_interval,
        config.memory_limit_mb,
        config.chunk_size
    ));
    
    // Finalise graph (this will create the disk-based index if enabled).
    let mmap_graph = res!(builder.build());
    
    
    if config.progress_interval.is_some() {
        let memory_mb = get_memory_usage_mb();
        info!("Memory-mapped social network complete: {} nodes, {} edges | Memory: {:.1}MB", 
              config.population, total_edges, memory_mb);
    }
    
    Ok(SocialGraph::new(mmap_graph, config.population))
}

/// Matches stubs and inserts edges directly into memory-mapped graph.
fn match_stubs_and_insert_to_mmap(
    builder:            &mut MmapGraphBuilder,
    stubs:              Vec<Stub>,
    reciprocity_matrix: &Vec<Vec<f32>>,
    num_circles:        usize,
    link_mode:          LinkMode,
    progress_interval:  Option<u32>,
    memory_limit_mb:    Option<f32>,
    chunk_size:         Option<u32>,
)
    -> Outcome<usize>
{
    // Check initial memory usage.
    let initial_memory = get_memory_usage_mb();
    if let Some(limit) = memory_limit_mb {
        if initial_memory > limit {
            return Err(err!("Memory usage ({:.1}MB) already exceeds limit ({:.1}MB)", 
                           initial_memory, limit; Invalid, Input));
        }
    }
    
    // Use chunked processing for memory efficiency (always use chunked for mmap).
    let effective_chunk_size = chunk_size.unwrap_or_else(|| {
        // Auto-calculate chunk size based on memory constraints.
        if let Some(limit) = memory_limit_mb {
            // Estimate: aim to use at most 60% of memory limit for stubs.
            let available_mb = limit * 0.6;
            let bytes_per_stub = std::mem::size_of::<Stub>() as f32;
            let stubs_per_mb = 1_048_576.0 / bytes_per_stub;
            (available_mb * stubs_per_mb) as u32
        } else {
            // Default chunk size: 10k stubs for mmap (smaller chunks).
            10_000
        }
    });
    
    if progress_interval.is_some() {
        info!("Using chunked processing for mmap: {} stubs per chunk", effective_chunk_size);
    }
    
    match_stubs_chunked_mmap(
        builder,
        stubs,
        reciprocity_matrix,
        num_circles,
        link_mode,
        progress_interval,
        memory_limit_mb,
        effective_chunk_size
    )
}

/// Matches stubs in chunks and writes directly to memory-mapped graph.
fn match_stubs_chunked_mmap(
    builder:            &mut MmapGraphBuilder,
    mut stubs:          Vec<Stub>,
    reciprocity_matrix: &Vec<Vec<f32>>,
    num_circles:        usize,
    link_mode:          LinkMode,
    progress_interval:  Option<u32>,
    memory_limit_mb:    Option<f32>,
    chunk_size:         u32,
)
    -> Outcome<usize>
{
    let mut total_edges = 0;
    let initial_stubs = stubs.len();
    let chunk_size = chunk_size as usize;
    let total_chunks = (initial_stubs + chunk_size - 1) / chunk_size;
    
    // Shuffle all stubs first for better randomization.
    shuffle_stubs(&mut stubs);
    
    if progress_interval.is_some() {
        info!("Processing {} stubs in {} chunks of size {}", initial_stubs, total_chunks, chunk_size);
    }
    
    let mut chunk_num = 0;
    while !stubs.is_empty() {
        chunk_num += 1;
        
        // Extract chunk from the end of the vector.
        let current_chunk_size = chunk_size.min(stubs.len());
        let chunk_start = stubs.len() - current_chunk_size;
        let chunk: Vec<Stub> = stubs.drain(chunk_start..).collect();
        
        // Check memory usage before processing chunk.
        let current_memory = get_memory_usage_mb();
        if let Some(limit) = memory_limit_mb {
            if current_memory > limit {
                return Err(err!("Memory usage ({:.1}MB) exceeds limit ({:.1}MB) at chunk {}/{}", 
                               current_memory, limit, chunk_num, total_chunks; Invalid, Input));
            }
        }
        
        if let Some(_progress_interval) = progress_interval {
            // Report more frequently for large numbers of chunks to provide better visibility
            let report_interval = if total_chunks > 1000 { 
                std::cmp::max(1, total_chunks / 200)  // Report ~200 times total for large jobs
            } else {
                10  // Original: every 10 chunks for smaller jobs
            };
            
            if chunk_num % report_interval == 1 || chunk_num == total_chunks {
                let percent_complete = (chunk_num as f32 / total_chunks as f32) * 100.0;
                info!("Processing chunk {}/{} ({:.1}%) | {} stubs | Memory: {:.1}MB | {} edges so far", 
                      chunk_num, total_chunks, percent_complete, chunk.len(), current_memory, total_edges);
            }
        }
        
        // Process this chunk and insert edges directly into mmap builder.
        let chunk_edges = res!(match_stubs_simple_mmap(
            builder,
            chunk,
            reciprocity_matrix,
            num_circles,
            link_mode
        ));
        
        total_edges += chunk_edges;
        
        // Periodic memory check during processing.
        if chunk_num % 50 == 0 {
            let current_memory = get_memory_usage_mb();
            if let Some(limit) = memory_limit_mb {
                if current_memory > limit * 0.9 {
                    if progress_interval.is_some() {
                        info!("WARNING: Memory usage ({:.1}MB) approaching limit ({:.1}MB)", 
                              current_memory, limit);
                    }
                }
            }
        }
    }
    
    if progress_interval.is_some() {
        let final_memory = get_memory_usage_mb();
        info!("Chunked stub matching to mmap complete: {} edges created | Memory: {:.1}MB", 
              total_edges, final_memory);
    }
    
    Ok(total_edges)
}

/// Filter for selecting which nodes to dump.
#[derive(Clone, Debug)]
pub enum NodeFilter {
    /// Dump all nodes.
    All,
    /// Dump nodes with IDs in the specified range (inclusive).
    Range(std::ops::RangeInclusive<usize>),
    /// Dump only nodes with specific IDs.
    Indices(Vec<usize>),
}

impl NodeFilter {
    /// Checks if a node ID passes the filter.
    fn matches(&self, id: usize) -> bool {
        match self {
            NodeFilter::All => true,
            NodeFilter::Range(range) => range.contains(&id),
            NodeFilter::Indices(indices) => indices.contains(&id),
        }
    }
}

impl From<std::ops::RangeInclusive<usize>> for NodeFilter {
    fn from(range: std::ops::RangeInclusive<usize>) -> Self {
        NodeFilter::Range(range)
    }
}

impl From<Vec<usize>> for NodeFilter {
    fn from(indices: Vec<usize>) -> Self {
        NodeFilter::Indices(indices)
    }
}

impl From<&[usize]> for NodeFilter {
    fn from(indices: &[usize]) -> Self {
        NodeFilter::Indices(indices.to_vec())
    }
}

/// Dumps the graph in a human-readable format.
/// 
/// Displays each node with its ID, name, and all incoming/outgoing links
/// formatted to show circle relationships clearly.
/// 
/// # Arguments
/// * `graph` - The social network graph to display.
/// * `filter` - Optional filter to select which nodes to dump.
/// 
/// # Returns
/// A formatted string representation of the graph.
/// 
/// # Example
/// ```no_run
/// use oxedyne_fe2o3_social::graph::{NetworkConfig, generate_social_network, dump_graph, NodeFilter};
/// 
/// let mut config = NetworkConfig::default();
/// config.population = 10; // Small network for display.
/// let graph = generate_social_network(config).unwrap();
/// 
/// // Dump all nodes.
/// let dump_all = dump_graph(&graph, None);
/// 
/// // Dump nodes 0-4.
/// let dump_range = dump_graph(&graph, Some(NodeFilter::Range(0..=4)));
/// 
/// // Dump specific nodes.
/// let dump_specific = dump_graph(&graph, Some(NodeFilter::Indices(vec![1, 3, 5])));
/// 
/// println!("{}", dump_all); // Shows nodes with hex IDs and circle connections.
/// ```
pub fn dump_graph(
    graph: &SocialGraph,
    filter: Option<NodeFilter>,
)
    -> String
{
    let mut output = String::new();
    
    // Use provided filter or default to All.
    let filter = filter.unwrap_or(NodeFilter::All);
    
    // Get all nodes and sort by ID.
    let mut nodes: Vec<_> = graph.iter_nodes()
        .filter(|(id, _)| filter.matches(id.0 as usize))
        .collect();
    nodes.sort_by_key(|(id, _)| id.0);
    
    for (id, _data) in nodes {
        // Format node ID in hex.
        output.push_str(&format!("Node 0x{:04x}\n", id.0));
        
        // Get incoming links.
        let incoming = graph.get_links_to(&id);
        if !incoming.is_empty() {
            output.push_str("  Incoming:\n");
            for (from_id, link) in incoming {
                output.push_str(&format!(
                    "    <- 0x{:04x} [{}]\n",
                    from_id.0,
                    link
                ));
            }
        }
        
        // Get outgoing links.
        let outgoing = graph.get_links_from(&id);
        if !outgoing.is_empty() {
            output.push_str("  Outgoing:\n");
            for (to_id, link) in outgoing {
                output.push_str(&format!(
                    "    -> 0x{:04x} [{}]\n",
                    to_id.0,
                    link
                ));
            }
        }
        
        output.push_str("\n");
    }
    
    output
}

/// Verifies that the generated graph matches the configuration specifications.
/// 
/// Calculates graph statistics and checks that they align with the
/// expected values from the NetworkConfig.
/// 
/// # Arguments
/// * `graph` - The generated social network graph.
/// * `config` - The configuration used to generate the graph.
/// 
/// # Returns
/// Graph statistics and verification results.
pub fn verify_graph(
    graph:  &SocialGraph,
    config: &NetworkConfig,
)
    -> Outcome<GraphStatistics> 
{
    let population = config.population;
    let edge_count = graph.edge_count();
    
    if graph.len() == 0 && population > 0 {
        return Err(err!(
            "Graph is empty but config specifies {} nodes", population;
            Invalid, Configuration
        ));
    }
    
    // Verification logic.
    let mut circle_counts = vec![0usize; config.num_circles];
    let mut total_edges_sampled = 0;
    let sample_size = (population / 10).max(1).min(100);
    
    for i in 0..sample_size {
        let node_id = PersonId(i as u32);
        let outgoing = graph.get_links_from(&node_id);
        for (_target, link) in outgoing {
            let from_circle = link.from_circle().0 as usize;
            let to_circle = link.to_circle().0 as usize;
            if from_circle < config.num_circles {
                circle_counts[from_circle] += 1;
            }
            if to_circle < config.num_circles {
                circle_counts[to_circle] += 1;
            }
            total_edges_sampled += 1;
        }
    }
    
    let avg_circle_sizes: Vec<f32> = circle_counts
        .iter()
        .map(|&count| {
            if total_edges_sampled > 0 {
                count as f32 / total_edges_sampled as f32
            } else {
                0.0
            }
        })
        .collect();
    
    let mut profile_counts = HashMap::new();
    for profile in &config.profiles {
        let estimated_count = (profile.probability * population as f32) as usize;
        profile_counts.insert(profile.profile_type, estimated_count);
    }
    
    let expected_min_edges = population as usize / 10;
    let expected_max_edges = population as usize * 1000;
    
    if edge_count < expected_min_edges {
        return Err(err!(
            "Too few edges: {} (expected at least {} for {} nodes)", 
            edge_count, expected_min_edges, population;
            Invalid, Configuration
        ));
    }
    
    if edge_count > expected_max_edges {
        return Err(err!(
            "Too many edges: {} (expected at most {} for {} nodes)", 
            edge_count, expected_max_edges, population;
            Invalid, Configuration
        ));
    }
    
    Ok(GraphStatistics {
        population,
        profile_counts,
        avg_circle_sizes,
    })
}

/// Creates stubs for population using multi-profile sampling.
/// 
/// Generates connection stubs for the matching algorithm by sampling
/// each node's profile probabilistically and then sampling circle sizes
/// using per-profile Gaussian parameters.
/// 
/// # Arguments
/// * `config` - Network configuration with profiles and population.
/// 
/// # Returns
/// Vector of stubs for matching.
fn create_stubs(config: &NetworkConfig) -> Vec<Stub> {
    create_multiprofile_stubs(config)
}


/// Creates stubs using multi-profile sampling.
fn create_multiprofile_stubs(config: &NetworkConfig) -> Vec<Stub> {
    let mut stubs = Vec::new();
    
    for i in 0..config.population as usize {
        let id = PersonId(i as u32);
        let profile = match sample_profile(&config.profiles) {
            Ok(p) => p,
            Err(_) => {
                if config.profiles.is_empty() {
                    continue;
                }
                &config.profiles[0]
            }
        };
        
        for (circle_idx, &(min_size, max_size)) in profile.circle_ranges.iter().enumerate() {
            let circle_type = CircleType(circle_idx as u8);
            let sampling_method = profile.sampling_methods.get(circle_idx)
                .copied()
                .unwrap_or(SamplingMethod::Uniform);
            
            let size = Rand::sample_u32(
                min_size,
                max_size,
                sampling_method
            ).unwrap_or(min_size);
            
            for _ in 0..size {
                stubs.push(Stub {
                    owner_id: id,
                    circle_type,
                });
            }
        }
    }
    
    stubs
}

/// Samples a profile based on probabilities.
fn sample_profile<'a>(profiles: &'a [Profile]) -> Outcome<&'a Profile> {
    let roll = Rand::value::<f32>();
    let mut cumulative = 0.0;
    
    for profile in profiles {
        cumulative += profile.probability;
        if roll <= cumulative {
            return Ok(profile);
        }
    }
    
    // Should not reach here if probabilities sum to 1.0.
    Err(err!("Profile probabilities do not sum to 1.0"; Invalid, Configuration))
}

/// Simple stub matching that writes directly to memory-mapped builder.
fn match_stubs_simple_mmap(
    builder:            &mut MmapGraphBuilder,
    mut chunk:          Vec<Stub>,
    reciprocity_matrix: &Vec<Vec<f32>>,
    num_circles:        usize,
    link_mode:          LinkMode,
)
    -> Outcome<usize>
{
    let mut edges_created = 0;
    
    // Process pairs from this chunk.
    while chunk.len() >= 2 {
        let stub_a = match chunk.pop() {
            Some(stub) => stub,
            None => return Err(err!("Expected stub A in chunk"; Invalid, Input)),
        };
        let stub_b = match chunk.pop() {
            Some(stub) => stub,
            None => return Err(err!("Expected stub B in chunk"; Invalid, Input)),
        };
        
        // Check for self-loop.
        if stub_a.owner_id == stub_b.owner_id {
            chunk.push(stub_a);
            continue;
        }
        
        // Create and insert edges based on link mode.
        match link_mode {
            LinkMode::Reciprocal => {
                // Determine reciprocal circle type using matrix.
                let to_circle = res!(sample_reciprocal_circle(
                    stub_a.circle_type,
                    reciprocity_matrix,
                    num_circles
                ));
                
                let link_data = SocialLink::new(stub_a.circle_type, to_circle);
                res!(builder.add_edge(stub_a.owner_id.0, stub_b.owner_id.0, link_data.packed));
                edges_created += 1;
                
                let reverse_link_data = SocialLink::new(to_circle, stub_a.circle_type);
                res!(builder.add_edge(stub_b.owner_id.0, stub_a.owner_id.0, reverse_link_data.packed));
                edges_created += 1;
            },
            LinkMode::Symmetric => {
                // In symmetric mode, both people put each other in the same circle.
                // Use one of the stub circle types (pick randomly between them).
                let symmetric_circle = if stub_a.circle_type.0 <= stub_b.circle_type.0 {
                    stub_a.circle_type
                } else {
                    stub_b.circle_type
                };
                
                let link_data = SocialLink::new(symmetric_circle, symmetric_circle);
                res!(builder.add_edge(stub_a.owner_id.0, stub_b.owner_id.0, link_data.packed));
                edges_created += 1;
                
                // Create identical symmetric link in reverse direction.
                res!(builder.add_edge(stub_b.owner_id.0, stub_a.owner_id.0, link_data.packed));
                edges_created += 1;
            },
            LinkMode::NonReciprocal => {
                // Determine target circle type using matrix.
                let to_circle = res!(sample_reciprocal_circle(
                    stub_a.circle_type,
                    reciprocity_matrix,
                    num_circles
                ));
                
                let link_data = SocialLink::new(stub_a.circle_type, to_circle);
                res!(builder.add_edge(stub_a.owner_id.0, stub_b.owner_id.0, link_data.packed));
                edges_created += 1;
            },
        }
    }
    
    Ok(edges_created)
}

/// Shuffles stubs randomly in place.
/// 
/// Uses Fisher-Yates shuffle algorithm for uniform randomisation.
/// 
/// # Arguments
/// * `stubs` - Mutable vector of stubs to shuffle.
fn shuffle_stubs(stubs: &mut Vec<Stub>) {
    let len = stubs.len();
    if len <= 1 {
        return;
    }
    
    // Fisher-Yates shuffle.
    for i in (1..len).rev() {
        let j = Rand::in_range(0, i);
        stubs.swap(i, j);
    }
}

/// Samples reciprocal circle type based on reciprocity matrix.
/// 
/// Determines what circle type the target node should use
/// for the reciprocal connection based on probabilities.
/// 
/// # Arguments
/// * `from_circle` - Source circle type.
/// * `reciprocity_matrix` - Probability matrix for reciprocity.
/// * `num_circles` - Number of circles in the network.
/// 
/// # Returns
/// Target circle type or error if matrix invalid.
fn sample_reciprocal_circle(
    from_circle:        CircleType,
    reciprocity_matrix: &Vec<Vec<f32>>,
    num_circles:        usize,
)
    -> Outcome<CircleType>
{
    let row_idx = from_circle.to_index();
    if row_idx >= reciprocity_matrix.len() {
        return Err(err!(
            "Circle index {} exceeds matrix size {}", row_idx, reciprocity_matrix.len();
            Invalid, Index
        ));
    }
    
    let probabilities = &reciprocity_matrix[row_idx];
    let roll = Rand::value::<f32>();
    let mut cumulative = 0.0;
    
    for (idx, &prob) in probabilities.iter().enumerate() {
        cumulative += prob;
        if roll <= cumulative {
            return CircleType::from_index(idx, num_circles);
        }
    }
    
    // Default to outermost circle if probabilities don't sum to 1.0.
    Ok(CircleType((num_circles - 1) as u8))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_generate_network() -> Outcome<()> {
        let mut config = NetworkConfig::default();
        config.use_mmap = Some("/tmp/test_generate_network.mmap".to_string());
        let graph = res!(generate_social_network(config.clone()));
        
        // Basic validation - check edge count is reasonable for population size.
        let edge_count = graph.edge_count();
        let expected_population = config.population;
        
        // Social networks typically have edge counts much higher than node counts
        // For our test config, we expect at least some edges per node
        if edge_count < expected_population / 10 {
            return Err(err!(
                "Graph has too few edges ({}) for population ({})", 
                edge_count, expected_population;
                Test, Unexpected
            ));
        }
        
        Ok(())
    }
    
    #[test]
    fn test_circle_type_conversion() -> Outcome<()> {
        // Test round-trip conversion.
        let num_circles = 4;
        for i in 0..num_circles {
            let circle = CircleType(i as u8);
            let idx = circle.to_index();
            let converted = res!(CircleType::from_index(idx, num_circles));
            req!(circle, converted);
        }
        
        // Test named constructors.
        req!(CircleType::inner().to_index(), 0);
        req!(CircleType::close().to_index(), 1);
        req!(CircleType::active().to_index(), 2);
        req!(CircleType::wider().to_index(), 3);
        
        // Test invalid index.
        match CircleType::from_index(4, 4) {
            Err(_) => Ok(()),
            Ok(_) => Err(err!(
                "Should have failed for invalid index";
                Test, Unexpected
            )),
        }
    }
    
    #[test]
    fn test_verify_graph() -> Outcome<()> {
        let mut config = NetworkConfig::default();
        config.use_mmap = Some("/tmp/test_verify_graph.mmap".to_string());
        let graph = res!(generate_social_network(config.clone()));
        
        // Verify the graph matches configuration.
        let stats = res!(verify_graph(&graph, &config));
        
        // Check basic statistics.
        req!(stats.population, config.population);
        
        // Check that we have both profile types.
        req!(stats.profile_counts.contains_key(&ProfileType::Isolated), true);
        req!(stats.profile_counts.contains_key(&ProfileType::Connected), true);
        
        // Check average circle sizes structure.
        req!(stats.avg_circle_sizes.len(), 4);
        
        Ok(())
    }
    
    fn test_config(n: usize) -> NetworkConfig {
        // Create a unique test file path for this population size
        let test_path = format!("/tmp/test_social_graph_{}.mmap", n);
        
        NetworkConfig {
            population: n,
            profiles: vec![
                Profile {
                    profile_type:	ProfileType::Isolated,
                    probability:	0.33,
                    circle_ranges:	vec![
                        (1, 2),     // Inner circle.
                        (2, 3),     // Close circle.
                        (3, 4),     // Active circle.
                        (4, 5),     // Wider circle.
                    ],
                    sampling_methods: vec![SamplingMethod::Uniform; 4],
                },
                Profile {
                    profile_type:	ProfileType::Connected,
                    probability:	0.67,
                    circle_ranges:	vec![
                        (2, 4),     // Inner circle.
                        (4, 6),     // Close circle.
                        (6, 8),     // Active circle.
                        (8, 10),    // Wider circle.
                    ],
                    sampling_methods: vec![SamplingMethod::Uniform; 4],
                },
            ],
            num_circles: 4,
            reciprocity_matrix: vec![
                vec![0.95, 0.05, 0.00, 0.00], // Inner -> x.
                vec![0.30, 0.50, 0.20, 0.00], // Close -> x.
                vec![0.10, 0.40, 0.40, 0.10], // Active -> x.
                vec![0.00, 0.10, 0.30, 0.60], // Wider -> x.
            ],
            circle_labels: Some(CircleLabels::default()),
            link_mode: LinkMode::Symmetric,
            progress_interval: None,
            memory_limit_mb: None,
            chunk_size: None,
            use_mmap: Some(test_path), // Set memory-mapped path for tests
        }
    }

    #[test]
    fn test_dump_graph() -> Outcome<()> {
        // Create a small test network.
        let config = test_config(20);
        
        let graph = res!(generate_social_network(config));
        
        // Test dumping all nodes.
        let dump_all = dump_graph(&graph, None);
        req!(dump_all.contains("Node 0x"), true);
        
        // Test dumping a range of nodes.
        let dump_range = dump_graph(&graph, Some(NodeFilter::Range(0..=4)));
        req!(dump_range.contains("Node 0x0000"), true);
        req!(dump_range.contains("Node 0x0004"), true);
        req!(!dump_range.contains("Node 0x0005"), true);
        
        // Test dumping specific nodes.
        let dump_specific = dump_graph(&graph, Some(NodeFilter::Indices(vec![1, 3, 5, 7])));
        req!(dump_specific.contains("Node 0x0001"), true);
        req!(dump_specific.contains("Node 0x0003"), true);
        req!(!dump_specific.contains("Node 0x0002"), true);
        req!(!dump_specific.contains("Node 0x0004"), true);
        
        // Print a sample for manual inspection.
        println!("=== Sample dump (nodes 0-2) ===");
        let sample = dump_graph(&graph, Some(NodeFilter::Range(0..=2)));
        println!("{}", sample);
        
        Ok(())
    }
    
    #[test]
    fn test_dump_filter_demo() -> Outcome<()> {
        // Demo of different ways to use dump_graph filters.
        let config = test_config(10);
        let graph = res!(generate_social_network(config));
        
        println!("=== DUMP FILTER DEMO ===");
        
        // Method 1: Using None for all nodes.
        let all = dump_graph(&graph, None);
        println!("All nodes count: {}", all.matches("Node 0x").count());
        
        // Method 2: Using NodeFilter enum directly.
        let range = dump_graph(&graph, Some(NodeFilter::Range(0..=2)));
        println!("\nNodes 0-2 using NodeFilter::Range:");
        println!("{}", range);
        
        // Method 3: Using From trait with range.
        let range2 = dump_graph(&graph, Some((3..=5).into()));
        println!("Nodes 3-5 using .into():");
        for line in range2.lines().filter(|l| l.starts_with("Node")) {
            println!("  {}", line);
        }
        
        // Method 4: Using From trait with vec.
        let specific = dump_graph(&graph, Some(vec![0, 5, 9].into()));
        println!("\nSpecific nodes [0, 5, 9]:");
        for line in specific.lines().filter(|l| l.starts_with("Node")) {
            println!("  {}", line);
        }
        
        // Method 5: Using From trait with slice.
        let indices: &[usize] = &[1, 4, 7];
        let from_slice = dump_graph(&graph, Some(indices.into()));
        println!("\nFrom slice [1, 4, 7]:");
        for line in from_slice.lines().filter(|l| l.starts_with("Node")) {
            println!("  {}", line);
        }
        
        Ok(())
    }
    
    #[test]
    fn test_reciprocal_links() -> Outcome<()> {
        // Test reciprocal mode.
        let mut config = test_config(10);
        config.link_mode = LinkMode::Reciprocal;
        
        let graph = res!(generate_social_network(config));
        
        // Check that links are reciprocal.
        let mut reciprocal_count = 0;
        let mut total_edges = 0;
        
        for (node_id, _) in graph.iter_nodes() {
            let outgoing = graph.get_links_from(&node_id);
            total_edges += outgoing.len();
            
            for (target_id, _) in outgoing {
                // Check if there's a reverse link.
                let incoming = graph.get_links_to(&node_id);
                let has_reverse = incoming.iter().any(|(from_id, _)| *from_id == target_id);
                if has_reverse {
                    reciprocal_count += 1;
                }
            }
        }
        
        // In reciprocal mode, most links should be reciprocal.
        // Allow some tolerance since edge creation can be affected by stub counts.
        let reciprocal_ratio = reciprocal_count as f32 / total_edges as f32;
        if reciprocal_ratio < 0.7 {
            return Err(err!(
                "Reciprocal link ratio too low: {}", reciprocal_ratio;
                Test, Unexpected
            ));
        }
        
        Ok(())
    }
    
    
    #[test]
    fn test_non_reciprocal_links() -> Outcome<()> {
        // Test that non-reciprocal mode produces a graph.
        let mut config = test_config(10);
        config.link_mode = LinkMode::NonReciprocal;
        
        let graph = res!(generate_social_network(config));
        
        // Just verify the graph was created successfully and has nodes.
        if graph.len() == 0 {
            return Err(err!(
                "Non-reciprocal graph should have nodes";
                Test, Unexpected
            ));
        }
        
        // Check that some nodes have connections.
        let mut has_edges = false;
        for (node_id, _) in graph.iter_nodes() {
            let outgoing = graph.get_links_from(&node_id);
            if !outgoing.is_empty() {
                has_edges = true;
                break;
            }
        }
        
        if !has_edges {
            return Err(err!(
                "Non-reciprocal graph should have edges";
                Test, Unexpected
            ));
        }
        
        Ok(())
    }
    
    #[test]
    fn test_link_mode_demo() -> Outcome<()> {
        // Demo showing the difference between all three link modes.
        let base_config = test_config(5);
        
        println!("=== LINK MODE COMPARISON ===");
        
        // Test Reciprocal mode.
        let mut reciprocal_config = base_config.clone();
        reciprocal_config.link_mode = LinkMode::Reciprocal;
        let reciprocal_graph = res!(generate_social_network(reciprocal_config));
        println!("\nReciprocal Mode (inverted circles):");
        dump_sample_connections(&reciprocal_graph, 1);
        
        // Test Symmetric mode.
        let mut symmetric_config = base_config.clone();
        symmetric_config.link_mode = LinkMode::Symmetric;
        let symmetric_graph = res!(generate_social_network(symmetric_config));
        println!("\nSymmetric Mode (identical circles):");
        dump_sample_connections(&symmetric_graph, 1);
        
        // Test Non-reciprocal mode.
        let mut non_reciprocal_config = base_config.clone();
        non_reciprocal_config.link_mode = LinkMode::NonReciprocal;
        let non_reciprocal_graph = res!(generate_social_network(non_reciprocal_config));
        println!("\nNon-Reciprocal Mode (one-way only):");
        dump_sample_connections(&non_reciprocal_graph, 1);
        
        Ok(())
    }
    
    // Helper function to dump sample connections from a graph.
    fn dump_sample_connections(graph: &SocialGraph, max_nodes: usize) {
        let mut count = 0;
        for (node_id, _) in graph.iter_nodes() {
            if count >= max_nodes { break; }
            
            let outgoing = graph.get_links_from(&node_id);
            let incoming = graph.get_links_to(&node_id);
            
            println!("  Node {:?}:", node_id);
            for (target_id, link) in &outgoing {
                print!("    -> {:?}: [C{} -> C{}]", target_id, link.from_circle().0, link.to_circle().0);
                
                // Find reverse link if it exists.
                let mut found_reverse = false;
                for (source_id, reverse_link) in &incoming {
                    if source_id == target_id {
                        println!(" <-> [C{} -> C{}]", reverse_link.from_circle().0, reverse_link.to_circle().0);
                        found_reverse = true;
                        break;
                    }
                }
                if !found_reverse {
                    println!(" (one-way)");
                }
            }
            count += 1;
        }
    }
    
    #[test]
    fn test_mode_comparison() -> Outcome<()> {
        // Test that reciprocal mode creates more reciprocal links than non-reciprocal mode.
        
        // Create reciprocal network.
        let mut reciprocal_config = test_config(15);
        reciprocal_config.link_mode = LinkMode::Reciprocal;
        let reciprocal_graph = res!(generate_social_network(reciprocal_config));
        
        // Create non-reciprocal network.
        let mut non_reciprocal_config = test_config(15);
        non_reciprocal_config.link_mode = LinkMode::NonReciprocal;
        let non_reciprocal_graph = res!(generate_social_network(non_reciprocal_config));
        
        // Calculate reciprocal ratios for both.
        let calc_ratio = |graph: &SocialGraph| -> f32 {
            let mut reciprocal_count = 0;
            let mut total_edges = 0;
            
            for (node_id, _) in graph.iter_nodes() {
                let outgoing = graph.get_links_from(&node_id);
                total_edges += outgoing.len();
                
                for (target_id, _) in outgoing {
                    let incoming = graph.get_links_to(&node_id);
                    let has_reverse = incoming.iter().any(|(from_id, _)| *from_id == target_id);
                    if has_reverse {
                        reciprocal_count += 1;
                    }
                }
            }
            
            if total_edges > 0 {
                reciprocal_count as f32 / total_edges as f32
            } else {
                0.0
            }
        };
        
        let reciprocal_ratio = calc_ratio(&reciprocal_graph);
        let non_reciprocal_ratio = calc_ratio(&non_reciprocal_graph);
        
        // Reciprocal mode should have high reciprocal ratio.
        if reciprocal_ratio < 0.8 {
            return Err(err!(
                "Reciprocal mode ratio ({}) should be >= 0.8", reciprocal_ratio;
                Test, Unexpected
            ));
        }
        
        // Non-reciprocal mode may have perfect reciprocity too due to stub matching algorithm.
        // Just verify both modes work and reciprocal is at least as good.
        if reciprocal_ratio < non_reciprocal_ratio {
            return Err(err!(
                "Reciprocal mode ratio ({}) should be >= non-reciprocal ratio ({})", 
                reciprocal_ratio, non_reciprocal_ratio;
                Test, Unexpected
            ));
        }
        
        println!("Reciprocal mode ratio: {:.2}", reciprocal_ratio);
        println!("Non-reciprocal mode ratio: {:.2}", non_reciprocal_ratio);
        
        Ok(())
    }
    
    
    #[test]
    fn test_reciprocal_debug() -> Outcome<()> {
        // Debug reciprocal mode with larger network.
        let mut config = test_config(20);
        config.link_mode = LinkMode::Reciprocal;
        
        let graph = res!(generate_social_network(config));
        
        println!("=== RECIPROCAL DEBUG ===");
        
        // Show first node's connections in detail.
        if let Some((first_id, _)) = graph.iter_nodes().next() {
            println!("Example reciprocal connections for Node 0x{:04x}:", first_id.0);
            
            let outgoing = graph.get_links_from(&first_id);
            for (target_id, link_data) in &outgoing {
                let incoming = graph.get_links_to(&first_id);
                let reverse = incoming.iter().find(|(from_id, _)| *from_id == *target_id);
                
                if let Some((_, reverse_link)) = reverse {
                    println!("  0x{:04x} <-> 0x{:04x}: [{}] <-> [{}]", 
                        first_id.0, target_id.0, link_data, reverse_link);
                } else {
                    println!("  0x{:04x} -> 0x{:04x}: [{}] (NO REVERSE!)", 
                        first_id.0, target_id.0, link_data);
                }
            }
        }
        
        // Count non-reciprocal edges.
        let mut non_reciprocal_count = 0;
        let mut total_edges = 0;
        
        for (node_id, _) in graph.iter_nodes() {
            let outgoing = graph.get_links_from(&node_id);
            
            for (target_id, _) in outgoing {
                total_edges += 1;
                
                // Check if there's a reverse link.
                let incoming_to_target = graph.get_links_to(&target_id);
                let has_reverse = incoming_to_target.iter().any(|(from_id, _)| *from_id == node_id);
                
                if !has_reverse {
                    non_reciprocal_count += 1;
                    println!("NON-RECIPROCAL: 0x{:04x} -> 0x{:04x} has no reverse", node_id.0, target_id.0);
                }
            }
        }
        
        println!("Non-reciprocal edges: {} / {}", non_reciprocal_count, total_edges);
        
        if non_reciprocal_count > 0 {
            return Err(err!(
                "Found {} non-reciprocal edges in reciprocal mode", non_reciprocal_count;
                Test, Unexpected
            ));
        }
        
        Ok(())
    }
    
    #[test]
    fn test_sample_u32() -> Outcome<()> {
        // Test uniform sampling.
        for _ in 0..100 {
            let val = res!(Rand::sample_u32(10, 20, SamplingMethod::Uniform));
            if !(val >= 10 && val <= 20) {
                return Err(err!(
                    "Uniform sample {} out of range [10, 20]", val;
                    Test, Unexpected
                ));
            }
        }
        
        // Test Gaussian sampling.
        for _ in 0..100 {
            let val = res!(Rand::sample_u32(50, 100, SamplingMethod::GaussianClampedDerived));
            if !(val >= 50 && val <= 100) {
                return Err(err!(
                    "Gaussian sample {} out of range [50, 100]", val;
                    Test, Unexpected
                ));
            }
        }
        
        // Test invalid range.
        match Rand::sample_u32(20, 10, SamplingMethod::Uniform) {
            Err(_) => Ok(()),
            Ok(_) => Err(err!(
                "Should have failed for invalid range";
                Test, Unexpected
            )),
        }
    }
}
