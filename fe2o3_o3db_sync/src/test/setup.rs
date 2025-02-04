use crate::{
    prelude::*,
    base::cfg::OzoneConfig,
    data::core::RestSchemesInput,
};

use oxedize_fe2o3_jdat::{
    prelude::*,
    id::IdDat,
};

use std::{
    mem,
    path::PathBuf,
    thread,
    time::Duration,
};

pub fn default_cfg() -> Outcome<OzoneConfig> {
    Ok(OzoneConfig {
        // Key hashing
        bytes_before_hashing:           32,
        // Caches
        cache_size_limit_bytes:         100_000_000,
        init_load_caches:               true,
        // Files
        data_file_max_bytes:            2_000,//1_000_000,
        // Chunking
        rest_chunk_threshold:           1_500,
        rest_chunk_bytes:               64,
        // Bots
        num_cbots_per_zone:             2,
        num_fbots_per_zone:             2,
        num_igbots_per_zone:            2,
        num_rbots_per_zone:             2,
        num_wbots_per_zone:             1,
        num_sbots:                      2,
        // Zones
        num_zones:                      2,
        zone_state_update_secs:         1, 
        zone_overrides:                 mapdat!{
                                            1u16    =>  mapdat!{
                                                "dir"       =>  "../test_db_zone_container",
                                                "max_size"  =>  1_000_000u64,
                                            },
                                            //2u16    =>  res!(mapdat!{
                                            //    "dir"       =>  "",
                                            //    "max_size"  =>  100u64,
                                            //}),
                                            3u16    =>  mapdat!{
                                                "dir"       =>  "",
                                                "max_size"  =>  1_000_000u64,
                                            },
                                        }.get_map().unwrap(),
    })
}

pub type UidTyp = u128; // Concrete underlying user id type for testing.
pub const UID_LEN: usize = mem::size_of::<UidTyp>();
pub type Uid = IdDat<{ UID_LEN }, UidTyp>;

pub fn start_db<
	ENC:    Encrypter + 'static,
	KH:     Hasher + 'static,
	PR:     Hasher + 'static,
	CS:     Checksummer + 'static,
>(
    db_root:        PathBuf,
    cfg_opt:        Option<OzoneConfig>,
    schms_input:    RestSchemesInput<ENC, KH, PR, CS>,
    _zone_path:      Option<String>, // create a separate zone container in this directory
    gc_on:          bool,
    wipe:           bool,
)
    -> Outcome<O3db<
        UID_LEN,
        Uid,
        ENC,
        KH,
        PR,
        CS,
    >>
{
    let mut db = res!(O3db::new(
        db_root,
        cfg_opt,
        schms_input,
        Uid::default(),
    ));
    let files = res!(db.find_all_data_files());
    test!(sync_log::stream(), "Found {} existing data and index files.", files.len());
    if wipe {
        for file in files {
            test!(sync_log::stream(), "  Deleting {:?}", file);
            res!(std::fs::remove_file(file));
        }
    }
    test!(sync_log::stream(), "Starting db...");
    res!(db.start("test"));
    res!(ok!(db.updated_api()).activate_gc(gc_on));

    thread::sleep(Duration::from_secs(1));

    // Ping all bots.
    let (start, msgs) = res!(db.api().ping_bots(constant::USER_REQUEST_WAIT));
    test!(sync_log::stream(), "{} ping replies received in {:?}.", msgs.len(), start.elapsed());
    Ok(db)
}

