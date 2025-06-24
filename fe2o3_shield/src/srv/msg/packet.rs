use crate::{
    srv::{
        constant,
        msg::{
            core::MsgType,
            handshake::HandshakeType,
        },
    },
};

use oxedyne_fe2o3_core::{
    prelude::*,
    byte::{
        FromBytes,
        ToBytes,
    },
};
use oxedyne_fe2o3_iop_crypto::sign::Signer;
use oxedyne_fe2o3_jdat::{
    id::NumIdDat,
    version::SemVer,
};
use oxedyne_fe2o3_hash::{
    pow::{
        PowCreateParams,
        PowSearchResult,
        PowVars,
        Pristine,
        ProofOfWork,
    },
};
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_namex::id::LocalId;

use std::{
    convert::TryFrom,
    ops::Range,
};

pub type PacketCount = u32;
pub const PACKET_COUNT_BYTE_LEN: usize = 4;

/// A more economical form of `oxedyne_fe2o3_jdat::chunk::ChunkState`.
#[derive(Clone, Debug, Default)]
pub struct PacketChunkState {
    pub index:      PacketCount,
    pub num_chunks: PacketCount,
    pub chunk_size: u16,
    pub pad_last:   bool,
}

impl ToBytes for PacketChunkState {
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        buf.extend_from_slice(&self.index.to_be_bytes());
        buf.extend_from_slice(&self.num_chunks.to_be_bytes());
        buf.extend_from_slice(&self.chunk_size.to_be_bytes());
        buf.push(if self.pad_last { 1 } else { 0 });
        Ok(buf)
    }
}

impl FromBytes for PacketChunkState {
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        let mut result = Self::default();
        if buf.len() < Self::BYTE_LEN {
            return Err(err!(
                "Not enough bytes to decode, require at least {}, found only {}.",
                Self::BYTE_LEN, buf.len();
                Bytes, Input, Decode, Missing));
        }
        let mut n: usize = 0;
        result.index = PacketCount::from_be_bytes(res!(
            <[u8; PACKET_COUNT_BYTE_LEN]>::try_from(&buf[n..n + PACKET_COUNT_BYTE_LEN]),
            Decode, Bytes));
        n += PACKET_COUNT_BYTE_LEN;

        result.num_chunks = PacketCount::from_be_bytes(res!(
            <[u8; PACKET_COUNT_BYTE_LEN]>::try_from(
                &buf[n..n + PACKET_COUNT_BYTE_LEN]
        ), Decode, Bytes));
        n += PACKET_COUNT_BYTE_LEN;

        result.chunk_size = u16::from_be_bytes(res!(
            <[u8; 2]>::try_from(
                &buf[n..n + 2]
        ), Decode, Bytes));
        n += 2;

        result.pad_last = match u8::from_be_bytes(res!(
            <[u8; 1]>::try_from(
                &buf[n..n + 1]
        ), Decode, Bytes)) {
            0 => false,
            _ => true,
        };
        n += 1;

        Ok((result, n))
    }
}

impl PacketChunkState {
    pub const BYTE_LEN: usize = 2 * PACKET_COUNT_BYTE_LEN + 2 + 1;
}

#[derive(Clone, Debug)]
pub struct PacketMeta<
    const MIDL: usize,
    const UIDL: usize,
    MID: NumIdDat<MIDL>,
    UID: NumIdDat<UIDL>,
> {
    pub typ:    MsgType,
    pub ver:    SemVer,
    pub mid:    MID,
    pub uid:    UID,
    pub chnk:   PacketChunkState,
}

impl<
    const MIDL: usize,
    const UIDL: usize,
    MID: NumIdDat<MIDL>,
    UID: NumIdDat<UIDL>,
>
    Default for PacketMeta<MIDL, UIDL, MID, UID>
{
    fn default() -> Self {
        Self {
            typ:    0,
            ver:    SemVer::default(),
            mid:    MID::default(),
            uid:    UID::default(),
            chnk:   PacketChunkState::default(),
        }
    }
}

/// ```ignore
///
/// Byte map:
///
/// □□ □□□ □□□□□□□□ □□□□□□□□□□□□□□□□  □□□□ □□□□ □□  □□□□□□□□
/// |   |    mid           uid         |    |   |    tstamp
/// typ |                             index | chunk_
///  ver+                                   | size
///                                       num_
///                                       chunks
///                                   \___________/
///                                         |
///                                    chunk state
///  ```
impl<
    const MIDL: usize,
    const UIDL: usize,
    MID: NumIdDat<MIDL>,
    UID: NumIdDat<UIDL>,
>
    ToBytes for PacketMeta<MIDL, UIDL, MID, UID>
{
    fn to_bytes(&self, mut buf: Vec<u8>) -> Outcome<Vec<u8>> {
        buf.extend_from_slice(&self.typ.to_be_bytes());
        buf = res!(self.ver.to_bytes(buf));
        buf = res!(self.mid.to_bytes(buf));
        buf = res!(self.uid.to_bytes(buf));
        buf = res!(self.chnk.to_bytes(buf));
        //buf.extend_from_slice(&self.tstamp.to_be_bytes());
        Ok(buf)
    }
}

impl<
    const MIDL: usize,
    const UIDL: usize,
    MID: NumIdDat<MIDL>,
    UID: NumIdDat<UIDL>,
>
    FromBytes for PacketMeta<MIDL, UIDL, MID, UID>
{
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        let mut result = Self::default();
        if buf.len() < Self::BYTE_LEN {
            return Err(err!(
                "Not enough bytes to decode, require at least {}, found only {}.",
                Self::BYTE_LEN, buf.len();
                Bytes, Input, Decode, Missing));
        }

        let mut n = constant::MSG_TYPE_BYTE_LEN;
        result.typ = MsgType::from_be_bytes(res!(
            <[u8; constant::MSG_TYPE_BYTE_LEN]>::try_from(&buf[0..n]),
                Decode, Bytes)
        );

        let (ver, n2) = res!(SemVer::from_bytes(&buf[n..]));
        result.ver = ver;
        n += n2;

        let (mid, n_mid) = res!(MID::from_bytes(&buf[n..]));
        result.mid = mid;
        n += n_mid;

        let (uid, n_uid) = res!(UID::from_bytes(&buf[n..]));
        result.uid = uid;
        n += n_uid;

        let (chnk, n0) = res!(PacketChunkState::from_bytes(&buf[n..]));
        result.chnk = chnk;
        n += n0;

        Ok((result, n))
    }
}

impl<
    const MIDL: usize,
    const UIDL: usize,
    MID: NumIdDat<MIDL>,
    UID: NumIdDat<UIDL>,
>
    PacketMeta<MIDL, UIDL, MID, UID>
{
    pub const BYTE_LEN: usize =
        constant::MSG_TYPE_BYTE_LEN + // message type
        SemVer::BYTE_LEN +
        MIDL +
        UIDL +
        PacketChunkState::BYTE_LEN;// +
        //8; // time since Unix epoch in seconds
}

#[repr(u8)]
pub enum PacketValidatorId {
    Pow = 1,
    BareSignature = 2,
    SignatureWithKey = 3,
}

impl TryFrom<u8> for PacketValidatorId {
    type Error = Error<ErrTag>;
    fn try_from(n: u8) -> std::result::Result<Self, Self::Error> {
        match n {
            1 => Ok(Self::Pow),
            2 => Ok(Self::BareSignature),
            3 => Ok(Self::SignatureWithKey),
            _ => Err(err!(
                "Number {} not recognised as a PacketValidatorId.", n;
                Input, Invalid)),
        }
    }
}

/// Contains the optional ranges for the validation artefacts in a byte slice, including:
/// - Proof of work artefact p0..p1,
/// - Signature artefact s0..s1 and s2..s3, s4..s5 when the public key is included.
#[derive(Default)]
pub struct PacketValidationArtefactRelativeIndices {
    pub pow: Option<Range<usize>>,
    pub sig: Option<(Range<usize>, Option<(Range<usize>, Range<usize>)>)>,
}

///```ignore
///
/// Case of no public key with (bare) signature:
///
/// u16 len                      u16 len
///    |                            |          signature
///  --+                          --+         / artefact
/// □□□ □□□□□□□□□□□□□□□□□□□□□□□□ □□□ □□□□□□□□
/// |   |           |          | |   |      |
/// id  |      pow artefact    | id  |      |
/// |   p0                     p1    s0     s1
/// n=0
/// 
/// Case of public key with signature:
///
///                        u16 len   u16 len   u16 len
/// u16 len                artefact  pub key   signature
///    |                         |     |         |    
///  --+                         +-- --+       --+    
/// □□□ □□□□□□□□□□□□□□□□□□□□□□□□ □□□ □□ □□□□□□ □□ □□□□□□□□
/// |   |           |          | |   |  |  | |    |  |   |
/// id  |      pow artefact    | id  |  |  | |    |  |   |
///     p0                     p1    s0 s2 | s3   s4 |   s1,s5
///                                        |         |
///                                    signature  signature
///                                    public key          
///```
impl FromBytes for PacketValidationArtefactRelativeIndices {
    fn from_bytes(buf: &[u8]) -> Outcome<(Self, usize)> {
        let mut result = Self::default();
        let mut n: usize = 0;
        match PacketValidatorId::try_from(buf[n] as u8) {
            Ok(PacketValidatorId::Pow) => {
                n += 1;
                let (len, ns) = res!(Self::read_size(&buf, n));
                n += ns;
                result.pow = Some(n..n + len);
                n += len;
            },
            _ => result.pow = None,
        }
        match PacketValidatorId::try_from(buf[n] as u8) {
            Ok(pvid) => {
                match pvid {
                    PacketValidatorId::BareSignature | PacketValidatorId::SignatureWithKey => {
                        // Get the overall artefact range.
                        n += 1;
                        let (len, ns) = res!(Self::read_size(&buf, n));
                        n += ns;
                        let sigval_rng = n..n + len;
                        //result.sig = Some(n..n + len);
                        let withkey_ranges = if let PacketValidatorId::SignatureWithKey = pvid {
                            // Get the ranges for the key and the signature.
                            let (pk_len, ns) = res!(Self::read_size(&buf, n));
                            n += ns;
                            let pk_rng = n..n + pk_len;
                            n += pk_len;
                            let (sig_len, ns) = res!(Self::read_size(&buf, n));
                            n += ns;
                            let sig_rng = n..n + sig_len;
                            n += sig_len;
                            if sigval_rng.end != sig_rng.end {
                                return Err(err!(
                                    "The end point of the overall relative signature artefact range, \
                                    {:?}, should match the end point of the relative range of the actual \
                                    signature, {:?}, when the public key (range {:?}) is included.",
                                    sigval_rng, sig_rng, pk_rng;
                                    Bug, Index, Mismatch));
                            }
                            Some((pk_rng, sig_rng))
                        } else {
                            n += len;
                            None
                        };
                        result.sig = Some((sigval_rng, withkey_ranges));
                    },
                    _ => result.sig = None,
                }
            },
            _ => result.sig = None,
        }
        Ok((result, n))
    }
}

impl PacketValidationArtefactRelativeIndices {

    pub const BYTE_PREFIX_LEN: usize = 1 + 2;

    fn read_size(buf: &[u8], n: usize) -> Outcome<(usize, usize)> {
        Ok((
            u16::from_be_bytes(res!(<[u8; 2]>::try_from(&buf[n..n+2]),
                Decode, Bytes)) as usize, 
            2,
        ))
    }
}

/// Contains the algorithmic schemes for shield packet validation.
#[derive(Clone, Debug, Default)]
pub struct PacketValidator<
    // Proof of work validation.
    H: Hasher, // Proof of work hasher.
    // Digital signature validation.
    S: Signer,
> {
    //pub pow: Option<(ProofOfWork<H>, Option<PowParams<P0, P1, PRIS>>)>,
    pub pow: Option<ProofOfWork<H>>,
    pub sig: Option<S>,
}

impl<
    // Proof of work validator.
    H: Hasher + Send + 'static, // Proof of work hasher.
    // Digital signature validation.
    S: Signer,
>
    PacketValidator<H, S>
{
    pub fn to_bytes<
        const N: usize, // Pristine + Nonce size.
        const P0: usize, // Length of pristine prefix bytes (i.e. not included in artefact).
        const P1: usize, // Length of pristine bytes (i.e. included in artefact).
        PRIS: Pristine<P0, P1>, // Pristine supplied to hasher.
    >(
        &self,
        mut buf:    Vec<u8>,
        powparams:  &PowCreateParams<P0, P1, PRIS>,
        inc_sigpk:  bool,
    )
        -> Outcome<Vec<u8>>
    {
        if let Some(power) = &self.pow {
            match power.create::<N, P0, P1, PRIS>(powparams) {
                Ok(PowSearchResult{
                    found,
                    mut artefact,
                    elapsed,
                    err,
                    ..
                }) => {
                    if let Some(e) = err {
                        return Err(e);
                    }
                    if !found {
                        return Err(err!(
                            "Proof of work validator: search timed out after {:?}.",
                            elapsed;
                            Timeout, Missing));
                    }

                    /////// Debugging only.
                    trace!(async_log::stream(), "POW tx:");
                    res!(self.trace(
                        Some(&powparams.pvars),
                        &artefact,
                    ));
                    ///////

                    buf.extend_from_slice(&(PacketValidatorId::Pow as u8).to_be_bytes());
                    buf.extend_from_slice(&(artefact.len() as u16).to_be_bytes());
                    buf.append(&mut artefact);
                },
                Err(e) => return Err(e),
            }
        }
        if let Some(signer) = &self.sig {
            match inc_sigpk {
                true => {
                    let mut sig = res!(signer.sign(&buf));
                    let pk = match res!(signer.get_public_key()) {
                        Some(k) => k,
                        None => return Err(err!(
                            "Signature validator: public key not available.";
                            Bug, Configuration, Missing)),
                    };
                    let mut artefact = Vec::new();
                    artefact.extend_from_slice(&(pk.len() as u16).to_be_bytes());
                    artefact.extend_from_slice(&pk[..]); // Key first..
                    artefact.extend_from_slice(&(sig.len() as u16).to_be_bytes());
                    artefact.append(&mut sig); // then signature

                    buf.extend_from_slice(&(PacketValidatorId::SignatureWithKey as u8).to_be_bytes());
                    buf.extend_from_slice(&(artefact.len() as u16).to_be_bytes());
                    buf.append(&mut artefact);
                },
                false => {
                    let mut artefact = res!(signer.sign(&buf));
                    buf.extend_from_slice(&(PacketValidatorId::BareSignature as u8).to_be_bytes());
                    buf.extend_from_slice(&(artefact.len() as u16).to_be_bytes());
                    buf.append(&mut artefact);
                },
            }
        }
        Ok(buf)
    }

    /// The signature is based on the entire packet up to but not including the signature artefact.
    pub fn validate<
        const P0: usize,
        const P1: usize,
        PRIS: Pristine<P0, P1>,
    >(
        self,
        buf:            &[u8], // Entire packet.
        n0:             usize, // Start index of validation artefacts within packet.
        afact_rel_ind:  PacketValidationArtefactRelativeIndices,
        //powvals:        Option<([u8; P0], pow::ZeroBits)>, // (pristine prefix, reqd zbits)
        powvars:        Option<PowVars<P0, P1, PRIS>>,
        msg_typ:        MsgType,
    )
        -> Outcome<PacketValidationResult>
    {
        let pow = match self.pow {
            Some(power) => {
                let powvars = res!(powvars.ok_or(err!(
                    "Proof of work validation missing requirements.";
                    Bug, Configuration, Missing)));
                match afact_rel_ind.pow {
                    Some(range) => {
                        Some(res!(power.validate(
                            &powvars,
                            &buf[n0 + range.start..n0 + range.end],
                        )))
                    }
                    None => return Err(err!(
                        "Proof of work validation missing artefact.";
                        Bug, Configuration, Missing)),
                }
            },
            None => None,
        };
        let sig = match self.sig {
            Some(mut signer) => match afact_rel_ind.sig {
                Some((range, None)) => { // range covers only the signature.
                    let len = range.len() + PacketValidationArtefactRelativeIndices::BYTE_PREFIX_LEN;
                    Some((
                        res!(signer.verify(
                            &buf[..buf.len() - len], // Sign against everything up to the signature.
                            &buf[n0 + range.start..n0 + range.end],
                        )),
                        None,
                    ))
                },
                Some((range, Some((pk_rng, sig_rng)))) => { // range covers the public key and the signature.
                    // Provision of the public key is only valid if the message is a
                    // HandshakeType::Req1.
                    if HandshakeType::from(msg_typ) != HandshakeType::Req1 {
                        return Ok(PacketValidationResult {
                            pow,
                            sig: None,
                        });
                    }
                    let len = range.len() + PacketValidationArtefactRelativeIndices::BYTE_PREFIX_LEN;
                    let pk = &buf[n0 + pk_rng.start..n0 + pk_rng.end];
                    signer = res!(signer.set_public_key(Some(&pk[..])));
                    Some((
                        res!(signer.verify(
                            &buf[..buf.len() - len], // Sign against everything up to the signature.
                            &buf[n0 + sig_rng.start..n0 + sig_rng.end],
                        )),
                        Some((signer.local_id(), pk)),
                    ))
                },
                None => return Err(err!(
                    "Validator requires a signature but no artefact has been included.";
                    Bug, Configuration, Missing)),
            },
            None => None,
        };
        
        Ok(PacketValidationResult {
            pow,
            sig,
        })
    }

    pub fn trace<
        const P0: usize,
        const P1: usize,
        PRIS: Pristine<P0, P1>,
    >(
        &self,
        powvars:    Option<&PowVars<P0, P1, PRIS>>,
        artefact:   &[u8],
    )
        -> Outcome<()>
    {
        match &self.pow {
            Some(power) => {
                match powvars {
                    Some(powvars) => {
                        let gnomon = power.hasher.hash_length();
                        let hlen = res!(gnomon.required("hash length"));
                        let alen = artefact.len();
                        let nlen = alen - hlen - (P1-P0);
                        let h_start = alen - hlen;
                        let n_start = h_start - nlen;
                        let pristine = res!(powvars.pristine.to_bytes());
                        trace!(async_log::stream(), "\nPristine    [{:>4}]: {:02x?}\
                            \n  Prefix    [{:>4}]: {:02x?}\
                            \nArtefact    [{:>4}]: {:02x?}\
                            \n  Nonce     [{:>4}]: {:02x?}\
                            \n  Hash      [{:>4}]: {:02x?}",
                            P1, pristine,
                            P0, &pristine[..P0],
                            alen, artefact,
                            nlen, &artefact[n_start..h_start],
                            hlen, &artefact[h_start..],
                        );
                    },
                    None => return Err(err!(
                        "Proof of work validation missing requirements.";
                        Bug, Configuration, Missing)),
                }
            },
            None => trace!(async_log::stream(), "No proof of work hasher provided."),
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct PacketValidationResult<'a> {
    pow: Option<bool>,
    sig: Option<(bool, Option<(LocalId, &'a[u8])>)>, // Public signing key may have been included.
}

impl<'a> PacketValidationResult<'a> {
    /// ```ignore
    ///
    ///                           pow
    ///                +-------+-------+-------+
    ///                |   T   |   F   |  None |
    ///         +------+-------+-------+-------+
    ///         |   T  |   T   |   F   |   T   |
    ///         +------+-------+-------+-------+
    ///    sig  |   F  |   F   |   F   |   F   |
    ///         +------+-------+-------+-------+
    ///         | None |   T   |   F   |  None |
    ///         +------+-------+-------+-------+
    ///          
    /// ```
    pub fn is_valid(self) -> Option<(bool, Option<(LocalId, &'a[u8])>)> {
        match self.pow {
            Some(pb) => match self.sig {
                Some((sb, pk_opt)) => Some((pb && sb, pk_opt)),
                None => Some((pb, None)),
            },
            None => match self.sig {
                Some((sb, pk_opt)) => Some((sb, pk_opt)),
                None => None,
            },
        }
    }

    pub fn pow_invalid(&self) -> bool {
        match self.pow {
            Some(b) => !b,
            None => false,
        }
    }

    pub fn sig_invalid(&self) -> bool {
        match self.sig {
            Some((b, _)) => !b,
            None => false,
        }
    }

    pub fn pow_state(&self) -> &'static str {
        match self.pow {
            Some(true) => "PASS",
            Some(false) => "FAIL",
            None => "NONE",
        }
    }

    pub fn sig_state(&self) -> &'static str {
        match self.sig {
            Some((true, _)) => "PASS",
            Some((false, _)) => "FAIL",
            None => "NONE",
        }
    }

}
