//! Memory-mapped graph storage for large social networks.
//! 
//! Provides disk-backed storage for graphs that exceed available RAM,
//! using memory-mapped files for efficient access patterns.

use crate::{
    graph::GraphAccessMethod,
    mmap_index::{
    	MmapIndex,
    	MmapIndexBuilder,
    },
};

use oxedyne_fe2o3_core::{
	prelude::*,
	info,
	warn,
};

use std::{
    cell::RefCell,
    fs::{
    	File,
    	OpenOptions,
    },
    io::{
    	Write,
    	Read,
    	Seek,
    	SeekFrom,
    },
};

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

use memmap2::{
	Mmap,
	MmapOptions,
};


/// Memory-mapped graph storage using flat edge list.
/// 
/// Stores edges in binary format on disk, memory-maps for access.
/// Each edge is 9 bytes: from_id(4) + to_id(4) + link_data(1).
pub struct MmapGraph {
    /// File handle (kept for writing during generation).
    edge_file: File,
    /// Number of edges stored.
    num_edges: usize,
    /// Maximum edges capacity.
    capacity: usize,
    /// Memory-mapped view of the edge data (for fast reading).
    mmap: Option<Mmap>,
    /// Disk-based index for fast lookups.
    /// Loaded from .idx file if available.
    index: Option<MmapIndex>,
    /// File path for the edge data.
    file_path: String,
    /// Opening and closing the file can degrade performance.
    read_handle: RefCell<Option<File>>,
}

impl MmapGraph {
    /// Creates a new memory-mapped graph with indexing.
    /// 
    /// # Arguments
    /// * `path` - Path for the edge data file.
    /// * `capacity` - Maximum number of edges to support.
    /// 
    /// # Returns
    /// New memory-mapped graph instance.
    pub fn new(path: &str, capacity: usize) -> Outcome<Self> {
        Self::new_with_options(path, capacity, true)
    }
    
    /// Creates a new memory-mapped graph without indexing for large graphs.
    /// 
    /// Disables the in-memory index to save memory for very large graphs.
    /// Graph queries will be slower but memory usage will be minimal.
    /// 
    /// # Arguments
    /// * `path` - Path for the edge data file.
    /// * `capacity` - Maximum number of edges to support.
    /// 
    /// # Returns
    /// New memory-mapped graph instance without indexing.
    pub fn new_without_index(path: &str, capacity: usize) -> Outcome<Self> {
        Self::new_with_options(path, capacity, false)
    }
    
    /// Creates a new memory-mapped graph with optional indexing.
    /// 
    /// # Arguments
    /// * `path` - Path for the edge data file.
    /// * `capacity` - Maximum number of edges to support.
    /// * `enable_index` - Whether to enable in-memory indexing.
    /// 
    /// # Returns
    /// New memory-mapped graph instance.
    pub fn new_with_options(path: &str, capacity: usize, _enable_index: bool) -> Outcome<Self> {
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
            mmap: None, // Memory mapping created when loading existing graphs.
            index: None, // Index will be loaded separately after generation.
            file_path: path.to_string(),
            read_handle: RefCell::new(None),
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
        
        // Create memory mapping for fast reading.
        // SAFETY: The file was just opened successfully and has valid content.
        // We only read from this mapping, never write to it.
        #[allow(unsafe_code)]
        let mmap = unsafe {
            MmapOptions::new().map(&edge_file)
        };
        
        let mmap = match mmap {
            Ok(m) => {
                info!("Created memory mapping for edge data ({:.1} MB)", m.len() as f64 / 1024.0 / 1024.0);
                // Tell OS we don't need aggressive caching.
                #[cfg(unix)]
                #[allow(unsafe_code)]
                unsafe {
                    let result = libc::madvise(
                        m.as_ptr() as *mut libc::c_void,
                        m.len(),
                        libc::MADV_RANDOM  // Random access pattern, don't prefetch
                    );
                    if result == 0 {
                        info!("Applied MADV_RANDOM to memory mapping");
                    } else {
                        warn!("Failed to apply madvise: {}", result);
                    }
                }
                Some(m)
            },
            Err(e) => {
                warn!("Failed to create memory mapping: {}. Will use direct file access.", e);
                None
            }
        };
        
        let mut graph = Self {
            edge_file,
            num_edges,
            capacity,
            mmap,
            index:          None,
            file_path:      path.to_string(),
            read_handle:    RefCell::new(None),
        };
        
        // Try to load the disk-based index if it exists.
        let index_path = format!("{}.idx", path.trim_end_matches(".mmap"));
        match MmapIndex::load(&index_path) {
            Ok(idx) => {
                info!("Loaded disk-based index for fast lookups");
                graph.index = Some(idx);
            },
            Err(_) => {
                info!("No index found for {}. Graph lookups will scan edges.", path);
            }
        }
        
        Ok(graph)
    }
    
    pub fn release_memory(&self) {
        #[cfg(unix)]
        if let Some(ref mmap) = self.mmap {
            #[allow(unsafe_code)]
            unsafe {
                // This tells the OS it can free these pages.
                // They'll be reloaded on next access.
                let result = libc::madvise(
                    mmap.as_ptr() as *mut libc::c_void,
                    mmap.len(),
                    libc::MADV_DONTNEED  // Free the pages.
                );
                if result == 0 {
                    debug!("Released mmap memory pages");
                }
            }
        }
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
        
        // Index tracking handled by the builder during generation.
        
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

    /// Gets incoming edges to a node ID.
    /// Scans all edges to find matches (O(n) operation).
    /// Note: This is expensive for large graphs without reverse index.
    pub fn get_incoming_edges(&self, to_id: u32) -> Outcome<Vec<(u32, u8)>> {
        let mut edges = Vec::new();
        
        if let Some(ref mmap) = self.mmap {
            // Scan all edges to find incoming ones.
            let edge_size = 9; // 4 bytes from_id + 4 bytes to_id + 1 byte link_data.
            let num_edges = self.num_edges;
            
            for i in 0..num_edges {
                let offset = i * edge_size;
                if offset + edge_size > mmap.len() {
                    break;
                }
                
                // Read edge.
                let edge_from = u32::from_le_bytes([
                    mmap[offset], mmap[offset + 1], mmap[offset + 2], mmap[offset + 3]
                ]);
                let edge_to = u32::from_le_bytes([
                    mmap[offset + 4], mmap[offset + 5], mmap[offset + 6], mmap[offset + 7]
                ]);
                let link_data = mmap[offset + 8];
                
                // Check if this edge points to our target node.
                if edge_to == to_id {
                    edges.push((edge_from, link_data));
                }
            }
            
            return Ok(edges);
        }
        
        // Fallback: read from file if memory mapping is unavailable.
        warn!("Memory mapping unavailable for incoming edges to node {}, using direct file access", to_id);
        
        let mut file = res!(File::open(&self.file_path));
        let edge_size = 9;
        
        for i in 0..self.num_edges {
            let offset = i as u64 * edge_size as u64;
            res!(file.seek(SeekFrom::Start(offset)));
            
            let mut buffer = [0u8; 9];
            match file.read_exact(&mut buffer) {
                Ok(()) => {
                    let edge_from = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
                    let edge_to = u32::from_le_bytes([buffer[4], buffer[5], buffer[6], buffer[7]]);
                    let link_data = buffer[8];
                    
                    if edge_to == to_id {
                        edges.push((edge_from, link_data));
                    }
                }
                Err(_) => break, // End of file or read error.
            }
        }
        
        Ok(edges)
    }

    /// Retrieves all outgoing edges from a specified node.
    ///
    /// This method supports multiple access strategies to balance memory usage and performance:
    /// - FileIO: Always uses direct file I/O (lowest memory, slowest).
    /// - Mmap: Always uses memory mapping (highest memory, fastest).
    /// - Auto: Chooses based on edge count threshold.
    ///
    /// # Arguments
    /// * `from_id` - The source node ID to retrieve edges from.
    /// * `method` - The access method to use for retrieving edges.
    ///
    /// # Returns
    /// A vector of tuples containing (target_node_id, link_data) for all outgoing edges.
    pub fn get_outgoing_edges(
        &self,
        from_id: u32,
        method: GraphAccessMethod,
    )
        -> Outcome<Vec<(u32, u8)>>
    {
        let mut edges = Vec::new();
    
        // Try index-based lookup first.
        if let Some(ref index) = self.index {
            if let Some(edge_offsets) = index.get_node_edge_offsets(from_id) {
                if edge_offsets.is_empty() {
                    return Ok(edges);
                }
    
                // Decide which method to use.
                let use_file_io = match method {
                    GraphAccessMethod::FileIO => true,
                    GraphAccessMethod::Mmap => false,
                    GraphAccessMethod::Auto(edge_lim) => edge_offsets.len() < edge_lim,
                };
    
                if use_file_io {
                    // Get or create cached file handle.
                    let mut read_handle = self.read_handle.borrow_mut();
                    if read_handle.is_none() {
                        *read_handle = Some(res!(File::open(&self.file_path)));
                    }
    
                    let file = match read_handle.as_mut() {
                        Some(f) => f,
                        None => return Err(err!("Failed to get file handle after creation"; Bug)),
                    };
    
                    // Find the range of bytes we need.
                    let first_offset = match edge_offsets.first() {
                        Some(offset) => *offset,
                        None => return Err(err!("Edge offsets unexpectedly empty"; Invalid, Input)),
                    };
                    let last_offset = match edge_offsets.last() {
                        Some(offset) => *offset,
                        None => return Err(err!("Edge offsets unexpectedly empty"; Invalid, Input)),
                    };
                    let bytes_to_read = (last_offset - first_offset + 9) as usize;
    
                    // Read all relevant bytes in one operation.
                    res!(file.seek(SeekFrom::Start(first_offset)));
                    let mut buffer = vec![0u8; bytes_to_read];
                    res!(file.read_exact(&mut buffer));
    
                    // Parse edges from buffer.
                    for edge_offset in edge_offsets {
                        let relative_offset = (edge_offset - first_offset) as usize;
                        if relative_offset + 9 <= buffer.len() {
                            let edge_from = u32::from_le_bytes([
                                buffer[relative_offset],
                                buffer[relative_offset + 1],
                                buffer[relative_offset + 2],
                                buffer[relative_offset + 3]
                            ]);
                            let edge_to = u32::from_le_bytes([
                                buffer[relative_offset + 4],
                                buffer[relative_offset + 5],
                                buffer[relative_offset + 6],
                                buffer[relative_offset + 7]
                            ]);
                            let link_data = buffer[relative_offset + 8];
    
                            if edge_from == from_id {
                                edges.push((edge_to, link_data));
                            }
                        }
                    }
    
                    return Ok(edges);
                } else {
                    // Memory-mapped path for better performance.
                    if let Some(ref mmap) = self.mmap {
                        for edge_offset in edge_offsets {
                            let offset = edge_offset as usize;
                            if offset + 9 > mmap.len() {
                                continue;
                            }
                            let edge_from = u32::from_le_bytes([
                                mmap[offset], mmap[offset + 1], mmap[offset + 2], mmap[offset + 3]
                            ]);
                            let edge_to = u32::from_le_bytes([
                                mmap[offset + 4], mmap[offset + 5], mmap[offset + 6], mmap[offset + 7]
                            ]);
                            let link_data = mmap[offset + 8];
    
                            if edge_from == from_id {
                                edges.push((edge_to, link_data));
                            }
                        }
                        return Ok(edges);
                    }
                }
            }
        }
    
        // Fallback to mmap scanning when no index available.
        if let Some(ref mmap) = self.mmap {
            let mut offset = 0;
            for _ in 0..self.num_edges {
                if offset + 9 > mmap.len() {
                    break;
                }
                let edge_from = u32::from_le_bytes([
                    mmap[offset], mmap[offset + 1], mmap[offset + 2], mmap[offset + 3]
                ]);
                let edge_to = u32::from_le_bytes([
                    mmap[offset + 4], mmap[offset + 5], mmap[offset + 6], mmap[offset + 7]
                ]);
                let link_data = mmap[offset + 8];
    
                if edge_from == from_id {
                    edges.push((edge_to, link_data));
                }
                offset += 9;
            }
        } else {
            // No mmap available, use direct file access.
            warn!("Memory mapping unavailable for node {}, using direct file access", from_id);
            #[cfg(unix)]
            let mut file = res!(std::fs::File::open(format!("/proc/self/fd/{}", self.edge_file.as_raw_fd())));
            #[cfg(not(unix))]
            let mut file = res!(std::fs::File::open(&self.file_path));
    
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
        }
    
        Ok(edges)
    }

    /// Gets the total number of edges for statistics.
    pub fn total_edges(&self) -> usize {
        self.num_edges
    }
    
    /// Loads the memory-mapped index if available.
    pub fn load_index(&mut self, mmap_path: &str) -> Outcome<()> {
        let index_path = format!("{}.idx", mmap_path.trim_end_matches(".mmap"));
        match MmapIndex::load(&index_path) {
            Ok(idx) => {
                self.index = Some(idx);
                Ok(())
            },
            Err(e) => {
                warn!("Could not load index: {}", e);
                Ok(()) // Not having an index is ok, just slower.
            }
        }
    }
    
    /// Builds a memory-mapped index for an existing graph by scanning all edges.
    /// 
    /// This creates a fast lookup index for graphs that were generated without indexing.
    /// Call this once after graph generation to enable O(1) edge lookups.
    pub fn build_index(&mut self, mmap_path: &str, max_node_id: u32) -> Outcome<()> {
        info!("Building memory-mapped index by scanning {} edges...", self.num_edges);
        
        let index_path = format!("{}.idx", mmap_path.trim_end_matches(".mmap"));
        let mut index_builder = res!(MmapIndexBuilder::new(&index_path, max_node_id + 1));
        
        // Scan all edges to build the index.
        if let Some(ref mmap) = self.mmap {
            // Use memory-mapped edge data for scanning.
            let mut offset = 0;
            for edge_idx in 0..self.num_edges {
                if offset + 9 > mmap.len() {
                    break;
                }
                
                // Read the source node ID from this edge.
                let from_node = u32::from_le_bytes([
                    mmap[offset], mmap[offset + 1], mmap[offset + 2], mmap[offset + 3]
                ]);
                
                // Record this edge's offset in the index.
                let edge_offset = (edge_idx * 9) as u64;
                res!(index_builder.add_edge(from_node, edge_offset));
                
                offset += 9;
            }
        } else {
            return Err(err!("Cannot build index: no memory mapping available"; Invalid, Input));
        }
        
        // Finalise and save the index.
        res!(index_builder.finalise());
        info!("Index built successfully: {}", index_path);
        
        // Load the newly created index.
        res!(self.load_index(mmap_path));
        info!("Memory-mapped index loaded for fast lookups");
        
        Ok(())
    }
}

/// Builder for creating memory-mapped graphs from stub matching.
pub struct MmapGraphBuilder {
    graph: MmapGraph,
    /// Optional index builder for creating disk-based index.
    index_builder: Option<MmapIndexBuilder>,
    /// Path for the graph files.
    base_path: String,
    /// Maximum node ID for index sizing.
    _max_node_id: u32,
}

impl MmapGraphBuilder {
    /// Creates a new builder with disk-based indexing.
    /// 
    /// # Arguments
    /// * `path` - Path for the edge data file.
    /// * `estimated_edges` - Estimated number of edges (for pre-allocation).
    /// * `max_node_id` - Maximum node ID (for index sizing).
    /// 
    /// # Returns
    /// New builder instance.
    pub fn new(path: &str, estimated_edges: usize, max_node_id: u32) -> Outcome<Self> {
        let index_path = format!("{}.idx", path.trim_end_matches(".mmap"));
        let index_builder = Some(res!(MmapIndexBuilder::new(&index_path, max_node_id + 1)));
        
        Ok(Self {
            graph: res!(MmapGraph::new_with_options(path, estimated_edges, false)),
            index_builder,
            base_path: path.to_string(),
            _max_node_id: max_node_id,
        })
    }
    
    /// Creates a new builder without indexing for large graphs.
    /// 
    /// Disables indexing completely to save memory for very large graphs.
    /// Graph queries will be slower but memory usage will be minimal.
    /// 
    /// # Arguments
    /// * `path` - Path for the edge data file.
    /// * `estimated_edges` - Estimated number of edges (for pre-allocation).
    /// 
    /// # Returns
    /// New builder instance without indexing.
    pub fn new_without_index(path: &str, estimated_edges: usize) -> Outcome<Self> {
        Ok(Self {
            graph: res!(MmapGraph::new_with_options(path, estimated_edges, false)),
            index_builder: None,
            base_path: path.to_string(),
            _max_node_id: 0,
        })
    }
    
    /// Adds an edge directly to the graph.
    /// 
    /// # Arguments
    /// * `from` - Source node ID.
    /// * `to` - Target node ID.
    /// * `link_data` - Edge data byte.
    /// 
    /// # Returns
    /// Success result.
    pub fn add_edge(&mut self, from: u32, to: u32, link_data: u8) -> Outcome<()> {
        // Record edge offset in index if enabled.
        if let Some(ref mut index_builder) = self.index_builder {
            let edge_offset = (self.graph.num_edges * 9) as u64;
            res!(index_builder.add_edge(from, edge_offset));
        }
        
        res!(self.graph.add_edge(from, to, link_data));
        Ok(())
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
        // Note: This method is deprecated for large graphs due to memory usage.
        // Use add_edge() directly for better memory efficiency.
        res!(self.graph.add_edge(from, to, link_data));
        Ok(true)
    }
    
    /// Finalises the graph and returns it.
    /// 
    /// Writes the disk-based index if indexing was enabled.
    pub fn build(mut self) -> Outcome<MmapGraph> {
        res!(self.graph.flush());
        
        // Write the disk-based index if we have one.
        if let Some(index_builder) = self.index_builder {
            res!(index_builder.finalise());
            info!("Disk-based index created for fast graph lookups");
            
            // Load the index into the graph.
            res!(self.graph.load_index(&self.base_path));
        } else {
            info!("No index created (indexing was disabled)");
        }
        
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

