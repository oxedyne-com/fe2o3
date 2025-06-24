use crate::scheme::SchemeTimestamp;

use oxedyne_fe2o3_core::{
    prelude::*,
    mem::Extract,
    rand::RanDef,
};
use oxedyne_fe2o3_data::{
    ring::RingBuffer,
    time::Timestamped,
};
use oxedyne_fe2o3_jdat::{
    prelude::*,
    try_extract_tup2dat,
    //try_extract_tup3dat,
    tup2dat,
    //tup3dat,
    file::JdatFile,
};
use oxedyne_fe2o3_namex::id::LocalId as SchemeLocalId;

use std::{
    fmt,
};

use rand_core::{
    OsRng,
    RngCore,
};

use secrecy::{
    ExposeSecret,
    Secret,
};

/// A private key with `SchemeTimestamp` metadata.  A heap `Vec` is used since the key accomodates
/// different schemes, so the length of the key is not known at compile time.
pub struct SecretKey {
    pub key: Secret<Vec<u8>>,
    pub sts: SchemeTimestamp,
}

impl Clone for SecretKey {
    fn clone(&self) -> Self {
        Self {
            key: {
                let sk = self.key.expose_secret();
                Secret::new(sk.clone())
            },
            sts: self.sts.clone(),
        }
    }
}

impl Default for SecretKey {
    fn default() -> Self {
        Self {
            key: Secret::new(Vec::new()),
            sts: SchemeTimestamp::default(),
        }
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sk = self.key.expose_secret();
        write!(f, "SecretKey{{ key: [{}] bytes, sts: {:?}, k}}", sk.len(), self.sts)
    }
}
    
impl ToDat for SecretKey {
    fn to_dat(&self) -> Outcome<Dat> {
        let sk = self.key.expose_secret();
        Ok(tup2dat![
            Dat::bytdat(sk.clone()),
            res!(self.sts.to_dat()),
        ])
    }
}

impl FromDat for SecretKey {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut result = SecretKey::default();
        let mut v = try_extract_tup2dat!(dat);
        result.key = Secret::new(try_extract_dat!(v[0].extract(), BU8, BU16, BU32, BU64));
        result.sts =  res!(SchemeTimestamp::from_dat(v[1].extract()));
        Ok(result)
    }
}

impl SecretKey {
    pub fn new(
        sts: SchemeTimestamp,
        key: Secret<Vec<u8>>,
    )
        -> Self
    {
        Self {
            sts,
            key,
        }
    }

    pub fn now(
        id: SchemeLocalId,
        key: Secret<Vec<u8>>,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            sts: res!(SchemeTimestamp::now(id)),
            key,
        })
    }
}

/// A public key with `SchemeTimestamp` metadata, stored on the heap.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct PublicKey {
    pub sts: SchemeTimestamp, // Derived ordering starts with the first field here.
    pub key: Vec<u8>,
}

impl Default for PublicKey {
    fn default() -> Self {
        Self {
            key: Vec::new(),
            sts: SchemeTimestamp::default(),
        }
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey{{ key: [{}] bytes, sts: {:?}, k}}", self.key.len(), self.sts)
    }
}
    
impl ToDat for PublicKey {
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(tup2dat![
            Dat::bytdat(self.key.clone()),
            res!(self.sts.to_dat()),
        ])
    }
}

impl FromDat for PublicKey {
    fn from_dat(dat: Dat) -> Outcome<Self> {
        let mut result = PublicKey::default();
        let mut v = try_extract_tup2dat!(dat);
        result.key = try_extract_dat!(v[0].extract(), BU8, BU16, BU32, BU64);
        result.sts = res!(SchemeTimestamp::from_dat(v[1].extract()));
        Ok(result)
    }
}

impl PublicKey {
    pub fn new(
        sts: SchemeTimestamp,
        key: Vec<u8>,
    )
        -> Self
    {
        Self {
            key,
            sts,
        }
    }

    pub fn now(
        id: SchemeLocalId,
        key: Vec<u8>,
    )
        -> Outcome<Self>
    {
        Ok(Self {
            sts: res!(SchemeTimestamp::now(id)),
            key,
        })
    }
}

/// An optional public and private key pair with no additional metadata.
#[derive(Default)]
pub struct Keys<
    const PK_LEN: usize,
    const SK_LEN: usize,
> {
    pub pk:     Option<[u8; PK_LEN]>,
    pub sks:    Option<Secret<[u8; SK_LEN]>>,
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    Clone for Keys<PK_LEN, SK_LEN>
{
    fn clone(&self) -> Self {
        Self {
            pk: self.pk.clone(),
            sks: match &self.sks {
                Some(sks) => {
                    let sk = sks.expose_secret();
                    Some(Secret::new(sk.clone()))
                },
                None => None,
            },
        }
    }
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    RanDef for Keys<PK_LEN, SK_LEN>
{
    fn randef() -> Self {
        let mut pk = [0u8; PK_LEN];
        let mut sk = [0u8; SK_LEN];
        OsRng.fill_bytes(&mut pk);
        OsRng.fill_bytes(&mut sk);
        Self {
            pk: Some(pk),
            sks: Some(Secret::new(sk)),
        }
    }
}

impl<
    const PK_LEN: usize,
    const SK_LEN: usize,
>
    Keys<PK_LEN, SK_LEN>
{
    pub fn new(
        pk: Option<[u8; PK_LEN]>,
        sks: Option<Secret<[u8; SK_LEN]>>,
    )
        -> Self
    {
        Self {
            pk,
            sks,
        }
    }

    pub fn randef_sk_only() -> Self {
        let mut sk = [0u8; SK_LEN];
        OsRng.fill_bytes(&mut sk);
        Self {
            pk: None,
            sks: Some(Secret::new(sk)),
        }
    }

    pub fn randef_pk_only() -> Self {
        let mut pk = [0u8; PK_LEN];
        OsRng.fill_bytes(&mut pk);
        Self {
            pk: Some(pk),
            sks: None,
        }
    }
}

/// Contains metadata, key derivation function (KDF) configuration strings which are used to
/// generate encryption keys from a passphrase, and hashes of recent passphrases.
#[derive(Clone, Default)]
pub struct Wallet<
    const PH: usize,
    D: Clone + Default + fmt::Debug + FromDat + ToDat,
> {
    metadata:   DaticleMap,
    kdf_cfgs:   DaticleMap,
    enc_secs:   DaticleMap,
    passhashes: RingBuffer<PH, Timestamped<D>>,
}

impl<
    const PH: usize,
    D: Clone + Default + fmt::Debug + FromDat + ToDat,
>
    ToDat for Wallet<PH, D>
{
    fn to_dat(&self) -> Outcome<Dat> {
        Ok(omapdat!{
            "metadata" => Dat::Map(self.metadata.clone()),
            "app_hashes" => Dat::Map(self.kdf_cfgs.clone()),
            "app_encrypted_secrets" => Dat::Map(self.enc_secs.clone()),
            "wallet_pass_hashes" => res!(self.passhashes.to_dat()),
        })
    }
}

impl<
    const PH: usize,
    D: Clone + Default + fmt::Debug + FromDat + ToDat,
>
    FromDat for Wallet<PH, D>
{
    fn from_dat(mut dat: Dat) -> Outcome<Self> {
        if dat.kind() == Kind::OrdMap {
            let ring_buf_dat = res!(dat.map_remove_must(&dat!("wallet_pass_hashes")));
            Ok(Self {
                metadata:   try_extract_dat!(res!(dat.map_remove_must(&dat!("metadata"))), Map),
                kdf_cfgs:   try_extract_dat!(res!(dat.map_remove_must(&dat!("app_hashes"))), Map),
                enc_secs:   try_extract_dat!(res!(dat.map_remove_must(&dat!("app_encrypted_secrets"))), Map),
                passhashes: res!(RingBuffer::from_dat(ring_buf_dat)),
            })
        } else {
            Err(err!(
                "Wallet must be a Dat::OrdMap, found a {:?}.", dat.kind();
            Input, Invalid, Mismatch))
        }
    }
}

impl<
    const PH: usize,
    D: Clone + Default + fmt::Debug + FromDat + ToDat,
>
    JdatFile for Wallet<PH, D> {}

impl<
    const PH: usize,
    D: Clone + Default + fmt::Debug + FromDat + ToDat,
>
    Wallet<PH, D>
{
    pub fn new(
        metadata:   DaticleMap,
        kdf_cfgs:   DaticleMap,
        enc_secs:   DaticleMap,
        passhashes: RingBuffer<PH, Timestamped<D>>,
    )
        -> Self
    {
        Self {
            metadata,
            kdf_cfgs,
            enc_secs,
            passhashes,
        }
    }

    pub fn metadata(&self)      -> &DaticleMap      { &self.metadata }
    pub fn kdf_cfgs(&self)      -> &DaticleMap      { &self.kdf_cfgs }
    pub fn enc_secs(&self)      -> &DaticleMap      { &self.enc_secs }
    pub fn passhashes(&self)    -> &RingBuffer<PH, Timestamped<D>> { &self.passhashes }
    pub fn metadata_mut(&mut self)      -> &mut DaticleMap      { &mut self.metadata }
    pub fn kdf_cfgs_mut(&mut self)      -> &mut DaticleMap      { &mut self.kdf_cfgs }
    pub fn enc_secs_mut(&mut self)      -> &mut DaticleMap      { &mut self.enc_secs }
    pub fn passhashes_mut(&mut self)    -> &mut RingBuffer<PH, Timestamped<D>> {
        &mut self.passhashes
    }
}

///// A public key with an optional private key that contains a `SchemeTimestamp`.
//#[derive(Clone, Debug)]
//pub struct KeyPair<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//> {
//    pub pk: [u8; PK_LEN],
//    pub sk_opt: Option<SecretKey<SK_LEN>>, // Includes scheme id and timestamp.
//}
//
//impl<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//>
//    Default for KeyPair<PK_LEN, SK_LEN>
//{
//    fn default() -> Self {
//        Self {
//            pk: [0u8; PK_LEN],
//            sk_opt: Some(SecretKey::<SK_LEN>::default()),
//        }
//    }
//}
//
//impl<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//>
//    RanDef for KeyPair<PK_LEN, SK_LEN>
//{
//    fn randef() -> Self {
//        let mut pk = [0u8; PK_LEN];
//        OsRng.fill_bytes(&mut pk);
//        Self {
//            pk,
//            sk_opt: Some(SecretKey::<SK_LEN>::randef()),
//        }
//    }
//}
//
//impl<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//>
//    ToDat for KeyPair<PK_LEN, SK_LEN>
//{
//    fn to_dat(&self) -> Outcome<Dat> {
//        Ok(tup2dat![
//            Dat::bytdat(self.pk.to_vec()),
//            match &self.sk_opt {
//                Some(sk) => Dat::Opt(Box::new(Some(res!(sk.to_dat())))),
//                None => Dat::Opt(Box::new(None)),
//            },
//        ])
//    }
//}
//
//impl<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//>
//    FromDat for KeyPair<PK_LEN, SK_LEN> {
//    fn from_dat(dat: Dat) -> Outcome<Self> {
//        let mut result = KeyPair::default();
//        let mut v = try_extract_tup2dat!(dat);
//        let pk_vec = try_extract_dat!(v[0].extract(), BU8, BU16, BU32, BU64);
//        result.pk = res!(<[u8; PK_LEN]>::try_from(&pk_vec[..]));
//        result.sk_opt = match *try_extract_dat!(v[1].extract(), Opt) {
//            Some(d) => Some(res!(SecretKey::from_dat(d))),
//            None => None,
//        };
//        Ok(result)
//    }
//}
//
////#[derive(Clone)]
//pub struct Wallet<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//> {
//    pub sym:    Vec<SecretKey<SK_LEN>>,
//    pub asym:   Vec<KeyPair<PK_LEN, SK_LEN>>,
//    pub sign1:  Vec<KeyPair<PK_LEN, SK_LEN>>,
//    pub sign2:  Vec<KeyPair<PK_LEN, SK_LEN>>,
//}
//
//impl<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//>
//    Default for Wallet<PK_LEN, SK_LEN>
//{
//    fn default() -> Self {
//        Self {
//            sym:    Vec::new(),
//            asym:   Vec::new(),
//            sign1:  Vec::new(),
//            sign2:  Vec::new(),
//        }
//    }
//}
//
//impl<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//>
//    Wallet<PK_LEN, SK_LEN>
//{
//    pub fn to_dat(&self) -> Outcome<Dat> {
//        let mut map = BTreeMap::new();
//        let mut list = Vec::new();
//        for item in &self.sym {
//            list.push(res!(item.to_dat()));
//        }
//        map.insert(dat!("sym"), Dat::List(list));
//        let mut list = Vec::new();
//        for item in &self.asym {
//            list.push(res!(item.to_dat()));
//        }
//        map.insert(dat!("asym"), Dat::List(list));
//        let mut list = Vec::new();
//        for item in &self.sign1 {
//            list.push(res!(item.to_dat()));
//        }
//        map.insert(dat!("sign1"), Dat::List(list));
//        let mut list = Vec::new();
//        for item in &self.sign2 {
//            list.push(res!(item.to_dat()));
//        }
//        map.insert(dat!("sign2"), Dat::List(list));
//        Ok(Dat::Map(map))
//    }
//
//}
//
//impl<
//    const PK_LEN: usize,
//    const SK_LEN: usize,
//>
//    FromDat for Wallet<PK_LEN, SK_LEN>
//{
//    fn from_dat(dat: Dat) -> Outcome<Self> {
//        let mut result = Self::default();
//        if let Dat::List(v) = res!(dat.map_get_type_must(&dat!("sym"), &[&Kind::List])).clone() {
//            for d in v {
//                result.sym.push(res!(SecretKey::from_dat(d)));  
//            }
//        }
//        if let Dat::List(v) = res!(dat.map_get_type_must(&dat!("asym"), &[&Kind::List])).clone() {
//            for d in v {
//                result.asym.push(res!(KeyPair::from_dat(d)));  
//            }
//        }
//        if let Dat::List(v) = res!(dat.map_get_type_must(&dat!("sign1"), &[&Kind::List])).clone() {
//            for d in v {
//                result.sign1.push(res!(KeyPair::from_dat(d)));  
//            }
//        }
//        if let Dat::List(v) = res!(dat.map_get_type_must(&dat!("sign2"), &[&Kind::List])).clone() {
//            for d in v {
//                result.sign2.push(res!(KeyPair::from_dat(d)));  
//            }
//        }
//        Ok(result)
//    }
//}
//
//#[derive(Clone, Debug, Default)]
//pub struct AllPublicKeys {
//    pub asym:   Option<PublicKey>,
//    pub sign1:  Option<PublicKey>,
//    pub sign2:  Option<PublicKey>,
//}
//
//impl ToDat for AllPublicKeys {
//    fn to_dat(&self) -> Outcome<Dat> {
//        Ok(tup3dat![
//            match &self.asym {
//                None => Dat::Opt(Box::new(None)),
//                Some(pubkey) => Dat::Opt(Box::new(Some(res!(pubkey.to_dat())))),
//            },
//            match &self.sign1 {
//                None => Dat::Opt(Box::new(None)),
//                Some(pubkey) => Dat::Opt(Box::new(Some(res!(pubkey.to_dat())))),
//            },
//            match &self.sign2 {
//                None => Dat::Opt(Box::new(None)),
//                Some(pubkey) => Dat::Opt(Box::new(Some(res!(pubkey.to_dat())))),
//            },
//        ])
//    }
//}
//
//impl FromDat for AllPublicKeys {
//    fn from_dat(dat: Dat) -> Outcome<Self> {
//        let mut result = Self::default();
//        let mut v = try_extract_tup3dat!(dat);
//        match *try_extract_dat!(v[0].extract(), Opt) {
//            Some(d) => result.asym = Some(res!(PublicKey::from_dat(d))),
//            None => (),
//        }
//        match *try_extract_dat!(v[1].extract(), Opt) {
//            Some(d) => result.sign1 = Some(res!(PublicKey::from_dat(d))),
//            None => (),
//        }
//        match *try_extract_dat!(v[2].extract(), Opt) {
//            Some(d) => result.sign2 = Some(res!(PublicKey::from_dat(d))),
//            None => (),
//        }
//        Ok(result)
//    }
//}
//
//pub struct Dilithium2;
//
//impl Dilithium2 {
//
//    pub const PUBLIC_KEY_BYTES: usize = dilithium2::public_key_bytes();
//    pub const SECRET_KEY_BYTES: usize = dilithium2::secret_key_bytes();
//
//    pub fn public_key_from_bytes(byts: &[u8]) -> Outcome<()> {
//        res!(dilithium2::PublicKey::from_bytes(byts));
//        Ok(())
//    }
//
//    pub fn public_key_as_bytes<'a>(pk: &'a dilithium2::PublicKey) -> &'a [u8] {
//        pk.as_bytes()
//    }
//
//    pub fn keypair() -> (dilithium2::PublicKey, dilithium2::SecretKey) {
//        dilithium2::keypair()
//    }
//
//    pub fn sign(
//        msg:    &[u8],
//        sk:     &dilithium2::SecretKey,
//    )
//        -> dilithium2::SignedMessage
//    {
//        dilithium2::sign(msg, sk)
//    }
//
//    //let message = vec![0, 1, 2, 3, 4, 5];
//    //let (pk, sk) = keypair();
//    //let sm = sign(&message, &sk);
//    //let verifiedmsg = open(&sm, &pk).unwrap();
//    //assert!(verifiedmsg == message);
//    
//    pub fn open(
//        sm:     &dilithium2::SignedMessage,
//        pk:     &dilithium2::PublicKey
//    )
//        -> Outcome<Vec<u8>>
//    {
//        match dilithium2::open(sm, pk) {
//            Ok(verified_msg) => Ok(verified_msg),
//            Err(e) => Err(err!(e, errmsg!(
//                "While attempting to open the message signed using Dilithium2.",
//            ), ErrTag::Invalid, ErrTag::Input)),
//        }
//    }
//
//    pub fn detached_sign(
//        msg:    &[u8],
//        sk:     &dilithium2::SecretKey,
//    )
//        -> dilithium2::DetachedSignature
//    {
//        dilithium2::detached_sign(msg, sk)
//    }
//
//    pub fn verify_detached_signature(
//        sig:    &dilithium2::DetachedSignature,
//        msg:    &[u8],
//        pk:     &dilithium2::PublicKey
//    )
//        -> Outcome<()>
//    {
//        match dilithium2::verify_detached_signature(sig, msg, pk) {
//            Ok(()) => Ok(()),
//            Err(e) => Err(err!(e, errmsg!(
//                "While attempting to verify detached Dilithium2 signature.",
//            ), ErrTag::Invalid, ErrTag::Input)),
//        }
//    }
//
//}
//
//pub struct DetachedSignature(dilithium2::DetachedSignature);
//
//impl Deref for DetachedSignature {
//    type Target = dilithium2::DetachedSignature;
//    fn deref(&self) -> &Self::Target { &self.0 }
//}
//
//impl From<dilithium2::DetachedSignature> for DetachedSignature {
//    fn from(inner: dilithium2::DetachedSignature) -> Self {
//        Self(inner)
//    }
//}
//
//impl DetachedSignature {
//
//    pub fn as_bytes(&self) -> &[u8] {
//        self.0.as_bytes()
//    }
//
//    pub fn from_bytes(byts: &[u8]) -> Outcome<Self> {
//        match dilithium2::DetachedSignature::from_bytes(byts) {
//            Ok(sig) => Ok(Self(sig)),
//            Err(e) => return Err(err!(e, errmsg!(
//                "While decoding a DetachedSignature from {} bytes {:02x?}.",
//                byts.len(), byts,
//            ), ErrTag::Invalid, ErrTag::Input)),
//        }
//    }
//
//}

//#[cfg(test)]
//mod tests {
//    use super::*;
//    use oxedyne_fe2o3_data::time::Timestamp;
//    use std::{
//        time::Duration,
//    };
//
//    #[test]
//    fn test_wallet_to_from_dat_00() -> Outcome<()> {
//        let mut secrets = DaticleMap::new();
//        let test_secret = vec![1u8,2,3,4];
//        // NOTE: We're not encrypting the secret here.
//        secrets.insert(dat!("o3db"), dat!(test_secret.clone()));
//        let mut passhashes = RingBuffer::default();
//        let ts1 = Timestamp(Duration::from_nanos(1234));
//        let ts2 = Timestamp(Duration::from_nanos(12345));
//        let phash1 = "phash1".to_string();
//        let phash2 = "phash2".to_string();
//        passhashes.set(Timestamped { data: phash1.clone(), t: ts1.clone() });
//        passhashes.adv();
//        passhashes.set(Timestamped { data: phash2.clone(), t: ts2.clone() });
//
//        let wallet: Wallet<2, String> = Wallet::new(
//            DaticleMap::new(),
//            DaticleMap::new(),
//            secrets,
//            passhashes,
//        );
//
//        let dat = res!(wallet.to_dat());
//        for line in dat.to_lines("  ", true) {
//            msg!("{}", line);
//        }
//        let wallet2: Wallet<2, String> = res!(Wallet::from_dat(dat));
//        let secrets2 = wallet2.enc_secs; 
//        match secrets2.get(&dat!("o3db")) {
//            Some(sec) => {
//                let v = try_extract_dat!(sec, BU64);
//                req!(v.len(), test_secret.len());
//                for (i, item) in v.iter().enumerate() {
//                    req!(test_secret[i], *item);
//                }
//            },
//            None => return Err(err!(
//                "Expected entry for 'o3db', found nothing.",
//            ), Test, Data, Missing)),
//        }
//        let mut passhashes2 = wallet2.passhashes;
//        match passhashes2.get() {
//            Some(tsph) => {
//                req!(ts1, tsph.t);
//                req!(phash1, tsph.data);
//            },
//            None => return Err(err!(
//                "Expected first entry in passhashes RingBuffer, found nothing.",
//            ), Test, Data, Missing)),
//        }
//        passhashes2.adv();
//        match passhashes2.get() {
//            Some(tsph) => {
//                req!(ts2, tsph.t);
//                req!(phash2, tsph.data);
//            },
//            None => return Err(err!(
//                "Expected second entry in passhashes RingBuffer, found nothing.",
//            ), Test, Data, Missing)),
//        }
//        Ok(())
//    }
//}
