use crate::{
    daticle::Dat,
};

use oxedyne_fe2o3_core::prelude::*;

use rand_core::{
    OsRng,
    RngCore,
};


new_type!(PartKey, [u64; 5], Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd);

impl PartKey {
    pub fn set_id(&self)    -> u64 { self.0[0] }
    pub fn index(&self)     -> u64 { self.0[1] }
    pub fn data_len(&self)  -> u64 { self.0[2] }
    pub fn num_parts(&self) -> u64 { self.0[3] }
    pub fn part_size(&self) -> u64 { self.0[4] }
}

#[derive(Clone, Debug, Default)]
pub struct Chunker {
    pub state:  ChunkState,
    pub cfg:    ChunkConfig,
}

impl Chunker {

    pub fn new(chunk_size: usize) -> Self {
        Self {
            cfg: ChunkConfig::default().set_size(chunk_size),
            ..Default::default()
        }
    }

    pub fn set_state(mut self, state: ChunkState) -> Self {
        self.state = state;
        self
    }
    pub fn set_config(mut self, cfg: ChunkConfig) -> Self {
        self.cfg = cfg;
        self
    }
    //pub fn state(&self) -> &ChunkState { &self.state }
    //pub fn config(&self) -> &ChunkConfig { &self.cfg }


    pub fn chunk(
        &self,
        vbuf:   &[u8],
    )
        -> Outcome<(
            Vec<Vec<u8>>,   // The chunks (possibly Dat-wrapped).
            ChunkState,     // Info about the chunks.
        )>
    {
        let data_len = vbuf.len();
        let chunk_size = self.cfg.chunk_size;
        let num_chunks = res!(oxedyne_fe2o3_num::int::usize_ceil_div(
                data_len,
                chunk_size,
        ));
        let chunk_state = ChunkState{
            index:      0,
            data_len,
            num_chunks,
            chunk_size,
        };

        let mut chunks = Vec::with_capacity(num_chunks);
        for (i, chunk) in vbuf.chunks(chunk_size).enumerate() {
            let mut cvbuf = chunk.to_vec();
            if self.cfg.pad_last && (i == num_chunks - 1) && cvbuf.len() < chunk_size {
                let mut padbuf = vec![0u8; chunk_size - cvbuf.len()];
                OsRng.fill_bytes(&mut padbuf);
                cvbuf.extend_from_slice(&padbuf);
            }
            if self.cfg.dat_wrap {
                cvbuf = res!(Dat::wrap_bytes_var(cvbuf));
            }
            chunks.push(cvbuf);
        }

        Ok((chunks, chunk_state))
    }

    pub fn keys(
        &self,
        id:     u64,
        state:  &ChunkState,
    )
        -> Outcome<Vec<Dat>>
    {
        let mut keys = Vec::with_capacity(state.num_chunks);
        keys.push(
            Dat::Tup5u64([
                id,
                0,
                state.data_len as u64,
                state.num_chunks as u64,
                state.chunk_size as u64,
            ])
        );
        
        for i in 0..state.num_chunks {
            let key = Dat::Tup5u64([
                id,
                (i as u64) + 1,
                state.data_len as u64,
                state.num_chunks as u64,
                state.chunk_size as u64,
            ]);
            keys.push(key);
        }

        Ok(keys)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChunkState {
    pub index:      usize,
    pub data_len:   usize,
    pub num_chunks: usize,
    pub chunk_size: usize,
}

impl ChunkState {

    pub fn set_index(mut self, index: usize) -> Self {
        self.index = index;
        self
    }
    pub fn set_num_of_chunks(mut self, num_chunks: usize) -> Self {
        self.num_chunks = num_chunks;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChunkConfig {
    pub threshold_bytes:    usize,
    pub chunk_size:         usize,
    pub dat_wrap:           bool,
    pub pad_last:           bool,
}

impl ChunkConfig {
    pub fn set_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }
    pub fn set_pad_last(mut self, pad_last: bool) -> Self {
        self.pad_last = pad_last;
        self
    }
}
