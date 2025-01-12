use oxedize_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
    },
};

use oxedize_fe2o3_crypto::{
    enc::EncryptionScheme,
    sign::SignatureScheme,
};
use oxedize_fe2o3_iop_crypto::{
    enc::{
        Encrypter,
        EncrypterDefAlt,
    },
    sign::{
        Signer,
        SignerDefAlt,
    },
};
use oxedize_fe2o3_jdat::{
    chunk::{
        //Chunker,
        ChunkConfig,
    },
    //daticle::Daticle,
};
use oxedize_fe2o3_hash::{
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
use oxedize_fe2o3_iop_hash::{
    api::Hasher,
    csum::Checksummer,
    //kdf::KeyDeriver,
};

use std::fmt;


pub trait WireSchemeTypes: Clone + fmt::Debug {
	type ENC:   Encrypter;
	type CS:    Checksummer;
    type POWH:  Hasher;
    type SGN:   Signer;
    type HS:    Encrypter; // Handshake encryption.
}

#[derive(Clone, Debug)]
pub struct DefaultWireSchemes;

impl WireSchemeTypes for DefaultWireSchemes {
    type ENC = EncryptionScheme;
    type CS = ChecksumScheme;
    type POWH = HashScheme;
    type SGN = SignatureScheme;
    type HS = EncryptionScheme;
}

#[derive(Clone, Debug)]
pub struct WireSchemesInput<W: WireSchemeTypes> {
    pub enc:    Alt<W::ENC>,
    pub csum:   Alt<W::CS>,
    pub powh:   Alt<W::POWH>,
    pub sign:   Alt<W::SGN>,
    pub hsenc:  Alt<W::HS>,
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
    pub fn ref_encrypter(&self)             -> &Alt<W::ENC>         { &self.enc }
    pub fn ref_checksummer(&self)           -> &Alt<W::CS>          { &self.csum }
    pub fn ref_pow_hasher(&self)            -> &Alt<W::POWH>        { &self.powh }
    pub fn ref_signer(&self)                -> &Alt<W::SGN>         { &self.sign }
    pub fn ref_handshake_encrypter(&self)   -> &Alt<W::HS>          { &self.hsenc }
    pub fn ref_chunk_config(&self)          -> &Option<ChunkConfig> { &self.chnk }

    pub fn own_encrypter(&mut self)     -> Alt<W::ENC>  { std::mem::replace(&mut self.enc, Alt::Unspecified) }
    pub fn own_checksummer(&mut self)   -> Alt<W::CS>   { std::mem::replace(&mut self.csum, Alt::Unspecified) }
    pub fn own_pow_hasher(&mut self)    -> Alt<W::POWH> { std::mem::replace(&mut self.powh, Alt::Unspecified) }
    pub fn own_signer(&mut self)        -> Alt<W::SGN>  { std::mem::replace(&mut self.sign, Alt::Unspecified) }
    pub fn own_handshake_encrypter(&mut self) -> Alt<W::HS> { std::mem::replace(&mut self.hsenc, Alt::Unspecified) }
    pub fn own_chunk_config(&mut self)  -> Option<ChunkConfig>  { std::mem::replace(&mut self.chnk, None) }

    pub fn clone_encrypter(&self)           -> Alt<W::ENC>          { self.enc.clone() }
    pub fn clone_checksummer(&self)         -> Alt<W::CS>           { self.csum.clone() }
    pub fn clone_pow_hasher(&self)          -> Alt<W::POWH>         { self.powh.clone() }
    pub fn clone_signer(&self)              -> Alt<W::SGN>          { self.sign.clone() }
    pub fn clone_handshake_encrypter(&self) -> Alt<W::HS>           { self.hsenc.clone() }
    pub fn clone_chunk_config(&self)        -> Option<ChunkConfig>  { self.chnk.clone() }

    pub fn set_signer(mut self, signer: Option<W::SGN>) -> Self {
        self.sign = Alt::Specific(signer);
        self
    }
}

#[derive(Clone, Debug)]
pub struct WireSchemes<W: WireSchemeTypes> {
    pub enc:    EncrypterDefAlt<EncryptionScheme, W::ENC>,
    pub csum:   ChecksummerDefAlt<ChecksumScheme, W::CS>,
    pub powh:   HasherDefAlt<HashScheme, W::POWH>,
    pub sign:   SignerDefAlt<SignatureScheme, W::SGN>,
    pub hsenc:  EncrypterDefAlt<EncryptionScheme, W::HS>,
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
    pub fn ref_encrypter(&self)     -> &EncrypterDefAlt<EncryptionScheme, W::ENC>   { &self.enc }
    pub fn ref_checksummer(&self)   -> &ChecksummerDefAlt<ChecksumScheme, W::CS>    { &self.csum }
    pub fn ref_pow_hasher(&self)    -> &HasherDefAlt<HashScheme, W::POWH>           { &self.powh }
    pub fn ref_signer(&self)        -> &SignerDefAlt<SignatureScheme, W::SGN>       { &self.sign }
    pub fn ref_handshake_encrypter(&self) -> &EncrypterDefAlt<EncryptionScheme, W::HS> { &self.hsenc }
    pub fn ref_chunk_config(&self) -> &ChunkConfig { &self.chnk }

    pub fn clone_encrypter(&self)   -> EncrypterDefAlt<EncryptionScheme, W::ENC>    { self.enc.clone() }
    pub fn clone_checksummer(&self) -> ChecksummerDefAlt<ChecksumScheme, W::CS>     { self.csum.clone() }
    pub fn clone_pow_hasher(&self)  -> HasherDefAlt<HashScheme, W::POWH>            { self.powh.clone() }
    pub fn clone_signer(&self)      -> SignerDefAlt<SignatureScheme, W::SGN>        { self.sign.clone() }
    pub fn clone_handshake_encrypter(&self) -> EncrypterDefAlt<EncryptionScheme, W::HS> { self.hsenc.clone() }
    pub fn clone_chunk_config(&self) -> ChunkConfig { self.chnk.clone() }

    //pub fn set_chunk_config(mut self, chnk: Option<ChunkConfig>) -> Self {
    //    self.chnk = chnk;
    //    self
    //}
}
