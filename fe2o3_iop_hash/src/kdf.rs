use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_namex::id::InNamex;

pub trait KeyDeriver:
    Clone
    + std::fmt::Debug
    + InNamex
    + Send
    + Sync
{
    fn get_hash(&self)                              -> Outcome<&[u8]>;
    fn set_rand_salt(&mut self, n: usize)           -> Outcome<()>;
    fn derive(&mut self, pass: &[u8])               -> Outcome<()>;
    fn verify(&self, pass: &[u8])                   -> Outcome<bool>;
    // String encoding.
    fn encode_to_string(&self)                      -> Outcome<String>;
    fn encode_cfg_to_string(&self)                  -> Outcome<String>; // Encoded string sans hash
    fn decode_from_string(&mut self, s: &str)       -> Outcome<()>;
    fn decode_cfg_from_string(&mut self, s: &str)   -> Outcome<()>;
}
