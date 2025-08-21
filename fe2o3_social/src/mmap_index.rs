//! Memory-mapped index for efficient graph lookups.
//! 
//! Provides O(1) access to node edges using memory-mapped files.
//! OS manages memory paging, keeping only accessed portions in RAM.

use oxedyne_fe2o3_core::prelude::*;

use std::fs::{
	File,
	OpenOptions,
};
use std::io::Write;
use std::path::Path;

use memmap2::{
	Mmap,
	MmapOptions,
};

/// Magic number identifying valid index files.
const INDEX_MAGIC: u32 = 0x4944584D; // "IDXM" in hex.

/// Current index format version.
const INDEX_VERSION: u32 = 1;

/// Header size in bytes (magic + version + node_count + reserved).
const HEADER_SIZE: usize = 16;

/// Size of each directory entry in bytes (node_offset + edge_count + reserved).
const DIRECTORY_ENTRY_SIZE: usize = 12;

/// Memory-mapped index for graph edge lookups.
/// 
/// Format:
/// - Header: magic(4) + version(4) + node_count(4) + reserved(4)
/// - Directory: array of (data_offset(4) + edge_count(4) + reserved(4)) per node
/// - Data: variable-length arrays of edge_offset(8) values
pub struct MmapIndex {
    /// Memory-mapped index data.
    mmap: Mmap,
    /// Number of nodes in the index.
    node_count: u32,
}

impl MmapIndex {
    /// Loads an existing index file.
    /// 
    /// # Arguments
    /// * `path` - Path to the index file.
    /// 
    /// # Returns
    /// Loaded index or error if file invalid/missing.
    pub fn load(path: &str) -> Outcome<Self> {
        if !Path::new(path).exists() {
            return Err(err!("Index file not found: {}", path; Missing, File));
        }
        
        let file = res!(File::open(path));
        let metadata = res!(file.metadata());
        
        if metadata.len() < HEADER_SIZE as u64 {
            return Err(err!("Index file too small: {}", path; Invalid, Format));
        }
        
        // Create memory mapping.
        // SAFETY: File was opened successfully and has valid length.
        #[allow(unsafe_code)]
        let mmap = res!(unsafe { MmapOptions::new().map(&file) });
        
        // Validate header from memory-mapped data.
        let magic = u32::from_le_bytes([mmap[0], mmap[1], mmap[2], mmap[3]]);
        if magic != INDEX_MAGIC {
            return Err(err!("Invalid index magic number"; Invalid, Format));
        }
        
        let version = u32::from_le_bytes([mmap[4], mmap[5], mmap[6], mmap[7]]);
        if version != INDEX_VERSION {
            return Err(err!("Unsupported index version: {}", version; Invalid, Version));
        }
        
        let node_count = u32::from_le_bytes([mmap[8], mmap[9], mmap[10], mmap[11]]);
        
        // Validate minimum file size (header + directory).
        let min_size = HEADER_SIZE + (node_count as usize * DIRECTORY_ENTRY_SIZE);
        if metadata.len() < min_size as u64 {
            return Err(err!("Index file too small: expected at least {} bytes", min_size; Invalid, Format));
        }
        
        Ok(Self { mmap, node_count })
    }
    
    /// Gets all edge offsets for a node.
    /// 
    /// # Arguments
    /// * `node_id` - The node ID to look up.
    /// 
    /// # Returns
    /// Vector of edge offsets in the edge file, or None if node doesn't exist.
    pub fn get_node_edge_offsets(&self, node_id: u32) -> Option<Vec<u64>> {
        if node_id >= self.node_count {
            return None;
        }
        
        // Calculate offset to this node's directory entry.
        let dir_offset = HEADER_SIZE + (node_id as usize * DIRECTORY_ENTRY_SIZE);
        
        if dir_offset + DIRECTORY_ENTRY_SIZE > self.mmap.len() {
            return None;
        }
        
        // Read directory entry: data_offset(4) + edge_count(4) + reserved(4).
        let data_offset = u32::from_le_bytes([
            self.mmap[dir_offset], self.mmap[dir_offset + 1], 
            self.mmap[dir_offset + 2], self.mmap[dir_offset + 3],
        ]) as usize;
        
        let edge_count = u32::from_le_bytes([
            self.mmap[dir_offset + 4], self.mmap[dir_offset + 5], 
            self.mmap[dir_offset + 6], self.mmap[dir_offset + 7],
        ]) as usize;
        
        // If no edges, return empty vector.
        if edge_count == 0 {
            return Some(Vec::new());
        }
        
        // Validate data bounds.
        let data_end = data_offset + (edge_count * 8); // 8 bytes per u64 offset
        if data_end > self.mmap.len() {
            return None;
        }
        
        // Read edge offsets from data section.
        let mut edge_offsets = Vec::with_capacity(edge_count);
        for i in 0..edge_count {
            let offset_pos = data_offset + (i * 8);
            let edge_offset = u64::from_le_bytes([
                self.mmap[offset_pos], self.mmap[offset_pos + 1], 
                self.mmap[offset_pos + 2], self.mmap[offset_pos + 3],
                self.mmap[offset_pos + 4], self.mmap[offset_pos + 5], 
                self.mmap[offset_pos + 6], self.mmap[offset_pos + 7],
            ]);
            edge_offsets.push(edge_offset);
        }
        
        Some(edge_offsets)
    }
    
    /// Gets the total number of nodes.
    pub fn node_count(&self) -> u32 {
        self.node_count
    }
}

/// Builder for creating memory-mapped indices during graph generation.
pub struct MmapIndexBuilder {
    /// Path for the index file.
    file_path: String,
    /// Number of nodes to index.
    node_count: u32,
    /// Tracks edge offsets per node.
    node_edges: Vec<Vec<u64>>,
}

impl MmapIndexBuilder {
    /// Creates a new index builder.
    /// 
    /// # Arguments
    /// * `path` - Path for the index file.
    /// * `node_count` - Total number of nodes to index.
    /// 
    /// # Returns
    /// New builder instance.
    pub fn new(path: &str, node_count: u32) -> Outcome<Self> {
        Ok(Self {
            file_path: path.to_string(),
            node_count,
            node_edges: vec![Vec::new(); node_count as usize],
        })
    }
    
    /// Records an edge for a node.
    /// 
    /// # Arguments
    /// * `from_node` - Source node ID.
    /// * `edge_offset` - Offset of this edge in the edge file.
    pub fn add_edge(&mut self, from_node: u32, edge_offset: u64) -> Outcome<()> {
        if from_node >= self.node_count {
            return Err(err!("Node ID {} exceeds node count {}", from_node, self.node_count; Invalid, Input));
        }
        
        self.node_edges[from_node as usize].push(edge_offset);
        Ok(())
    }
    
    /// Finalises the index by writing all data to disk.
    /// 
    /// Must be called after all edges have been added.
    pub fn finalise(self) -> Outcome<()> {
        let mut file = res!(OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.file_path));
        
        // Calculate file size.
        let directory_size = self.node_count as usize * DIRECTORY_ENTRY_SIZE;
        let total_edges: usize = self.node_edges.iter().map(|edges| edges.len()).sum();
        let data_size = total_edges * 8; // 8 bytes per u64 offset
        let file_size = HEADER_SIZE + directory_size + data_size;
        
        // Pre-allocate file space.
        res!(file.set_len(file_size as u64));
        
        // Write header.
        res!(file.write_all(&INDEX_MAGIC.to_le_bytes()));
        res!(file.write_all(&INDEX_VERSION.to_le_bytes()));
        res!(file.write_all(&self.node_count.to_le_bytes()));
        res!(file.write_all(&[0u8; 4])); // Reserved.
        
        // Calculate data section start.
        let data_start = HEADER_SIZE + directory_size;
        let mut current_data_offset = data_start;
        
        // Write directory entries and collect data to write.
        let mut data_buffer = Vec::with_capacity(data_size);
        
        for node_id in 0..self.node_count {
            let edges = &self.node_edges[node_id as usize];
            let edge_count = edges.len() as u32;
            
            // Write directory entry: data_offset(4) + edge_count(4) + reserved(4).
            res!(file.write_all(&(current_data_offset as u32).to_le_bytes()));
            res!(file.write_all(&edge_count.to_le_bytes()));
            res!(file.write_all(&[0u8; 4])); // Reserved.
            
            // Add edge offsets to data buffer.
            for &edge_offset in edges {
                data_buffer.extend_from_slice(&edge_offset.to_le_bytes());
            }
            
            // Update data offset for next node.
            current_data_offset += edges.len() * 8;
        }
        
        // Write all edge offset data.
        res!(file.write_all(&data_buffer));
        res!(file.flush());
        
        Ok(())
    }
}