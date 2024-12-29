use oxedize_fe2o3_core::{
    prelude::*,
};
use oxedize_fe2o3_crypto::{
    keys::{
        PublicKey,
        SecretKey,
    },
    scheme::SchemeTimestamp,
};
use oxedize_fe2o3_hash::pow::ZeroBits;
use oxedize_fe2o3_jdat::id::{
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
};

/// Just a namespace for some functions to interchange between a string and a [`std::net::SocketAddress`].
pub struct Address;

impl Address {

    //pub fn udp_socket<S: Into<String> + std::fmt::Display>(&mut self, addr: S) -> Outcome<UdpSocket> {
    //    res!(self.update());
    //    let socket = res!(UdpSocket::bind(
    //        Self::server_address(addr, self.cfg().server_port_udp)));
    //    Ok(socket)
    //}

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

/// [`AddressData`] contains parameters that are provided to an address, which are expected during
/// packet validation, and that an address provides to us, which it expects during packet
/// validation.  Used by [`oxedize_fe2o3_net::guard::addr::AddressGuard`] to store information relating to
/// an IP address.
#[derive(Clone, Debug, Default)]
pub struct AddressData {
    pub my_zbits:   ZeroBits, // The difficulty I require of your POW.
    pub your_zbits: ZeroBits, // The difficulty you require of my POW.
}

impl AsMut<AddressData> for AddressData {
    fn as_mut(&mut self) -> &mut AddressData {
        self
    }
}

/// Used by [`oxedize_fe2o3_net::guard::user::UserGuard`] to store information relating to a user.
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
