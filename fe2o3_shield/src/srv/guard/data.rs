use crate::srv::msg::handshake::HandshakeType;

use oxedyne_fe2o3_core::{
    prelude::*,
};
use oxedyne_fe2o3_crypto::{
    keys::{
        PublicKey,
        SecretKey,
    },
    scheme::SchemeTimestamp,
};
use oxedyne_fe2o3_hash::pow::ZeroBits;
use oxedyne_fe2o3_jdat::id::{
    IdDat,
    NumIdDat,
};

use std::{
    collections::{
        BTreeMap,
        BTreeSet,
    },
    net::{
        IpAddr,
        SocketAddr,
    },
    str::FromStr,
    time::SystemTime,
};

/// Just a namespace for some functions to interchange between a string and a
/// [`std::net::SocketAddress`].
pub struct Address;

impl Address {

    pub fn server_address<S: Into<String> + std::fmt::Display>(addr: S, port: u16) -> String {
        fmt!("{}:{}", addr, port)
    }

    pub fn socket_address_udp<
        S: Into<String> + std::fmt::Display,
    >(
        addr: S,
        port: u16,
    )
        -> Outcome<SocketAddr>
    {
        let sock_addr = SocketAddr::new(res!(IpAddr::from_str(&addr.into())), port);
        Ok(sock_addr)
    }
}

/// SHIELD-specific extension payload attached to each generic
/// [`oxedyne_fe2o3_net::guard::addr::AddressLog`].
///
/// Carries proof-of-work difficulty targets negotiated with the peer, plus the pending
/// handshake request that allows the shield wrapper to enforce the HReq1 then HReq2 then
/// HReq3 sequence on top of the generic rate-limit state machine.
#[derive(Clone, Debug, Default)]
pub struct AddressData {
    /// The difficulty I require of your POW.
    pub my_zbits:   ZeroBits,
    /// The difficulty you require of my POW.
    pub your_zbits: ZeroBits,
    /// Pending handshake step for this address, if any. The timestamp marks when the step
    /// was recorded so expired handshakes can be discarded.
    pub pending:    Option<(HandshakeType, SystemTime)>,
}

impl AsMut<AddressData> for AddressData {
    fn as_mut(&mut self) -> &mut AddressData {
        self
    }
}

/// Used by [`oxedyne_fe2o3_shield::srv::guard::user::UserGuard`] to store information relating
/// to a user.
#[derive(Clone, Debug, Default)]
pub struct UserData<
    const SIDL: usize,
    const C: usize,
    SID: NumIdDat<SIDL>,
> {
    pub sigtpk_opt:         Option<PublicKey>, // My record of your current public signing key.
    pub sigtpk_opt_old:     Option<PublicKey>, // My record of your old public signing key.
    pub waiting_for_sigpk:  bool,
    pub sessions:           BTreeMap<IdDat<SIDL, SID>, SecretKey>,
    pub code:               Option<[u8; C]>,
    pub pack_sigpk_set:     BTreeSet<PublicKey>,
    pub pack_sigpk_map:     BTreeMap<SchemeTimestamp, Vec<u8>>,
    pub sign_pack_this:     Option<Vec<u8>>,
}
