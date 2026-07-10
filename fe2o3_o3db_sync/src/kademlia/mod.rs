//! A Kademlia DHT routing-table primitive for the Hematite distributed Ozone
//! layer.
//!
//! This crate is the *routing* layer only -- pure data structures and
//! algorithms. It has no transport, no asynchrony, no I/O. Higher layers
//! (Shield transport, OAM allocation, IBLT anti-entropy, HotStuff consensus)
//! depend on it but are kept in their own crates.
//!
//! # Identifier space
//!
//! Node identifiers are 256 bits. Distance is XOR -- see [`id::NodeId`] and
//! [`id::Distance`]. A node's routing table holds 256 k-maps, one per bit of
//! XOR distance from the local node; the k-map for bit `i` holds peers whose
//! distance from the local node is in `[2^i, 2^(i+1))`.
//!
//! # Replacement
//!
//! Each k-map holds at most `k` contacts. When a new contact arrives for a
//! full bucket the routing table does *not* evict unilaterally -- it returns
//! the current LRU as a candidate and waits for the caller to probe it.
//! Kademlia's LRU-biased policy is that a responsive LRU stays and the new
//! contact is dropped; only a confirmed-dead LRU is replaced. This crate
//! encodes that contract through [`kmap::InsertOutcome`] and
//! [`table::RoutingTable::keep_lru`] / [`table::RoutingTable::evict_and_insert`].
//!
//! # What this crate does not do
//!
//! - Send or receive any messages.
//! - Authenticate peers. Public keys, key rotation and signature verification
//!   belong to fe2o3_crypto and the distributed Ozone layer above this one.
//! - Store data records. That role belongs to OAM and the local Ozone engine.
//! - Maintain bucket-refresh schedules or iterative lookup state. Those are
//!   higher-layer concerns -- they consume [`table::RoutingTable::k_closest`]
//!   but own their own state.
//!
//! # Example
//!
//! ```
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_o3db_sync::kademlia::{
//!     contact::Contact,
//!     id::NodeId,
//!     table::RoutingTable,
//! };
//! use std::net::SocketAddr;
//!
//! # fn main() -> Outcome<()> {
//! let me = NodeId::from_bytes([0u8; 32]);
//! let mut table = res!(RoutingTable::new(me, 20));
//!
//! let mut peer_id_bytes = [0u8; 32];
//! peer_id_bytes[31] = 1;
//! let peer_id = NodeId::from_bytes(peer_id_bytes);
//! let addr: SocketAddr = res!("127.0.0.1:60000".parse());
//! let contact = Contact::new(peer_id, vec![addr]);
//!
//! let _ = res!(table.insert(contact));
//! assert_eq!(table.len(), 1);
//! # Ok(())
//! # }
//! ```
pub mod contact;
pub mod id;
pub mod kmap;
pub mod table;
