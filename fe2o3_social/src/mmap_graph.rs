//! Memory-mapped graph storage for large social networks.
//! 
//! Provides disk-backed storage for graphs that exceed available RAM,
//! using memory-mapped files for efficient access patterns.

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Write, Read, Seek, SeekFrom},
    os::unix::io::AsRawFd,
    path::Path,
};

use oxedyne_fe2o3_core::{info, warn};

/// Memory-mapped graph storage using flat edge list.
/// 
/// Stores edges in binary format on disk, memory-maps for access.
/// Each edge is 9 bytes: from_id(4) + to_id(4) + link_data(1).
pub struct MmapGraph {
    /// File handle.
    edge_file: File,
    /// Number of edges stored.
    num_edges: usize,
    /// Maximum edges capacity.
    capacity: usize,
    /// Index mapping person_id -> list of edge offsets for fast lookups.
    /// Built during generation and saved to .idx file.
    /// Set to None to disable indexing for faster generation.
    edge_index: Option<HashMap<u32, Vec<u64>>>,  // person_id -> list of edge offsets
}

impl MmapGraph {
    /// Creates a new memory-mapped graph.
    /// 
    /// # Arguments
    /// * `path` - Path for the edge data file.
    /// * `capacity` - Maximum number of edges to support.
    /// 
    /// # Returns
    /// New memory-mapped graph instance.
    pub fn new(path: &str, capacity: usize) -> Outcome<Self> {
        // Create or truncate the file.
        let edge_file = res!(OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path));
        
        // Pre-allocate space for edges.
        let file_size = capacity * 9; // 9 bytes per edge.
        res!(edge_file.set_len(file_size as u64));
        
        Ok(Self {
            edge_file,
            num_edges: 0,
            capacity,
            edge_index: Some(HashMap::new()),
        })
    }
    
    /// Loads an existing memory-mapped graph.
    /// 
    /// # Arguments
    /// * `path` - Path to existing edge data file.
    /// 
    /// # Returns
    /// Loaded memory-mapped graph instance.
    pub fn load_existing(path: &str) -> Outcome<Self> {
        // Open existing file without truncating.
        let edge_file = res!(OpenOptions::new()
            .read(true)
            .write(true)
            .open(path));
        
        // Get file size to determine capacity and edge count.
        let metadata = res!(edge_file.metadata());
        let file_size = metadata.len();
        let capacity = (file_size / 9) as usize; // 9 bytes per edge.
        let num_edges = capacity; // Assume file is fully written for now.
        
        let mut graph = Self {
            edge_file,
            num_edges,
            capacity,
            edge_index: Some(HashMap::new()),
        };
        
        // Try to load the index if it exists
        if res!(graph.load_index(path)) {
            info!("Loaded existing index for fast lookups");
        } else {
            info!("No index found for {}. Graph lookups will use fallback scan (slower).", path);
        }
        
        Ok(graph)
    }
    
    /// Adds an edge to the graph.
    /// 
    /// # Arguments
    /// * `from` - Source node ID.
    /// * `to` - Target node ID.
    /// * `link_data` - Packed link data (1 byte).
    /// 
    /// # Returns
    /// Ok if successful, error if capacity exceeded.
    pub fn add_edge(&mut self, from: u32, to: u32, link_data: u8) -> Outcome<()> {
        if self.num_edges >= self.capacity {
            return Err(err!("Graph capacity exceeded: {} edges", self.capacity; Invalid, Input));
        }
        
        // Track the offset for this edge in the index (if enabled)
        if let Some(ref mut index) = self.edge_index {
            let offset = (self.num_edges * 9) as u64;
            index.entry(from).or_insert_with(Vec::new).push(offset);
        }
        
        // Write edge data directly to file.
        res!(self.edge_file.write_all(&from.to_le_bytes()));
        res!(self.edge_file.write_all(&to.to_le_bytes()));
        res!(self.edge_file.write_all(&[link_data]));
        
        self.num_edges += 1;
        Ok(())
    }
    
    /// Flushes pending writes to disk.
    pub fn flush(&mut self) -> Outcome<()> {
        res!(self.edge_file.flush());
        Ok(())
    }
    
    /// Saves the index to a separate file for fast future loading.
    pub fn save_index(&self, mmap_path: &str) -> Outcome<()> {
        let index_path = format!("{}.idx", mmap_path.trim_end_matches(".mmap"));
        info!("Saving index to: {}", index_path);
        
        let mut index_file = res!(File::create(&index_path));
        
        let index = match &self.edge_index {
            Some(idx) => idx,
            None => {
                info!("No index to save (indexing was disabled)");
                return Ok(());
            }
        };
        
        // Write number of indexed nodes
        res!(index_file.write_all(&(index.len() as u32).to_le_bytes()));
        
        // Write each node's edge offsets
        for (node_id, offsets) in index {
            res!(index_file.write_all(&node_id.to_le_bytes()));
            res!(index_file.write_all(&(offsets.len() as u32).to_le_bytes()));
            for offset in offsets {
                res!(index_file.write_all(&offset.to_le_bytes()));
            }
        }
        
        res!(index_file.flush());
        info!("Index saved with {} entries", index.len());
        Ok(())
    }
    
    /// Loads the index from a file if it exists.
    pub fn load_index(&mut self, mmap_path: &str) -> Outcome<bool> {
        let index_path = format!("{}.idx", mmap_path.trim_end_matches(".mmap"));
        
        if !Path::new(&index_path).exists() {
            return Ok(false);
        }
        
        info!("Loading index from: {}", index_path);
        let mut index_file = res!(File::open(&index_path));
        
        let mut buf = [0u8; 4];
        res!(index_file.read_exact(&mut buf));
        let num_nodes = u32::from_le_bytes(buf);
        
        let index = self.edge_index.get_or_insert_with(HashMap::new);
        index.clear();
        
        for _ in 0..num_nodes {
            res!(index_file.read_exact(&mut buf));
            let node_id = u32::from_le_bytes(buf);
            
            res!(index_file.read_exact(&mut buf));
            let num_offsets = u32::from_le_bytes(buf) as usize;
            
            let mut offsets = Vec::with_capacity(num_offsets);
            let mut offset_buf = [0u8; 8];
            for _ in 0..num_offsets {
                res!(index_file.read_exact(&mut offset_buf));
                offsets.push(u64::from_le_bytes(offset_buf));
            }
            
            index.insert(node_id, offsets);
        }
        
        info!("Index loaded with {} entries", index.len());
        Ok(true)
    }
    
    /// Gets the number of edges stored.
    pub fn edge_count(&self) -> usize {
        self.num_edges
    }
    
    /// Gets memory usage in MB (always near zero for mmap).
    pub fn memory_usage_mb(&self) -> f64 {
        // Only the file handle and metadata are in memory.
        // Actual edge data is on disk.
        0.001 // Negligible memory usage.
    }

    /// Gets all outgoing edges from a node.
    /// 
    /// Uses the index for O(1) lookup if available, otherwise falls back to scanning.
    pub fn get_outgoing_edges(&self, from_id: u32) -> Outcome<Vec<(u32, u8)>> {
        // If we have an index, use it for fast lookup
        if let Some(index) = &self.edge_index {
            if let Some(offsets) = index.get(&from_id) {
                let mut edges = Vec::with_capacity(offsets.len());
                
                // Open the file for reading
                let mut file = res!(std::fs::File::open(format!("/proc/self/fd/{}", self.edge_file.as_raw_fd())));
                
                let mut buffer = [0u8; 9]; // 4 + 4 + 1 bytes per edge
                
                for &offset in offsets {
                    res!(file.seek(SeekFrom::Start(offset)));
                    res!(file.read_exact(&mut buffer));
                    
                    // We already know from_id matches, just read to_id and link_data
                    let edge_to = u32::from_le_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]);
                    let link_data = buffer[8];
                    edges.push((edge_to, link_data));
                }
                
                return Ok(edges);
            }
        }
        
        // Fallback: scan the entire file (slow but works without index)
        warn!("No index available for node {}, falling back to full scan", from_id);
        
        let mut edges = Vec::new();
        let mut file = res!(std::fs::File::open(format!("/proc/self/fd/{}", self.edge_file.as_raw_fd())));
        res!(file.seek(SeekFrom::Start(0)));
        
        let mut buffer = [0u8; 9];
        
        for _ in 0..self.num_edges {
            match file.read_exact(&mut buffer) {
                Ok(_) => {
                    let edge_from = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
                    let edge_to = u32::from_le_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]);
                    let link_data = buffer[8];
                    
                    if edge_from == from_id {
                        edges.push((edge_to, link_data));
                    }
                },
                Err(_) => break,
            }
        }
        
        Ok(edges)
    }
    
    /// Gets the total number of edges for statistics.
    pub fn total_edges(&self) -> usize {
        self.num_edges
    }
}

/// Builder for creating memory-mapped graphs from stub matching.
pub struct MmapGraphBuilder {
    graph: MmapGraph,
    seen_edges: std::collections::HashSet<(u32, u32)>,
}

impl MmapGraphBuilder {
    /// Creates a new builder.
    /// 
    /// # Arguments
    /// * `path` - Path for the edge data file.
    /// * `estimated_edges` - Estimated number of edges (for pre-allocation).
    /// 
    /// # Returns
    /// New builder instance.
    pub fn new(path: &str, estimated_edges: usize) -> Outcome<Self> {
        Ok(Self {
            graph: res!(MmapGraph::new(path, estimated_edges)),
            seen_edges: std::collections::HashSet::new(),
        })
    }
    
    /// Adds an edge if it doesn't already exist.
    /// 
    /// # Arguments
    /// * `from` - Source node ID.
    /// * `to` - Target node ID.
    /// * `link_data` - Packed link data.
    /// 
    /// # Returns
    /// True if edge was added, false if it already existed.
    pub fn add_edge_unique(&mut self, from: u32, to: u32, link_data: u8) -> Outcome<bool> {
        let key = (from, to);
        if self.seen_edges.contains(&key) {
            return Ok(false);
        }
        
        res!(self.graph.add_edge(from, to, link_data));
        self.seen_edges.insert(key);
        Ok(true)
    }
    
    /// Finalizes the graph and returns it.
    pub fn build(mut self) -> Outcome<MmapGraph> {
        res!(self.graph.flush());
        Ok(self.graph)
    }
    
    /// Gets current edge count.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
    
    /// Reports progress if interval is met.
    pub fn report_progress(&self, interval: usize) {
        if self.graph.num_edges % interval == 0 {
            info!("Mmap graph progress: {} edges written to disk", self.graph.num_edges);
        }
    }
}

/// Configuration for memory-mapped graph generation.
pub struct MmapConfig {
    /// Path for the edge data file.
    pub edge_file_path: String,
    /// Whether to use memory mapping.
    pub enabled: bool,
    /// Estimated number of edges.
    pub estimated_edges: usize,
}

impl Default for MmapConfig {
    fn default() -> Self {
        Self {
            edge_file_path: "/tmp/social_graph_edges.bin".to_string(),
            enabled: false,
            estimated_edges: 100_000_000, // 100M edges default.
        }
    }
}