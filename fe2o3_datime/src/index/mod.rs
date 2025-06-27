/// Time indexing and lookup system for efficient time-based queries
/// 
/// This module provides indexing structures for fast time-based lookups,
/// range queries, and temporal data organization.

pub mod time_index;
pub mod range_index;
pub mod temporal_btree;
pub mod time_integer;
pub mod time_basis;

pub use time_index::{TimeIndex, TimeIndexEntry, IndexKey};
pub use range_index::{RangeIndex, RangeQuery, RangeResult};
pub use temporal_btree::{TemporalBTree, TemporalEntry, TemporalQuery};
pub use time_integer::{TimeInteger, TimeLong, TimeBigInt};
pub use time_basis::{
	TimeIndexBasis, UnixTime, JavaTime, NanoTime, CustomTime,
	TimeIndex as GenericTimeIndex, TimeIndexDuration, TimeIndexInterval
};