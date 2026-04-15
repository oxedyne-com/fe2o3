//! Ozone persistence of admin dashboard state.
//!
//! Dashboard history (the Overview sparkline strip) lives in an in-memory
//! ring that is wiped on every Steel restart. That's fine for the live
//! process but makes charts empty after every deploy and makes it harder
//! for the operator to notice slow-moving trends through a restart.
//!
//! This module writes the derived sparkline points to the default vhost's
//! ozone database on a fixed cadence and reads them back at start-up so
//! the Overview strip picks up where the previous run left off.
//!
//! The encoded payload is a [`Dat::Vek`] of [`Dat::Tup5`] rows,
//! `(t_secs: u64, cpu_pct: f64, mem_pct: f64, disk_bps: f64, net_bps: f64)`,
//! stored under a fixed string key with a version suffix so future format
//! changes can migrate gracefully.

use crate::srv::admin::host_sampler::DerivedHostPoint;

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::{
    prelude::*,
    id::NumIdDat,
    try_extract_tup5dat,
    tup5dat,
};

/// Ozone key under which derived host-sampler points are persisted.
/// The `.v1` suffix exists so a future format change can be picked
/// up by bumping to `.v2` without colliding with old entries.
pub const HOST_HISTORY_KEY: &str = "admin.history.host.v1";

/// Encode a slice of derived points as a `Dat::Vek` of five-tuples,
/// suitable for writing via [`Database::insert`].
pub fn encode_host_points(points: &[DerivedHostPoint]) -> Dat {
    let mut v = Vec::with_capacity(points.len());
    for p in points {
        v.push(tup5dat![
            Dat::U64(p.t_secs),
            dat!(p.cpu_pct),
            dat!(p.mem_pct),
            dat!(p.disk_bps),
            dat!(p.net_bps),
        ]);
    }
    Dat::Vek(Vek(v))
}

/// Decode a stored `Dat::Vek<Dat::Tup5>` back into a vector of derived
/// points. Malformed entries are skipped so a stale or partially valid
/// payload degrades to a shorter history rather than a hard load failure.
pub fn decode_host_points(dat: Dat) -> Outcome<Vec<DerivedHostPoint>> {
    let vek = try_extract_dat!(dat, Vek);
    let mut out = Vec::with_capacity(vek.len());
    for item in vek {
        // Use a local closure so a single bad row does not abort the
        // whole load. Operators never want the first sample after a
        // redeploy to be "dashboard is blank because one row had a
        // typo somewhere".
        let row_result: Outcome<DerivedHostPoint> = (|| {
            let arr = try_extract_tup5dat!(item);
            let t   = try_extract_dat!(arr[0].clone(), U64);
            let cpu = try_extract_dat!(arr[1].clone(), F64);
            let mem = try_extract_dat!(arr[2].clone(), F64);
            let dsk = try_extract_dat!(arr[3].clone(), F64);
            let net = try_extract_dat!(arr[4].clone(), F64);
            Ok(DerivedHostPoint {
                t_secs:   t,
                cpu_pct:  *cpu,
                mem_pct:  *mem,
                disk_bps: *dsk,
                net_bps:  *net,
            })
        })();
        match row_result {
            Ok(p) => out.push(p),
            Err(e) => {
                warn!("persist: skipping malformed host history row: {}", e);
            },
        }
    }
    Ok(out)
}

/// Write the supplied derived points to `db` under [`HOST_HISTORY_KEY`].
/// The write is attributed to `user`; callers typically pass the same
/// admin/system user they use for dashboard writes.
pub fn save_host_points<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
    ENC: Encrypter,
    KH:  Hasher,
    DB:  Database<UIDL, UID, ENC, KH>,
>(
    db:     &DB,
    user:   UID,
    points: &[DerivedHostPoint],
)
    -> Outcome<()>
{
    let key = Dat::Str(HOST_HISTORY_KEY.to_string());
    let val = encode_host_points(points);
    let _ = res!(db.insert(key, val, user, None));
    Ok(())
}

/// Read the derived points previously saved under [`HOST_HISTORY_KEY`].
/// Returns an empty vector when the key is missing, which happens on the
/// first run after the persistence feature ships.
pub fn load_host_points<
    const UIDL: usize,
    UID: NumIdDat<UIDL>,
    ENC: Encrypter,
    KH:  Hasher,
    DB:  Database<UIDL, UID, ENC, KH>,
>(
    db: &DB,
)
    -> Outcome<Vec<DerivedHostPoint>>
{
    let key = Dat::Str(HOST_HISTORY_KEY.to_string());
    match res!(db.get(&key, None)) {
        Some((dat, _meta)) => decode_host_points(dat),
        None => Ok(Vec::new()),
    }
}
