use crate::base::constant;

/// Fixed-length hash used to deterministically select the owning cache bot and zone.
pub type ChooseHash = [u8; constant::CACHE_HASH_BYTES];
/// Unsigned integer form of a [`ChooseHash`], used in modular bot selection.
pub type ChooseHashUint = u32;
