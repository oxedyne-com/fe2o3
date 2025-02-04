use crate::{
    prelude::*,
    bots::worker::bot::WorkerType,
};

use oxedize_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
    chunk::{
        Chunker,
        ChunkConfig,
    },
    id::NumIdDat,
};

use std::{
    collections::BTreeMap,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
    time::Duration,
};

use num_cpus;


/// Use these as format strings in macros.
#[macro_export]
macro_rules! format_zones_dir { () => { "{:03}_zone" } }
#[macro_export]
macro_rules! format_zone_dir { () => { "zone_{:03}" } }
#[macro_export]
macro_rules! format_data_file { () => { "{:09}" } } // Further separated like 000_000_000 in file/zdir.rs
#[macro_export]
macro_rules! regex_data_file { () => { r"\d{3}_\d{3}_\d{3}" } }

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL>,
    ENC:    Encrypter,
    KH:     Hasher,
    PR:     Hasher,
    CS:     Checksummer,
>
    O3db<UIDL, UID, ENC, KH, PR, CS>
{
    // System reflection.
    pub fn num_cpus() -> usize {
        num_cpus::get()
    }
}

/// Holds primary configuration information for an ozone database in a Rust struct that can be
/// read and written directly to file.  The `zones` field must be a `Vec<Daticle>` because
/// `FromDatMap` and `ToDatMap` currently do not handle recursion.
#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct OzoneConfig {
    // Key hashing
    pub bytes_before_hashing:           u64, // applies only to keys
    // Caches
    pub cache_size_limit_bytes:         u64,
    pub init_load_caches:               bool,
    // Files
    pub data_file_max_bytes:            u64,
    // Chunking
    pub rest_chunk_threshold:           u64, // applies only to values
    pub rest_chunk_bytes:               u64,
    // Bots
    pub num_cbots_per_zone:             u16, // cache bots
    pub num_fbots_per_zone:             u16, // file bots
    pub num_igbots_per_zone:            u16, // init and garbage collecting bots
    pub num_rbots_per_zone:             u16, // reader bots
    pub num_wbots_per_zone:             u16, // writer bots
    pub num_sbots:                      u16, // server bots
    // Zones
    pub num_zones:                      u16,
    pub zone_state_update_secs:         u8, 
    pub zone_overrides:                 BTreeMap<Dat, Dat>,
}

impl Config for OzoneConfig {

    /// Performs a sequence of checks of the configuration data.
    fn check_and_fix(&mut self) -> Outcome<()> {
        res!(self.check_rest_chunk_config(&self.chunk_config()));
        res!(self.check_file_size());
        Ok(())
    }
}

impl Default for OzoneConfig {
    fn default() -> Self {
        Self {
            // Key hashing
            bytes_before_hashing:           32,
            // Caches
            cache_size_limit_bytes:         1_073_742_000, // 1 GiB
            init_load_caches:               true,
            // Files
            data_file_max_bytes:            1_048_576, // 1 MiB
            // Chunking
            rest_chunk_threshold:           716_800, // 700 KiB,
            rest_chunk_bytes:               102_400, // 100 KiB
            // Bots
            num_cbots_per_zone:             2,
            num_fbots_per_zone:             2,
            num_igbots_per_zone:            2,
            num_rbots_per_zone:             2,
            num_wbots_per_zone:             2,
            num_sbots:                      2,
            // Zones
            num_zones:                      2,
            zone_state_update_secs:         5, 
            zone_overrides:                 mapdat!{ // use mapdat! for convenience.
                                                1u16 => mapdat!{
                                                    "dir" => "",
                                                    "max_size" => 104_857_600u64,
                                                },
                                                2u16 => mapdat!{
                                                    "dir" => "",
                                                    "max_size" => 104_857_600u64,
                                                },
                                            }.get_map().unwrap(),
        }
    }
}

impl OzoneConfig {
    pub fn rest_chunk_size(&self)           -> usize { self.rest_chunk_bytes as usize }
    pub fn rest_chunking_threshold(&self)   -> usize { self.rest_chunk_threshold as usize }
    pub fn hashing_threshold(&self)         -> usize { self.bytes_before_hashing as usize }

    pub fn num_zones(&self) -> usize { self.num_zones as usize }
    pub fn num_cbots_per_zone(&self) -> usize { self.num_cbots_per_zone as usize }
    pub fn num_fbots_per_zone(&self) -> usize { self.num_fbots_per_zone as usize }
    pub fn num_sbots(&self) -> usize { self.num_sbots as usize }

    pub fn num_caches(&self) -> usize {
        let nz = self.num_zones as usize;
        let nbots = self.num_cbots_per_zone as usize;
        nz * nbots
    }

    pub fn num_filemaps(&self) -> usize {
        let nz = self.num_zones as usize;
        let nbots = self.num_fbots_per_zone as usize;
        nz * nbots
    }

    pub fn num_wbots(&self) -> usize {
        let nz = self.num_zones as usize;
        let nbots = self.num_wbots_per_zone as usize;
        nz * nbots
    }

    pub fn num_bots_per_zone(&self, wtyp: &WorkerType) -> usize {
        (match wtyp {
            WorkerType::Cache       => self.num_cbots_per_zone,
            WorkerType::File        => self.num_fbots_per_zone,
            WorkerType::InitGarbage => self.num_igbots_per_zone,
            WorkerType::Reader      => self.num_rbots_per_zone,
            WorkerType::Writer      => self.num_wbots_per_zone,
        }) as usize
    }

    pub fn zone_config(&self) -> ZoneConfig {
        let nz = self.num_zones();
        let nc = self.num_cbots_per_zone as usize;
        let nf = self.num_fbots_per_zone as usize;

        let total_num_caches = self.num_caches();
        let cache_lim = self.cache_size_limit_bytes as usize;
        let per_cache_lim = cache_lim / total_num_caches;
        info!(sync_log::stream(), 
            "Total size limit for all caches, {} [B], will be used to set the size \
            limit of {} [B] for {} caches across {} zones.",
            cache_lim, per_cache_lim, total_num_caches, nz,
        );
        info!(sync_log::stream(), 
            "File data will {}be loaded into caches upon initialisation.",
            if self.init_load_caches { "" } else { "not" },
        );
        ZoneConfig {
            ncbots:             nc,
            nfbots:             nf,
            cache_size_lim:     per_cache_lim,
            init_load_caches:   self.init_load_caches,
        }
    }

    pub fn zone_overrides(&self) -> &BTreeMap<Dat, Dat> { &self.zone_overrides }

    pub fn zone_state_update_interval(&self) -> Duration {
        Duration::from_secs(self.zone_state_update_secs as u64)
    }

    pub fn zone_state_update_interval_secs(&self) -> u64 {
        self.zone_state_update_secs as u64
    }

    pub fn check_zone_index(&self, z: usize) -> Outcome<()> {
        let nz = self.num_zones as usize;
        if z >= nz {
            Err(err!(
                "{} cannot be used to index a zone, with {} zone(s) at present.", z, nz;
                Configuration))
        } else {
            Ok(())
        }
    }

    pub fn write_config_file(&self, db_root: &Path) -> Outcome<()> {
        let path = Self::config_path(db_root);
        let mut file = res!(std::fs::File::create(&path));
        let dat = Self::to_datmap(self.clone());
        for mut line in dat.to_lines("    ", true) {
            line.push_str("\n");
            res!(file.write(line.as_bytes()));
        }
        debug!(sync_log::stream(), "O3db configuration written to {:?}: ", path);
        for line in dat.to_lines("    ", true) {
            debug!(sync_log::stream(), "{}", line);
        }
        Ok(())
    }

    // Path generation.

    /// Ensures that the configured database container path is an absolute, comparable path.
    pub fn canonicalize_path(pbuf: PathBuf) -> Outcome<String> {
        let canonical = match (&pbuf).canonicalize() {
            Ok(pbuf) => pbuf,
            Err(e) => return Err(err!(e,
                "The path {:?} does not exist and must be created.", pbuf;
                Configuration, Invalid, Input)),
        };
        match canonical.into_os_string().into_string() {
            Ok(s) => Ok(s),
            _ => return Err(err!(
                "Could not canonicalize {:?}, it may contain non-UTF-8 encoding.", pbuf;
                Configuration, String, Conversion)),
        }
    }

    pub fn zone_root(&self, container: &Path) -> PathBuf {
        self.append_zone_root(PathBuf::from(container))
    }

    pub fn config_path(db_root: &Path) -> PathBuf { 
        let mut path = PathBuf::from(db_root);
        path.push(constant::CONFIG_FILENAME);
        path
    }

    pub fn append_zone_root(&self, mut path: PathBuf) -> PathBuf {
        path.push(fmt!(format_zones_dir!(), self.num_zones));
        path
    }

    // Chunking.
    
    /// Return a rest chunk configuration from the database configuration.
    pub fn chunk_config(&self) -> ChunkConfig {
        ChunkConfig {
            threshold_bytes:    self.rest_chunk_threshold as usize,
            chunk_size:         self.rest_chunk_bytes as usize, 
            dat_wrap:           true,
            pad_last:           true,
        }
    }

    pub fn chunker(cfg: ChunkConfig) -> Chunker {
        Chunker::default().set_config(cfg)
    }

    pub fn check_rest_chunk_config(&self, chunk_cfg: &ChunkConfig) -> Outcome<()> {
        let max_file_size = self.data_file_max_bytes as f64;
        let chunk_threshold = chunk_cfg.threshold_bytes as f64;
        if chunk_threshold >= constant::MAX_FILE_TO_CHUNKING_THRESHOLD_RATIO * max_file_size {
            return Err(err!(
                "The size threshold before file keys and values are chunked, \
                supplied in the configuration, of {} bytes, is too large compared \
                to the specified maximum data file size of {} bytes. The ratio \
                should not exceed {:.1}%.",
                chunk_cfg.threshold_bytes, self.data_file_max_bytes,
                constant::MAX_FILE_TO_CHUNKING_THRESHOLD_RATIO * 100.0;
                TooBig, Configuration));
        }
        if chunk_cfg.chunk_size < constant::MIN_CHUNK_SIZE {
            return Err(err!(
                "Chunk size of {} is less than the current minimum of {}.",
                chunk_cfg.chunk_size, constant::MIN_CHUNK_SIZE;
                TooSmall, Configuration));
        }
        let chunk_size = chunk_cfg.chunk_size as f64;
        if chunk_size * constant::MAX_FILE_TO_CHUNK_SIZE_RATIO >= max_file_size {
            return Err(err!(
                "The chunk size, supplied in the configuration, of {} bytes, is \
                too large compared to the specified maximum data file size of {} \
                bytes. The ratio should not exceed {:.1}%.",
                chunk_cfg.chunk_size, self.data_file_max_bytes,
                constant::MAX_FILE_TO_CHUNK_SIZE_RATIO * 100.0;
                TooBig, Configuration));
        }
        Ok(())
    }

    pub fn check_file_size(&self) -> Outcome<()> {
        if self.data_file_max_bytes > constant::MAX_FILE_BYTES {
            return Err(err!(
                "The configured maximum data file size in bytes of {} is too big, \
                for safety it cannot exceed {}.",
                self.data_file_max_bytes, constant::MAX_FILE_BYTES;
                Invalid, Input));
        }
        Ok(())
    }

}

#[derive(Clone, Debug, Default)]
pub struct ZoneConfig {
    pub ncbots:             usize,
    pub nfbots:             usize,
    pub cache_size_lim:     usize,
    pub init_load_caches:   bool,
}
