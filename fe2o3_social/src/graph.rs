//! Social network graph generator using stub matching algorithm.
//!
//! This module generates realistic social networks with configurable
//! population profiles, social circles, and geographic distributions.
//! poo.

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};
use oxedyne_fe2o3_data::digraph::{
    DiGraph,
    LinkData,
    NodeData,
    NodeId,
};

use crate::person::{
    Person,
    PersonGenConfig,
};

use std::{
    collections::HashMap,
    fmt,
    hash::Hash,
};


/// Person identifier in the social network.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PersonId(pub usize);

impl NodeId for PersonId {}

/// Geographic location using x, y coordinates.
#[derive(Clone, Debug)]
pub struct Location {
    pub x: f64,
    pub y: f64,
}

/// Data stored for each person in the network.
#[derive(Clone, Debug)]
pub struct PersonData {
    pub person:			Person,
    pub profile_type:	ProfileType,
    pub location:		Location,
    pub circle_sizes:	Vec<usize>,
}

impl fmt::Display for PersonData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.person)
    }
}

impl NodeData for PersonData {}

/// Type of social circle relationship.
/// 
/// Represents a numbered circle from 0 (innermost) to n-1 (outermost).
#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub struct CircleType(pub usize);

impl CircleType {
    /// Converts circle type to matrix index.
    /// 
    /// # Returns
    /// Index for use in reciprocity matrix.
    pub fn to_index(&self) -> usize {
        self.0
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
        if idx >= max_circles {
            Err(err!(
                "Invalid circle index: {} (max: {})", idx, max_circles - 1;
                Invalid, Index
            ))
        } else {
            Ok(Self(idx))
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
#[derive(Clone, Debug)]
pub struct SocialLink {
    pub from_circle:	CircleType,
    pub to_circle:		CircleType,
}

impl fmt::Display for SocialLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "C{} -> C{}", self.from_circle.0, self.to_circle.0)
    }
}

impl LinkData for SocialLink {}

/// Profile types for population segments.
#[derive(Clone, Debug, Copy, PartialEq, Eq, Hash)]
pub enum ProfileType {
    Isolated,
    Connected,
}

/// Profile definition with circle size ranges.
#[derive(Clone, Debug)]
pub struct Profile {
    pub profile_type:	ProfileType,
    pub probability:	f64,
    pub circle_ranges:	Vec<(usize, usize)>, // (min, max) for each circle.
}

/// Sampling method for circle sizes and locations.
#[derive(Clone, Debug, Copy)]
pub enum SamplingMethod {
    Uniform,
    Gaussian,
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

/// Geographic distribution parameters.
#[derive(Clone, Debug)]
pub struct GeographicParams {
    pub origin_x:	f64,
    pub origin_y:	f64,
    pub extent:		f64,
    pub method:		SamplingMethod,
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
                "Inner".to_string(),
                "Close".to_string(),
                "Active".to_string(),
                "Wider".to_string(),
            ],
        }
    }
}

/// Configuration for social network generation.
#[derive(Clone)]
pub struct NetworkConfig {
    pub population:			usize,
    pub profiles:			Vec<Profile>,
    pub sampling_method:	SamplingMethod,
    pub geographic_params:	GeographicParams,
    pub num_circles:		usize,
    pub reciprocity_matrix:	Vec<Vec<f64>>, // NxN matrix for circle reciprocity.
    pub circle_labels:		Option<CircleLabels>, // Optional labels for circles.
    pub link_mode:			LinkMode, // Whether links are reciprocal or not.
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
                },
            ],
            sampling_method: SamplingMethod::Uniform,
            geographic_params: GeographicParams {
                origin_x:	0.0,
                origin_y:	0.0,
                extent:		100.0,
                method:		SamplingMethod::Gaussian,
            },
            num_circles: 4,
            reciprocity_matrix: vec![
                vec![0.95, 0.05, 0.00, 0.00], // Inner -> x.
                vec![0.30, 0.50, 0.20, 0.00], // Close -> x.
                vec![0.10, 0.40, 0.40, 0.10], // Active -> x.
                vec![0.00, 0.10, 0.30, 0.60], // Wider -> x.
            ],
            circle_labels: Some(CircleLabels::default()),
            link_mode: LinkMode::Reciprocal, // Default to reciprocal for backward compatibility.
        }
    }
}

/// Statistics from graph verification.
#[derive(Debug)]
pub struct GraphStatistics {
    pub population:				usize,
    pub profile_counts:			HashMap<ProfileType, usize>,
    pub avg_circle_sizes:		Vec<f64>, // Average circle sizes by type.
}

/// Generates a social network graph using the stub matching algorithm.
/// 
/// Creates a directed graph representing social relationships between
/// people based on profile types and geographic distribution.
/// 
/// # Arguments
/// * `config` - Network generation configuration.
/// 
/// # Returns
/// A directed graph of social connections or error if generation fails.
pub fn generate_social_network(
    config: NetworkConfig,
)
    -> Outcome<DiGraph<PersonId, PersonData, SocialLink>>
{
    let mut graph = DiGraph::new();
    
    // Step 1: Create nodes with sampled properties.
    let nodes = res!(create_nodes(&config));
    
    // Insert nodes into graph.
    for (id, data) in &nodes {
        graph.insert(id.clone(), data.clone());
    }
    
    // Step 2: Create stubs based on circle sizes.
    let stubs = create_stubs(&nodes);
    
    // Step 3: Match stubs to create edges.
    let edges = res!(match_stubs(stubs, &config.reciprocity_matrix, config.num_circles, config.link_mode));
    
    // Insert edges into graph.
    for (from, to, link_data) in edges {
        graph.link(&from, &to, link_data);
    }
    
    Ok(graph)
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
    graph: &DiGraph<PersonId, PersonData, SocialLink>,
    filter: Option<NodeFilter>,
)
    -> String
{
    let mut output = String::new();
    
    // Use provided filter or default to All.
    let filter = filter.unwrap_or(NodeFilter::All);
    
    // Get all nodes and sort by ID.
    let mut nodes: Vec<_> = graph.iter_nodes()
        .filter(|(id, _)| filter.matches(id.0))
        .collect();
    nodes.sort_by_key(|(id, _)| id.0);
    
    for (id, data) in nodes {
        // Format node ID in hex.
        output.push_str(&format!("Node 0x{:04x}: {}\n", id.0, data));
        
        // Get incoming links.
        let incoming = graph.get_links_to(id);
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
        let outgoing = graph.get_links_from(id);
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
    graph: &DiGraph<PersonId, PersonData, SocialLink>,
    config: &NetworkConfig,
)
    -> Outcome<GraphStatistics>
{
    // Count nodes by profile type and calculate average circle sizes.
    let mut profile_counts = HashMap::new();
    let mut circle_totals = vec![0usize; config.num_circles];
    let mut node_count = 0usize;
    
    // Use find_nodes_with_data to get all nodes.
    let all_nodes = graph.find_nodes_with_data(|_| true);
    
    for (_id, data) in &all_nodes {
        *profile_counts.entry(data.profile_type).or_insert(0) += 1;
        node_count += 1;
        
        // Sum up circle sizes.
        for (idx, &size) in data.circle_sizes.iter().enumerate() {
            if idx < config.num_circles {
                circle_totals[idx] += size;
            }
        }
    }
    
    let population = graph.len();
    
    // Calculate average circle sizes.
    let avg_circle_sizes: Vec<f64> = circle_totals
        .iter()
        .map(|&total| {
            if node_count > 0 {
                total as f64 / node_count as f64
            } else {
                0.0
            }
        })
        .collect();
    
    // Verify population matches.
    if population != config.population {
        return Err(err!(
            "Population mismatch: expected {}, got {}", config.population, population;
            Invalid, Configuration
        ));
    }
    
    // Verify profile distributions are approximately correct.
    for profile in &config.profiles {
        let expected_count = (profile.probability * config.population as f64).round() as usize;
        let actual_count = *profile_counts.get(&profile.profile_type).unwrap_or(&0);
        
        // Allow 10% deviation.
        let tolerance = (expected_count as f64 * 0.1).max(10.0) as usize;
        if actual_count < expected_count.saturating_sub(tolerance) ||
           actual_count > expected_count + tolerance {
            return Err(err!(
                "Profile count mismatch for {:?}: expected ~{}, got {}",
                profile.profile_type, expected_count, actual_count;
                Invalid, Configuration
            ));
        }
    }
    
    Ok(GraphStatistics {
        population,
        profile_counts,
        avg_circle_sizes,
    })
}

/// Creates nodes with sampled properties.
/// 
/// Generates population nodes with profile types, locations,
/// and social circle sizes based on configuration.
/// 
/// # Arguments
/// * `config` - Network configuration parameters.
/// 
/// # Returns
/// Vector of person IDs and their associated data.
fn create_nodes(
    config: &NetworkConfig,
)
    -> Outcome<Vec<(PersonId, PersonData)>>
{
    let mut nodes = Vec::new();
    
    // Create person generation config.
    let person_config = PersonGenConfig::default();
    
    for i in 0..config.population {
        // Sample profile type.
        let profile = res!(sample_profile(&config.profiles));
        
        // Sample location.
        let location = res!(sample_location(&config.geographic_params));
        
        // Sample circle sizes.
        let mut circle_sizes = Vec::new();
        for (min, max) in &profile.circle_ranges {
            let size = res!(sample_range(*min, *max, config.sampling_method));
            circle_sizes.push(size);
        }
        
        // Generate random 6-letter name.
        let given_name = Rand::generate_random_string(6, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        
        // Generate person with random attributes.
        let mut person = res!(Person::generate_random(&person_config));
        person.given_name = given_name;
        
        let data = PersonData {
            person,
            profile_type:	profile.profile_type,
            location,
            circle_sizes,
        };
        
        nodes.push((PersonId(i), data));
    }
    
    Ok(nodes)
}

/// Samples a profile based on probabilities.
/// 
/// Selects a profile type using weighted random selection
/// based on configured probabilities.
/// 
/// # Arguments
/// * `profiles` - Available profiles with probabilities.
/// 
/// # Returns
/// Selected profile or error if probabilities invalid.
fn sample_profile<'a>(
    profiles: &'a [Profile],
)
    -> Outcome<&'a Profile>
{
    let roll = Rand::value::<f64>();
    let mut cumulative = 0.0;
    
    for profile in profiles {
        cumulative += profile.probability;
        if roll <= cumulative {
            return Ok(profile);
        }
    }
    
    // Should not reach here if probabilities sum to 1.0.
    Err(err!(
        "Profile probabilities do not sum to 1.0";
        Invalid, Configuration
    ))
}

/// Samples a geographic location.
/// 
/// Generates a location based on the specified distribution
/// method (uniform or Gaussian).
/// 
/// # Arguments
/// * `params` - Geographic distribution parameters.
/// 
/// # Returns
/// Sampled location within specified bounds.
fn sample_location(
    params: &GeographicParams,
)
    -> Outcome<Location>
{
    let (dx, dy) = match params.method {
        SamplingMethod::Uniform => {
            let dx = Rand::in_range(-params.extent, params.extent);
            let dy = Rand::in_range(-params.extent, params.extent);
            (dx, dy)
        }
        SamplingMethod::Gaussian => {
            // Standard deviation is 1/3 of extent for ~99.7% within bounds.
            let stdev = params.extent / 3.0;
            let dx = Rand::normal(0.0, stdev);
            let dy = Rand::normal(0.0, stdev);
            // Clamp to bounds.
            let dx = dx.max(-params.extent).min(params.extent);
            let dy = dy.max(-params.extent).min(params.extent);
            (dx, dy)
        }
    };
    
    Ok(Location {
        x: params.origin_x + dx,
        y: params.origin_y + dy,
    })
}

/// Samples a value from a range using the specified method.
/// 
/// Generates an integer within the given range using either
/// uniform or Gaussian distribution.
/// 
/// # Arguments
/// * `min` - Minimum value (inclusive).
/// * `max` - Maximum value (inclusive).
/// * `method` - Sampling method to use.
/// 
/// # Returns
/// Sampled value within range or error if range invalid.
fn sample_range(
    min:    usize,
    max:    usize,
    method: SamplingMethod,
)
    -> Outcome<usize>
{
    if min > max {
        return Err(err!(
            "Invalid range: {} > {}", min, max;
            Invalid, Range
        ));
    }
    
    let value = match method {
        SamplingMethod::Uniform => {
            Rand::in_range(min, max)
        }
        SamplingMethod::Gaussian => {
            let mean = (min + max) as f64 / 2.0;
            let stdev = (max - min) as f64 / 6.0; // 99.7% within range.
            let sample = Rand::normal(mean, stdev).round() as usize;
            // Clamp to range.
            sample.max(min).min(max)
        }
    };
    
    Ok(value)
}

/// Creates stubs for all nodes based on their circle sizes.
/// 
/// Generates stub objects representing potential connections
/// for each person based on their social circle sizes.
/// 
/// # Arguments
/// * `nodes` - Vector of person IDs and data.
/// 
/// # Returns
/// Vector of stubs ready for matching.
fn create_stubs(nodes: &[(PersonId, PersonData)]) -> Vec<Stub> {
    let mut stubs = Vec::new();
    
    for (id, data) in nodes {
        for (circle_idx, &size) in data.circle_sizes.iter().enumerate() {
            let circle_type = CircleType(circle_idx);
            
            for _ in 0..size {
                stubs.push(Stub {
                    owner_id:		id.clone(),
                    circle_type,
                });
            }
        }
    }
    
    stubs
}

/// Matches stubs to create edges with optional reciprocity.
/// 
/// Uses random matching to pair stubs and create directed edges
/// with reciprocal circle types based on the reciprocity matrix.
/// Ensures only one edge per direction between any two nodes.
/// 
/// # Arguments
/// * `stubs` - Vector of stubs to match.
/// * `reciprocity_matrix` - Matrix defining reciprocal probabilities.
/// * `num_circles` - Number of circles in the network.
/// * `link_mode` - Whether to create reciprocal or non-reciprocal links.
/// 
/// # Returns
/// Vector of edges (from, to, link data) or error if matching fails.
fn match_stubs(
    mut stubs: Vec<Stub>,
    reciprocity_matrix: &Vec<Vec<f64>>,
    num_circles: usize,
    link_mode: LinkMode,
)
    -> Outcome<Vec<(PersonId, PersonId, SocialLink)>>
{
    use std::collections::{HashMap, HashSet};
    
    let mut edges = Vec::new();
    let mut existing_edges: HashSet<(PersonId, PersonId)> = HashSet::new();
    
    // Group stubs by owner for efficient lookup.
    let mut stubs_by_owner: HashMap<PersonId, Vec<CircleType>> = HashMap::new();
    for stub in &stubs {
        stubs_by_owner.entry(stub.owner_id.clone())
            .or_insert_with(Vec::new)
            .push(stub.circle_type);
    }
    
    // Shuffle stubs for random matching.
    shuffle_stubs(&mut stubs);
    
    // Match pairs, avoiding duplicate edges.
    while stubs.len() >= 2 {
        // Pop two stubs.
        let stub_a = match stubs.pop() {
            Some(s) => s,
            None => break,
        };
        let stub_b = match stubs.pop() {
            Some(s) => s,
            None => {
                stubs.push(stub_a); // Put first one back.
                break;
            }
        };
        
        // Check for self-loop.
        if stub_a.owner_id == stub_b.owner_id {
            stubs.push(stub_a); // Put one back.
            continue;
        }
        
        // Check if edge already exists.
        let edge_key = (stub_a.owner_id.clone(), stub_b.owner_id.clone());
        if existing_edges.contains(&edge_key) {
            // Skip this pair, but don't put stubs back.
            continue;
        }
        
        // Determine reciprocal circle type.
        let to_circle = res!(sample_reciprocal_circle(
            stub_a.circle_type,
            reciprocity_matrix,
            num_circles
        ));
        
        // Create edges based on link mode.
        match link_mode {
            LinkMode::Reciprocal => {
                // Create the edge from A to B.
                edges.push((
                    stub_a.owner_id.clone(),
                    stub_b.owner_id.clone(),
                    SocialLink {
                        from_circle:	stub_a.circle_type,
                        to_circle,
                    },
                ));
                
                // Create the reverse edge from B to A with inverted circle relationship.
                let reverse_key = (stub_b.owner_id.clone(), stub_a.owner_id.clone());
                if !existing_edges.contains(&reverse_key) {
                    edges.push((
                        stub_b.owner_id.clone(),
                        stub_a.owner_id.clone(),
                        SocialLink {
                            from_circle:	to_circle,        // Use the target circle from forward edge.
                            to_circle:		stub_a.circle_type, // Use the source circle from forward edge.
                        },
                    ));
                    
                    // Mark reverse edge as created.
                    existing_edges.insert(reverse_key);
                }
            },
            LinkMode::Symmetric => {
                // In symmetric mode: both A and B put each other in the same circle.
                // If A is in circle C2 and puts B in their C2, then B (regardless of their circle)
                // also puts A in their C2. Both use C2 -> C2.
                
                // Determine which circle to use - we'll use the calculated to_circle
                // but make both directions symmetric.
                let symmetric_circle = to_circle;
                
                // Create the edge from A to B.
                edges.push((
                    stub_a.owner_id.clone(),
                    stub_b.owner_id.clone(),
                    SocialLink {
                        from_circle:	symmetric_circle,  // Both use same circle.
                        to_circle:		symmetric_circle,   // Both use same circle.
                    },
                ));
                
                // Create the reverse edge from B to A with identical relationship.
                let reverse_key = (stub_b.owner_id.clone(), stub_a.owner_id.clone());
                if !existing_edges.contains(&reverse_key) {
                    edges.push((
                        stub_b.owner_id.clone(),
                        stub_a.owner_id.clone(),
                        SocialLink {
                            from_circle:	symmetric_circle,  // Same circle.
                            to_circle:		symmetric_circle,   // Same circle.
                        },
                    ));
                    
                    // Mark reverse edge as created.
                    existing_edges.insert(reverse_key);
                }
            },
            LinkMode::NonReciprocal => {
                // Create the edge from A to B.
                edges.push((
                    stub_a.owner_id.clone(),
                    stub_b.owner_id.clone(),
                    SocialLink {
                        from_circle:	stub_a.circle_type,
                        to_circle,
                    },
                ));
            },
        }
        
        // Mark this edge as created.
        existing_edges.insert(edge_key);
    }
    
    Ok(edges)
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
    from_circle: CircleType,
    reciprocity_matrix: &Vec<Vec<f64>>,
    num_circles: usize,
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
    let roll = Rand::value::<f64>();
    let mut cumulative = 0.0;
    
    for (idx, &prob) in probabilities.iter().enumerate() {
        cumulative += prob;
        if roll <= cumulative {
            return CircleType::from_index(idx, num_circles);
        }
    }
    
    // Default to outermost circle if probabilities don't sum to 1.0.
    Ok(CircleType(num_circles - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_generate_network() -> Outcome<()> {
        let config = NetworkConfig::default();
        let graph = res!(generate_social_network(config));
        
        // Basic validation.
        req!(graph.len(), 1000);
        
        Ok(())
    }
    
    #[test]
    fn test_circle_type_conversion() -> Outcome<()> {
        // Test round-trip conversion.
        let num_circles = 4;
        for i in 0..num_circles {
            let circle = CircleType(i);
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
        let config = NetworkConfig::default();
        let graph = res!(generate_social_network(config));
        
        // Verify the graph matches configuration.
        let config = NetworkConfig::default();
        let stats = res!(verify_graph(&graph, &config));
        
        // Check basic statistics.
        req!(stats.population, 1000);
        
        // Check that we have both profile types.
        req!(stats.profile_counts.contains_key(&ProfileType::Isolated), true);
        req!(stats.profile_counts.contains_key(&ProfileType::Connected), true);
        
        // Check average circle sizes.
        req!(stats.avg_circle_sizes.len(), 4);
        for avg_size in &stats.avg_circle_sizes {
            // Check that average sizes are positive.
            if *avg_size <= 0.0 {
                return Err(err!(
                    "Average circle size should be positive, got {}", avg_size;
                    Test, Unexpected
                ));
            }
        }
        
        Ok(())
    }
    
    fn test_config(n: usize) -> NetworkConfig {
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
                },
            ],
            sampling_method: SamplingMethod::Uniform,
            geographic_params: GeographicParams {
                origin_x:	0.0,
                origin_y:	0.0,
                extent:		100.0,
                method:		SamplingMethod::Gaussian,
            },
            num_circles: 4,
            reciprocity_matrix: vec![
                vec![0.95, 0.05, 0.00, 0.00], // Inner -> x.
                vec![0.30, 0.50, 0.20, 0.00], // Close -> x.
                vec![0.10, 0.40, 0.40, 0.10], // Active -> x.
                vec![0.00, 0.10, 0.30, 0.60], // Wider -> x.
            ],
            circle_labels: Some(CircleLabels::default()),
            link_mode: LinkMode::Symmetric,
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
            let outgoing = graph.get_links_from(node_id);
            total_edges += outgoing.len();
            
            for (target_id, _) in outgoing {
                // Check if there's a reverse link.
                let incoming = graph.get_links_to(node_id);
                let has_reverse = incoming.iter().any(|(from_id, _)| *from_id == target_id);
                if has_reverse {
                    reciprocal_count += 1;
                }
            }
        }
        
        // In reciprocal mode, most links should be reciprocal.
        // Allow some tolerance since edge creation can be affected by stub counts.
        let reciprocal_ratio = reciprocal_count as f64 / total_edges as f64;
        if reciprocal_ratio < 0.7 {
            return Err(err!(
                "Reciprocal link ratio too low: {}", reciprocal_ratio;
                Test, Unexpected
            ));
        }
        
        Ok(())
    }
    
    #[test]
    fn test_symmetric_links() -> Outcome<()> {
        // Test symmetric mode.
        let mut config = test_config(10);
        config.link_mode = LinkMode::Symmetric;
        
        let graph = res!(generate_social_network(config));
        
        // Verify symmetric mode creates truly symmetric relationships.
        
        // Check that links are symmetric (same circle type in both directions).
        let mut symmetric_count = 0;
        let mut total_edges = 0;
        
        for (node_id, _) in graph.iter_nodes() {
            let outgoing = graph.get_links_from(node_id);
            for (target_id, link) in &outgoing {
                total_edges += 1;
                
                // Check if there's a reverse link.
                let incoming = graph.get_links_to(node_id);
                for (source_id, reverse_link) in &incoming {
                    if source_id == target_id {
                        // Found reverse link - check if it's truly symmetric.
                        // In symmetric mode: both directions should be identical.
                        // If A->B is [Cx -> Cx], then B->A should also be [Cx -> Cx].
                        if link.from_circle == reverse_link.from_circle && 
                           link.to_circle == reverse_link.to_circle &&
                           link.from_circle == link.to_circle {  // Should be same circle.
                            symmetric_count += 1;
                        }
                        break;
                    }
                }
            }
        }
        
        // Most links should be symmetric in symmetric mode.
        let symmetric_ratio = symmetric_count as f64 / total_edges as f64;
        println!("Symmetric links: {}/{} ({:.2}%)", symmetric_count, total_edges, symmetric_ratio * 100.0);
        
        // Should have high symmetry (allow some tolerance for edge cases).
        if symmetric_ratio < 0.8 {
            return Err(err!(
                "Symmetric link ratio too low: {}", symmetric_ratio;
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
            let outgoing = graph.get_links_from(node_id);
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
    fn dump_sample_connections(graph: &DiGraph<PersonId, PersonData, SocialLink>, max_nodes: usize) {
        let mut count = 0;
        for (node_id, _) in graph.iter_nodes() {
            if count >= max_nodes { break; }
            
            let outgoing = graph.get_links_from(node_id);
            let incoming = graph.get_links_to(node_id);
            
            println!("  Node {:?}:", node_id);
            for (target_id, link) in &outgoing {
                print!("    -> {:?}: [C{} -> C{}]", target_id, link.from_circle.0, link.to_circle.0);
                
                // Find reverse link if it exists.
                let mut found_reverse = false;
                for (source_id, reverse_link) in &incoming {
                    if source_id == target_id {
                        println!(" <-> [C{} -> C{}]", reverse_link.from_circle.0, reverse_link.to_circle.0);
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
        let calc_ratio = |graph: &DiGraph<PersonId, PersonData, SocialLink>| -> f64 {
            let mut reciprocal_count = 0;
            let mut total_edges = 0;
            
            for (node_id, _) in graph.iter_nodes() {
                let outgoing = graph.get_links_from(node_id);
                total_edges += outgoing.len();
                
                for (target_id, _) in outgoing {
                    let incoming = graph.get_links_to(node_id);
                    let has_reverse = incoming.iter().any(|(from_id, _)| *from_id == target_id);
                    if has_reverse {
                        reciprocal_count += 1;
                    }
                }
            }
            
            if total_edges > 0 {
                reciprocal_count as f64 / total_edges as f64
            } else {
                0.0
            }
        };
        
        let reciprocal_ratio = calc_ratio(&reciprocal_graph);
        let non_reciprocal_ratio = calc_ratio(&non_reciprocal_graph);
        
        // Reciprocal mode should have higher reciprocal ratio.
        if reciprocal_ratio <= non_reciprocal_ratio {
            return Err(err!(
                "Reciprocal mode ratio ({}) should be > non-reciprocal ratio ({})", 
                reciprocal_ratio, non_reciprocal_ratio;
                Test, Unexpected
            ));
        }
        
        println!("Reciprocal mode ratio: {:.2}", reciprocal_ratio);
        println!("Non-reciprocal mode ratio: {:.2}", non_reciprocal_ratio);
        
        Ok(())
    }
    
    #[test]
    fn test_symmetric_verification() -> Outcome<()> {
        // Verify that symmetric mode creates true Cx -> Cx relationships.
        let mut config = test_config(5);
        config.link_mode = LinkMode::Symmetric;
        
        let graph = res!(generate_social_network(config));
        
        println!("\n=== SYMMETRIC MODE VERIFICATION ===");
        println!("All relationships should be of form [Cx -> Cx]:");
        
        for (node_id, _) in graph.iter_nodes() {
            let outgoing = graph.get_links_from(node_id);
            for (target_id, link) in &outgoing {
                // In symmetric mode, from_circle should equal to_circle.
                if link.from_circle != link.to_circle {
                    return Err(err!(
                        "Non-symmetric link found: [{:?} -> {:?}]", 
                        link.from_circle, link.to_circle;
                        Test, Unexpected
                    ));
                }
                
                // Find reverse link and verify it's identical.
                let incoming = graph.get_links_to(target_id);
                for (source_id, reverse_link) in &incoming {
                    if source_id == &node_id {
                        if reverse_link.from_circle != link.from_circle ||
                           reverse_link.to_circle != link.to_circle {
                            return Err(err!(
                                "Asymmetric reverse link: forward [{:?} -> {:?}], reverse [{:?} -> {:?}]",
                                link.from_circle, link.to_circle,
                                reverse_link.from_circle, reverse_link.to_circle;
                                Test, Unexpected
                            ));
                        }
                        println!("  {:?} <-> {:?}: C{} (symmetric)", node_id, target_id, link.from_circle.0);
                        break;
                    }
                }
            }
        }
        
        println!("✓ All links verified as symmetric (Cx -> Cx)");
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
            
            let outgoing = graph.get_links_from(first_id);
            for (target_id, link_data) in &outgoing {
                let incoming = graph.get_links_to(first_id);
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
            let outgoing = graph.get_links_from(node_id);
            
            for (target_id, _) in outgoing {
                total_edges += 1;
                
                // Check if there's a reverse link.
                let incoming_to_target = graph.get_links_to(target_id);
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
    fn test_sample_range() -> Outcome<()> {
        // Test uniform sampling.
        for _ in 0..100 {
            let val = res!(sample_range(10, 20, SamplingMethod::Uniform));
            if !(val >= 10 && val <= 20) {
                return Err(err!(
                    "Uniform sample {} out of range [10, 20]", val;
                    Test, Unexpected
                ));
            }
        }
        
        // Test Gaussian sampling.
        for _ in 0..100 {
            let val = res!(sample_range(50, 100, SamplingMethod::Gaussian));
            if !(val >= 50 && val <= 100) {
                return Err(err!(
                    "Gaussian sample {} out of range [50, 100]", val;
                    Test, Unexpected
                ));
            }
        }
        
        // Test invalid range.
        match sample_range(20, 10, SamplingMethod::Uniform) {
            Err(_) => Ok(()),
            Ok(_) => Err(err!(
                "Should have failed for invalid range";
                Test, Unexpected
            )),
        }
    }
}
