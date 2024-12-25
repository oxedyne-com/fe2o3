use crate::msg::syntax;

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
use oxedize_fe2o3_syntax::core::SyntaxRef;

use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct WireSchemesInput<
	WENC:   Encrypter, // wire encrypter
	WCS:    Checksummer, // wire checksummer
    POWH:   Hasher,
    SGN:    Signer, // digital signature
    HS:     Encrypter, // handshake encryption
>{
    pub enc:    Alt<WENC>,
    pub csum:   Alt<WCS>,
    pub powh:   Alt<POWH>,
    pub sign:   Alt<SGN>,
    pub hsenc:  Alt<HS>,
    pub chnk:   Option<ChunkConfig>,
    pub syntax: SyntaxRef,
}

impl<
	WENC:   Encrypter,
	WCS:    Checksummer,
    POWH:   Hasher,
    SGN:    Signer,
    HS:     Encrypter,
>
    Default for WireSchemesInput<WENC, WCS, POWH, SGN, HS>
{
    fn default() -> Self {
        Self {
            enc:    Alt::Unspecified,
            csum:   Alt::Unspecified,
            powh:   Alt::Unspecified,
            sign:   Alt::Unspecified,
            hsenc:  Alt::Unspecified,
            chnk:   None,
            syntax: SyntaxRef(Arc::new(syntax::empty())),
        }
    }
}

impl<
	WENC:   Encrypter,
	WCS:    Checksummer,
    POWH:   Hasher,
    SGN:    Signer,
    HS:     Encrypter,
>
    WireSchemesInput<WENC, WCS, POWH, SGN, HS>
{
    pub fn ref_encrypter(&self)             -> &Alt<WENC>           { &self.enc }
    pub fn ref_checksummer(&self)           -> &Alt<WCS>            { &self.csum }
    pub fn ref_pow_hasher(&self)            -> &Alt<POWH>           { &self.powh }
    pub fn ref_signer(&self)                -> &Alt<SGN>            { &self.sign }
    pub fn ref_handshake_encrypter(&self)   -> &Alt<HS>             { &self.hsenc }
    pub fn ref_chunk_config(&self)          -> &Option<ChunkConfig> { &self.chnk }
    pub fn ref_syntax(&self)                -> SyntaxRef            { self.syntax.clone() }

    pub fn own_encrypter(&mut self)     -> Alt<WENC>    { std::mem::replace(&mut self.enc, Alt::Unspecified) }
    pub fn own_checksummer(&mut self)   -> Alt<WCS>     { std::mem::replace(&mut self.csum, Alt::Unspecified) }
    pub fn own_pow_hasher(&mut self)    -> Alt<POWH>    { std::mem::replace(&mut self.powh, Alt::Unspecified) }
    pub fn own_signer(&mut self)        -> Alt<SGN>     { std::mem::replace(&mut self.sign, Alt::Unspecified) }
    pub fn own_handshake_encrypter(&mut self) -> Alt<HS> { std::mem::replace(&mut self.hsenc, Alt::Unspecified) }
    pub fn own_chunk_config(&mut self)  -> Option<ChunkConfig>  { std::mem::replace(&mut self.chnk, None) }

    pub fn clone_encrypter(&self)           -> Alt<WENC>                { self.enc.clone() }
    pub fn clone_checksummer(&self)         -> Alt<WCS>                 { self.csum.clone() }
    pub fn clone_pow_hasher(&self)          -> Alt<POWH>                { self.powh.clone() }
    pub fn clone_signer(&self)              -> Alt<SGN>                 { self.sign.clone() }
    pub fn clone_handshake_encrypter(&self) -> Alt<HS>                  { self.hsenc.clone() }
    pub fn clone_chunk_config(&self)        -> Option<ChunkConfig>      { self.chnk.clone() }

    pub fn set_signer(mut self, signer: Option<SGN>) -> Self {
        self.sign = Alt::Specific(signer);
        self
    }
    pub fn set_syntax(mut self, syntax: SyntaxRef) -> Self {
        self.syntax = syntax;
        self
    }
}

#[derive(Clone, Debug)]
pub struct WireSchemes<
	WENC:   Encrypter, // wire encrypter
	WCS:    Checksummer, // wire checksummer
    POWH:   Hasher,
    SGN:    Signer, // digital signature
    HS:     Encrypter, // handshake encryption
>{
    pub enc:    EncrypterDefAlt<EncryptionScheme, WENC>,
    pub csum:   ChecksummerDefAlt<ChecksumScheme, WCS>,
    pub powh:   HasherDefAlt<HashScheme, POWH>,
    pub sign:   SignerDefAlt<SignatureScheme, SGN>,
    pub hsenc:  EncrypterDefAlt<EncryptionScheme, HS>,
    pub chnk:   ChunkConfig,
    pub syntax: SyntaxRef,
}

impl<
	WENC:   Encrypter,
	WCS:    Checksummer,
    POWH:   Hasher,
    SGN:    Signer,
    HS:     Encrypter,
>
    Default for WireSchemes<WENC, WCS, POWH, SGN, HS>
{
    fn default() -> Self {
        Self {
            enc:    EncrypterDefAlt(DefAlt::None),
            csum:   ChecksummerDefAlt(DefAlt::None),
            powh:   HasherDefAlt(DefAlt::None),
            sign:   SignerDefAlt(DefAlt::None),
            hsenc:  EncrypterDefAlt(DefAlt::None),
            chnk:   ChunkConfig::default(),
            syntax: SyntaxRef(Arc::new(syntax::empty())),
        }
    }
}

impl<
	WENC:   Encrypter,
	WCS:    Checksummer,
    POWH:   Hasher,
    SGN:    Signer,
    HS:     Encrypter,
>
    From<WireSchemesInput<WENC, WCS, POWH, SGN, HS>> for WireSchemes<WENC, WCS, POWH, SGN, HS>
{
    fn from(input: WireSchemesInput<WENC, WCS, POWH, SGN, HS>) -> Self {
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
            syntax: input.syntax,
        }
    }
}

impl<
	WENC:   Encrypter,
	WCS:    Checksummer,
    POWH:   Hasher,
    SGN:    Signer,
    HS:     Encrypter,
>
    WireSchemes<WENC, WCS, POWH, SGN, HS>
{
    pub fn ref_encrypter(&self)             -> &EncrypterDefAlt<EncryptionScheme, WENC> { &self.enc }
    pub fn ref_checksummer(&self)           -> &ChecksummerDefAlt<ChecksumScheme, WCS>  { &self.csum }
    pub fn ref_pow_hasher(&self)            -> &HasherDefAlt<HashScheme, POWH>          { &self.powh }
    pub fn ref_signer(&self)                -> &SignerDefAlt<SignatureScheme, SGN>      { &self.sign }
    pub fn ref_handshake_encrypter(&self)   -> &EncrypterDefAlt<EncryptionScheme, HS>   { &self.hsenc }
    pub fn ref_chunk_config(&self)          -> &ChunkConfig                             { &self.chnk }
    pub fn ref_syntax(&self)                -> SyntaxRef                                { self.syntax.clone() }

    pub fn clone_encrypter(&self)           -> EncrypterDefAlt<EncryptionScheme, WENC>  { self.enc.clone() }
    pub fn clone_checksummer(&self)         -> ChecksummerDefAlt<ChecksumScheme, WCS>   { self.csum.clone() }
    pub fn clone_pow_hasher(&self)          -> HasherDefAlt<HashScheme, POWH>           { self.powh.clone() }
    pub fn clone_signer(&self)              -> SignerDefAlt<SignatureScheme, SGN>       { self.sign.clone() }
    pub fn clone_handshake_encrypter(&self) -> EncrypterDefAlt<EncryptionScheme, HS>    { self.hsenc.clone() }
    pub fn clone_chunk_config(&self)        -> ChunkConfig                              { self.chnk.clone() }

    //pub fn clone_syntax(&self) -> Outcome<SyntaxRef> {
    //    match &self.syntax {
    //        DefAlt::Given(inner) | DefAlt::Default(inner) => Ok(inner.clone()),
    //        DefAlt::None => Err(err!(errmsg!(
    //            "The wire syntax protocol should never be DefAlt::None.",
    //        ), Bug, Missing, Data)),
    //    }
    //}

    //pub fn set_chunk_config(mut self, chnk: Option<ChunkConfig>) -> Self {
    //    self.chnk = chnk;
    //    self
    //}
}
