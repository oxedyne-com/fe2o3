use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_namex::id::InNamex;

pub trait Checksummer:
    Clone
    + std::fmt::Debug
    + InNamex
    + Send
    + Sync
{
    fn len(&self)                                       -> Outcome<usize>;
    fn calculate(self, buf: &[u8])                      -> Outcome<Vec<u8>>;
    fn update(&mut self, buf: &[u8])                    -> Outcome<()>;
    fn finalize(self)                                   -> Outcome<Vec<u8>>;
    fn copy(&self, buf: &[u8])                          -> Outcome<Vec<u8>>;
    fn append(self, buf: Vec<u8>)                       -> Outcome<(Vec<u8>, Vec<u8>)>;
    fn verify(self, buf: &[u8])                         -> Outcome<Vec<u8>>;
    fn read_bytes<R: std::io::Read>(&self, r: &mut R)   -> Outcome<(Vec<u8>, usize)>;
}

