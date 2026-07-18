use oxedyne_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
    },
};

use oxedyne_fe2o3_crypto::{
    enc::EncryptionScheme,
    sign::SignatureScheme,
};
use oxedyne_fe2o3_iop_crypto::{
    enc::{
        Encrypter,
        EncrypterDefAlt,
    },
    sign::{
        Signer,
        SignerDefAlt,
    },
};
use oxedyne_fe2o3_jdat::{
    chunk::{
        //Chunker,
        ChunkConfig,
    },
    //daticle::Daticle,
};
use oxedyne_fe2o3_hash::{
    csum::{
        ChecksummerDefAlt,
        ChecksumScheme,
    },
    hash::{
        HasherDefAlt,
        HashScheme,
    },
    //kdf::{
    //    KeyDerivationScheme,
    //    KeyDeriverDefAlt,
    //},
};
use oxedyne_fe2o3_iop_hash::{
    api::Hasher,
    csum::Checksummer,
    //kdf::KeyDeriver,
};

use std::fmt;


/// Bundles the cryptographic scheme types used on the wire: payload
/// encryption, checksumming, proof-of-work hashing, signing and handshake
/// encryption.
pub trait WireSchemeTypes: Clone + fmt::Debug {
	/// Payload encryption scheme.
	type ENC:   Encrypter;
	/// Checksum scheme.
	type CS:    Checksummer;
    /// Proof-of-work hashing scheme.
    type POWH:  Hasher;
    /// Signing scheme.
    type SGN:   Signer;
    /// Handshake encryption scheme.
    type HS:    Encrypter; // Handshake encryption.
}

/// Default [`WireSchemeTypes`] binding using the standard `fe2o3_crypto` and
/// `fe2o3_hash` schemes.
#[derive(Clone, Debug)]
pub struct DefaultWireSchemes;

impl WireSchemeTypes for DefaultWireSchemes {
    type ENC = EncryptionScheme;
    type CS = ChecksumScheme;
    type POWH = HashScheme;
    type SGN = SignatureScheme;
    type HS = EncryptionScheme;
}

/// Optional wire-scheme selections supplied by the caller, where each field
/// may be left unspecified to fall back to a default.
#[derive(Clone, Debug)]
pub struct WireSchemesInput<W: WireSchemeTypes> {
    /// Payload encryption scheme, if specified.
    pub enc:    Alt<W::ENC>,
    /// Checksum scheme, if specified.
    pub csum:   Alt<W::CS>,
    /// Proof-of-work hasher, if specified.
    pub powh:   Alt<W::POWH>,
    /// Signer, if specified.
    pub sign:   Alt<W::SGN>,
    /// Handshake encryption scheme, if specified.
    pub hsenc:  Alt<W::HS>,
    /// Chunking configuration, if specified.
    pub chnk:   Option<ChunkConfig>,
}

impl<W: WireSchemeTypes>
    Default for WireSchemesInput<W>
{
    fn default() -> Self {
        Self {
            enc:    Alt::Unspecified,
            csum:   Alt::Unspecified,
            powh:   Alt::Unspecified,
            sign:   Alt::Unspecified,
            hsenc:  Alt::Unspecified,
            chnk:   None,
        }
    }
}

impl<W: WireSchemeTypes>
    WireSchemesInput<W>
{
    /// Borrows the encryption scheme selection.
    pub fn ref_encrypter(&self)             -> &Alt<W::ENC>         { &self.enc }
    /// Borrows the checksum scheme selection.
    pub fn ref_checksummer(&self)           -> &Alt<W::CS>          { &self.csum }
    /// Borrows the proof-of-work hasher selection.
    pub fn ref_pow_hasher(&self)            -> &Alt<W::POWH>        { &self.powh }
    /// Borrows the signer selection.
    pub fn ref_signer(&self)                -> &Alt<W::SGN>         { &self.sign }
    /// Borrows the handshake encryption scheme selection.
    pub fn ref_handshake_encrypter(&self)   -> &Alt<W::HS>          { &self.hsenc }
    /// Borrows the chunking configuration.
    pub fn ref_chunk_config(&self)          -> &Option<ChunkConfig> { &self.chnk }

    /// Takes the encryption scheme selection, leaving it unspecified.
    pub fn own_encrypter(&mut self)     -> Alt<W::ENC>  { std::mem::replace(&mut self.enc, Alt::Unspecified) }
    /// Takes the checksum scheme selection, leaving it unspecified.
    pub fn own_checksummer(&mut self)   -> Alt<W::CS>   { std::mem::replace(&mut self.csum, Alt::Unspecified) }
    /// Takes the proof-of-work hasher selection, leaving it unspecified.
    pub fn own_pow_hasher(&mut self)    -> Alt<W::POWH> { std::mem::replace(&mut self.powh, Alt::Unspecified) }
    /// Takes the signer selection, leaving it unspecified.
    pub fn own_signer(&mut self)        -> Alt<W::SGN>  { std::mem::replace(&mut self.sign, Alt::Unspecified) }
    /// Takes the handshake encryption selection, leaving it unspecified.
    pub fn own_handshake_encrypter(&mut self) -> Alt<W::HS> { std::mem::replace(&mut self.hsenc, Alt::Unspecified) }
    /// Takes the chunking configuration, leaving it empty.
    pub fn own_chunk_config(&mut self)  -> Option<ChunkConfig>  { std::mem::replace(&mut self.chnk, None) }

    /// Clones the encryption scheme selection.
    pub fn clone_encrypter(&self)           -> Alt<W::ENC>          { self.enc.clone() }
    /// Clones the checksum scheme selection.
    pub fn clone_checksummer(&self)         -> Alt<W::CS>           { self.csum.clone() }
    /// Clones the proof-of-work hasher selection.
    pub fn clone_pow_hasher(&self)          -> Alt<W::POWH>         { self.powh.clone() }
    /// Clones the signer selection.
    pub fn clone_signer(&self)              -> Alt<W::SGN>          { self.sign.clone() }
    /// Clones the handshake encryption scheme selection.
    pub fn clone_handshake_encrypter(&self) -> Alt<W::HS>           { self.hsenc.clone() }
    /// Clones the chunking configuration.
    pub fn clone_chunk_config(&self)        -> Option<ChunkConfig>  { self.chnk.clone() }

    /// Returns a copy with the signer selection set to the given scheme.
    pub fn set_signer(mut self, signer: Option<W::SGN>) -> Self {
        self.sign = Alt::Specific(signer);
        self
    }
}

/// Fully resolved wire schemes, where each selection has been fixed to a
/// concrete scheme or its default.
#[derive(Clone, Debug)]
pub struct WireSchemes<W: WireSchemeTypes> {
    /// Payload encryption scheme.
    pub enc:    EncrypterDefAlt<EncryptionScheme, W::ENC>,
    /// Checksum scheme.
    pub csum:   ChecksummerDefAlt<ChecksumScheme, W::CS>,
    /// Proof-of-work hasher.
    pub powh:   HasherDefAlt<HashScheme, W::POWH>,
    /// Signer.
    pub sign:   SignerDefAlt<SignatureScheme, W::SGN>,
    /// Handshake encryption scheme.
    pub hsenc:  EncrypterDefAlt<EncryptionScheme, W::HS>,
    /// Chunking configuration.
    pub chnk:   ChunkConfig,
}

impl<W: WireSchemeTypes>
    Default for WireSchemes<W>
{
    fn default() -> Self {
        Self {
            enc:    EncrypterDefAlt(DefAlt::None),
            csum:   ChecksummerDefAlt(DefAlt::None),
            powh:   HasherDefAlt(DefAlt::None),
            sign:   SignerDefAlt(DefAlt::None),
            hsenc:  EncrypterDefAlt(DefAlt::None),
            chnk:   ChunkConfig::default(),
        }
    }
}

impl<W: WireSchemeTypes>
    From<WireSchemesInput<W>> for WireSchemes<W>
{
    fn from(input: WireSchemesInput<W>) -> Self {
        Self {
            enc:    EncrypterDefAlt::from(input.enc),
            csum:   ChecksummerDefAlt::from(input.csum),
            powh:   HasherDefAlt::from(input.powh),
            sign:   SignerDefAlt::from(input.sign),
            hsenc:  EncrypterDefAlt::from(input.hsenc),
            chnk:   match input.chnk {
                Some(cfg) => cfg,
                None => ChunkConfig::default(),
            },
        }
    }
}

impl<W: WireSchemeTypes>
    WireSchemes<W>
{
    /// Borrows the resolved encryption scheme.
    pub fn ref_encrypter(&self)     -> &EncrypterDefAlt<EncryptionScheme, W::ENC>   { &self.enc }
    /// Borrows the resolved checksum scheme.
    pub fn ref_checksummer(&self)   -> &ChecksummerDefAlt<ChecksumScheme, W::CS>    { &self.csum }
    /// Borrows the resolved proof-of-work hasher.
    pub fn ref_pow_hasher(&self)    -> &HasherDefAlt<HashScheme, W::POWH>           { &self.powh }
    /// Borrows the resolved signer.
    pub fn ref_signer(&self)        -> &SignerDefAlt<SignatureScheme, W::SGN>       { &self.sign }
    /// Borrows the resolved handshake encryption scheme.
    pub fn ref_handshake_encrypter(&self) -> &EncrypterDefAlt<EncryptionScheme, W::HS> { &self.hsenc }
    /// Borrows the chunking configuration.
    pub fn ref_chunk_config(&self) -> &ChunkConfig { &self.chnk }

    /// Clones the resolved encryption scheme.
    pub fn clone_encrypter(&self)   -> EncrypterDefAlt<EncryptionScheme, W::ENC>    { self.enc.clone() }
    /// Clones the resolved checksum scheme.
    pub fn clone_checksummer(&self) -> ChecksummerDefAlt<ChecksumScheme, W::CS>     { self.csum.clone() }
    /// Clones the resolved proof-of-work hasher.
    pub fn clone_pow_hasher(&self)  -> HasherDefAlt<HashScheme, W::POWH>            { self.powh.clone() }
    /// Clones the resolved signer.
    pub fn clone_signer(&self)      -> SignerDefAlt<SignatureScheme, W::SGN>        { self.sign.clone() }
    /// Clones the resolved handshake encryption scheme.
    pub fn clone_handshake_encrypter(&self) -> EncrypterDefAlt<EncryptionScheme, W::HS> { self.hsenc.clone() }
    /// Clones the chunking configuration.
    pub fn clone_chunk_config(&self) -> ChunkConfig { self.chnk.clone() }

    //pub fn set_chunk_config(mut self, chnk: Option<ChunkConfig>) -> Self {
    //    self.chnk = chnk;
    //    self
    //}
}
