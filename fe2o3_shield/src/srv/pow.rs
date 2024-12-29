pub use oxedize_fe2o3_core::{
    prelude::*,
    rand::Rand,
};
use oxedize_fe2o3_hash::pow::{
    Pristine,
    ZeroBits
};

use std::{
    net::{
        IpAddr,
        Ipv4Addr,
    },
    time::{
        Duration,
        SystemTime,
    },
};

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DifficultyProfile {
    Linear  = 0,
}

impl TryFrom<u8> for DifficultyProfile {
    type Error = Error<ErrTag>;

    fn try_from(n: u8) -> std::result::Result<Self, Self::Error> {
        match n {
            0 => Ok(Self::Linear),
            _ => Err(err!(
                "'{}' not recognised as a valid server_rps_zbits_profile configuration value, \
                use a value in the range 0..0.", n;
                Invalid, Input)),
        }
    }
}

/// Vary the required zero bits in proof of work hashes as a function of the requests-per-second
/// using the given profile and min/max limits.
#[derive(Clone, Debug)]
pub struct DifficultyParams {
    pub profile:    DifficultyProfile,
    pub max:        u16,
    pub min:        u16,
    pub rps_max:    u64,//u16
}

impl DifficultyParams {
    #[inline(always)]
    pub fn required_global_zbits(&self, rps: u16) -> Outcome<ZeroBits> {
        match self.profile {
            DifficultyProfile::Linear => Ok(
                ((((self.max - self.min) * rps)
                / self.max) + self.min) as ZeroBits
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PowPristine<
    const C: usize,
    const P0: usize,
    const P1: usize,
> {
    pub code:       [u8; C],
    pub src_addr:   IpAddr,
    pub timestamp:  Duration,
    pub time_horiz: u64,
}

impl<
    const C: usize,
    const P0: usize,
    const P1: usize,
>
    Default for PowPristine<C, P0, P1>
{
    fn default() -> Self {
        Self {
            code:       [0; C],
            src_addr:   IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            timestamp:  Duration::ZERO,
            time_horiz: 600,
        }
    }
}

impl<
    const C: usize,
    const P0: usize,
    const P1: usize,
>
    Pristine<P0, P1> for PowPristine<C, P0, P1>
{
    fn to_bytes(&self) -> Outcome<[u8; P1]> {
        let mut byts = [0u8; P1];
        let mut i = res!(self.prefix(&mut byts));
        let t = self.timestamp.as_secs().to_be_bytes();
        for b in t {
            byts[i] = b;
            i += 1;
        }
        Ok(byts)
    }

    /// Pad IPv4 addresses out to the length of an IPv6 address by repetition.  Append the code.
    fn prefix(&self, byts: &mut [u8]) -> Outcome<usize> {
        if byts.len() < Self::PREFIX_BYTE_LEN {
            return Err(err!(
                "Cannot fit {} address bytes into given slice of length {}.",
                Self::PREFIX_BYTE_LEN, byts.len(); 
                Bug, Input, TooSmall));
        }
        let mut i: usize = 0;
        let addr = self.src_addr;
        match addr {
            IpAddr::V4(addr) => {
                for _ in 0..4 {
                    for b in addr.octets() {
                        byts[i] = b;
                        i += 1;
                    }
                }
            },
            IpAddr::V6(addr) => {
                for b in addr.octets() {
                    byts[i] = b;
                    i += 1;
                }
            },
        }
        for b in self.code {
            byts[i] = b;
            i += 1;
        }
        Ok(Self::PREFIX_BYTE_LEN)
    }

    /// Check that the timestamp in the artefact is less than the given time horizon in seconds.
    fn timestamp_valid(&self, artefact: &[u8]) -> Outcome<bool> {
        if artefact.len() < Self::TIMESTAMP_LEN {
            return Err(err!(
                "Artefact slice length {} must be at least a timestamp length, {}.",
                artefact.len(), Self::TIMESTAMP_LEN;
                Input, TooSmall));
        }
        let t0 = u64::from_be_bytes(
            res!(<[u8; 8]>::try_from(&artefact[..8]), Decode, Bytes)
        );
        let t1 = res!(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)).as_secs();
        if t0 > t1 {
            Ok(false)
        } else {
            Ok((t1 - t0) < self.time_horiz)
        }
    }
}

impl<
    const C: usize,
    const P0: usize,
    const P1: usize,
>
    PowPristine<C, P0, P1>
{
    pub const TIMESTAMP_LEN:    usize = 8;
    pub const ADDR_LEN:         usize = 16;

    /// Create a new `PowPristine` for the purpose of validating an incoming packet.
    pub fn new_rx(
        code:       [u8; C],
        src_addr:   IpAddr,
        time_horiz: u64,
    )
        -> Outcome<Self>
    {
        if P0 >= P1 {
            return Err(err!(
                "The pristine prefix length {} must be less than the pristine length, {}.",
                P0, P1;
                Input, Mismatch));
        }
        // TODO Other checks?
        Ok(Self {
            code,
            src_addr,
            timestamp:  Duration::ZERO,
            time_horiz,
        })
    }

    pub fn trace(&self) -> Outcome<()> {
        let byts = res!(self.to_bytes());
        trace!("\nPrefix   [{:>4}]: {:02x?}\
            \n  Addr   [{:>4}]: {:02x?}\
            \n  Code   [{:>4}]: {:02x?}\
            \nArtefact [{:>4}]: {:02x?}\
            \n  Time   [{:>4}]: {:02x?}",
            P0, &byts[..P0],
            P0-C, &byts[..P0-C],
            C, self.code,
            P1-P0, &byts[P0..],
            P1-P0, &byts[P0..],
        );
        Ok(())
    }
}
