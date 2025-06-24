use oxedyne_fe2o3_core::{
    prelude::*,
    alt::{
        Alt,
        DefAlt,
    },
    byte::byte_slices_equal,
};
use oxedyne_fe2o3_iop_hash::csum::Checksummer;
use oxedyne_fe2o3_namex::{
    id::{
        LocalId,
        InNamex,
        NamexId,
    },
};

use std::{
    fmt,
    str,
};

use crc32fast;

#[derive(Clone)]
pub enum ChecksumScheme {
    Crc32(crc32fast::Hasher),
}

impl fmt::Debug for ChecksumScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Crc32(..) => write!(f, "Crc32"),
        }
    }
}
    
impl InNamex for ChecksumScheme {

    fn name_id(&self) -> Outcome<NamexId> {
	    Ok(match self {
            Self::Crc32(..) =>
                res!(NamexId::try_from("+n9EN0dPKFF7q6iiwjtkgAUyT3odnl95JwnQd4z+jfw=")),
        })
    }

    fn local_id(&self) -> LocalId {
	    match self {
            Self::Crc32(..) => LocalId(1),
        }
    }

    fn assoc_names_base64(
        gname: &'static str,
    )
        -> Outcome<Option<Vec<(
            &'static str,
            &'static str,
        )>>>
    {
        let ids = match gname {
            "schemes" => [
	            ("CRC32", "9m5ja11mi86oosyB7LG4Kvcx83ln3stpD0/8Pnq01tE="),
            ],
            _ => return Err(err!(
                "The Namex group name '{}' is not recognised for ChecksumScheme.", gname;
            Invalid, Input)),
        };
        Ok(if ids.len() == 0 {
            None
        } else {
            Some(ids.to_vec())
        })
    }
}

impl Checksummer for ChecksumScheme {

    fn len(&self) -> Outcome<usize> {
        match self {
            Self::Crc32(_) => Ok(4),
        }
    }

    /// Calculate the checksum of the supplied byte slice.
    fn calculate(mut self, buf: &[u8]) -> Outcome<Vec<u8>> {
        res!(self.update(buf));
        self.finalize()
    }

    fn update(&mut self, buf: &[u8]) -> Outcome<()> {
        match self {
            Self::Crc32(hasher) => hasher.update(&buf),
        }
        Ok(())
    }

    /// Calculate the checksum of the supplied byte slice.
    fn finalize(self) -> Outcome<Vec<u8>> {
        match self {
            Self::Crc32(hasher) => {
                let csum = hasher.finalize();
                Ok(csum.to_be_bytes().to_vec())
            },
        }
    }

    fn copy(&self, buf: &[u8]) -> Outcome<Vec<u8>> {
        let clen = res!(self.len());
        if buf.len() < clen {
            return Err(err!(
                "Cannot snip checksum from given byte slice of length {} \
                because it is less than the required minimum {}.",
                buf.len(), clen;
            Input, Mismatch))
        }
        Ok(buf[buf.len() - clen..].to_vec())
    }

    fn append(self, mut buf: Vec<u8>) -> Outcome<(Vec<u8>, Vec<u8>)> {
        let csum = res!(self.calculate(&buf));
        buf.extend_from_slice(&csum);
        Ok((buf, csum))
    }

    /// Assumes that the specified buffer contains data with an appended checksum.  This function
    /// returns an error if the checksum cannot be reproduced.
    fn verify(self, buf: &[u8]) -> Outcome<Vec<u8>> {
        let clen = res!(self.len());
        if buf.len() < clen {
            return Err(err!(
                "A request to verify the checksum of a slice of bytes of length {} \
                requires the slice to be at least the length of a checksum, {}.",
                buf.len(), clen;
            Input, Mismatch))
        }
        let csum1 = res!(self.calculate(&buf[..buf.len() - clen]));
        let csum2 = &buf[buf.len() - clen..];
        res!(byte_slices_equal(&csum1, csum2), Checksum);
        Ok(csum1)
    }

    fn read_bytes<R: std::io::Read>(
        &self,
        r: &mut R,
    )
        -> Outcome<(Vec<u8>, usize)>
    {
        let clen = res!(self.len());
        let mut csum = vec![0u8; clen];
        res!(r.read_exact(&mut csum));
        Ok((csum.to_vec(), clen))
    }

}

impl str::FromStr for ChecksumScheme {
    type Err = Error<ErrTag>;

    fn from_str(name: &str) -> std::result::Result<Self, Self::Err> {
        match name {
            "CRC32" => Ok(Self::new_crc32()),
            _ => Err(err!(
                "The checksum scheme '{}' is not recognised.", name;
            Invalid, Input)),
        }
    }
}

impl TryFrom<LocalId> for ChecksumScheme {
    type Error = Error<ErrTag>;

    fn try_from(n: LocalId) -> std::result::Result<Self, Self::Error> {
        match n {
            LocalId(1) => Ok(Self::new_crc32()),
            _ => Err(err!(
                "The checksum scheme with local id {} is not recognised.", n;
            Invalid, Input)),
        }
    }
}

impl ChecksumScheme {

    pub const CRC32_BYTE_LEN:       usize   = 4;
    //pub const USR_VERSION:          SemVer  = SemVer::new(0,0,1);

    //pub fn new(name: &str) -> Outcome<Self> {
    //    match name {
    //        "CRC32" => Ok(Self::new_crc32()),
    //        _ => Err(err!(
    //            "The checksum scheme '{}' is not recognised.", name,
    //        ), Invalid, Input)),
    //    }
    //}

    pub fn new_crc32() -> Self {
        Self::Crc32(crc32fast::Hasher::new())
    }

    pub fn compare(csum1: &[u8], csum2: &[u8]) -> Outcome<()> {
        res!(byte_slices_equal(csum1, csum2), Checksum);
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct ChecksummerDefAlt<
    D: Checksummer,
    G: Checksummer,
> (pub DefAlt<D, G>);

impl<
    D: Checksummer,
    G: Checksummer,
>
    std::ops::Deref for ChecksummerDefAlt<D, G>
{
    type Target = DefAlt<D, G>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl<
    D: Checksummer,
    G: Checksummer,
>
    From<Option<G>> for ChecksummerDefAlt<D, G>
{
    fn from(opt: Option<G>) -> Self {
        Self(
            DefAlt::from(opt),
        )
    }
}

impl<
    D: Checksummer,
    G: Checksummer,
>
    From<Alt<G>> for ChecksummerDefAlt<D, G>
{
    fn from(alt: Alt<G>) -> Self {
        Self(
            DefAlt::from(alt),
        )
    }
}

impl<
    D: Checksummer,
    G: Checksummer,
>
    fmt::Display for ChecksummerDefAlt<D, G>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl<
    D: Checksummer + InNamex,
    G: Checksummer + InNamex,
>
    InNamex for ChecksummerDefAlt<D, G>
{
    fn name_id(&self) -> Outcome<NamexId> {
        match &self.0 {
            DefAlt::Default(inner) => inner.name_id(),
            DefAlt::Given(inner) => inner.name_id(),
            DefAlt::None => Err(err!(
                "No Namex id can be specified for DefAlt::None.";
            Missing, Bug)),
        }
    }

    fn local_id(&self) -> LocalId {
        match &self.0 {
            DefAlt::Default(inner)  => inner.local_id(),
            DefAlt::Given(inner)    => inner.local_id(),
            DefAlt::None            => LocalId::default(),
        }
    }

    fn assoc_names_base64(
        gname: &'static str,
    )
        -> Outcome<Option<Vec<(
            &'static str,
            &'static str,
        )>>>
    {
        match res!(D::assoc_names_base64(gname)) {
            Some(mut vd) => match res!(G::assoc_names_base64(gname)) {
                Some(vg) => {
                    vd.extend(vg);
                    Ok(Some(vd))
                },
                None => Ok(Some(vd)),
            },
            None => match res!(G::assoc_names_base64(gname)) {
                Some(vg) => Ok(Some(vg)),
                None => Ok(None),
            },
        }
    }
}

impl<
    D: Checksummer,
    G: Checksummer,
>
    Checksummer for ChecksummerDefAlt<D, G>
{
    fn len(&self) -> Outcome<usize> {
        match &self.0 {
            DefAlt::Default(inner) => inner.len(),
            DefAlt::Given(inner) => inner.len(),
            DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn calculate(self, buf: &[u8]) -> Outcome<Vec<u8>> {
        match self.0 {
            DefAlt::Default(inner) => inner.calculate(buf),
            DefAlt::Given(inner) => inner.calculate(buf),
            DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn update(&mut self, buf: &[u8]) -> Outcome<()> {
        match &mut self.0 {
            DefAlt::Default(inner) => inner.update(buf),
            DefAlt::Given(inner) => inner.update(buf),
            DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn finalize(self) -> Outcome<Vec<u8>> {
        match self.0 {
            DefAlt::Default(inner) => inner.finalize(),
            DefAlt::Given(inner) => inner.finalize(),
            DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn copy(&self, buf: &[u8]) -> Outcome<Vec<u8>>{
        match &self.0 {
            DefAlt::Default(inner) => inner.copy(buf),
            DefAlt::Given(inner) => inner.copy(buf),
            DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn append(self, buf: Vec<u8>) -> Outcome<(Vec<u8>, Vec<u8>)> {
        match self.0 {
            DefAlt::Default(inner) => inner.append(buf),
            DefAlt::Given(inner) => inner.append(buf),
            DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn verify(self, buf: &[u8]) -> Outcome<Vec<u8>> {
        match self.0 {
            DefAlt::Default(inner) => inner.verify(buf),
            DefAlt::Given(inner) => inner.verify(buf),
            DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
        }
    }

    fn read_bytes<R: std::io::Read>(&self, r: &mut R) -> Outcome<(Vec<u8>, usize)> {
        match &self.0 {
            DefAlt::Default(inner) => inner.read_bytes(r),
            DefAlt::Given(inner) => inner.read_bytes(r),
            DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
        }
    }
}

impl<
    D: Checksummer,
    G: Checksummer,
>
    ChecksummerDefAlt<D, G>
{
    pub const HASHER_MISSING_MSG: &'static str = "Checksum hasher not specified.";

    pub fn or_len<OR: Checksummer>(&self, alt: &Alt<OR>) -> Outcome<usize> {
        match &alt {
            Alt::Specific(Some(inner)) => inner.len(),
            Alt::Specific(None) => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.len(),
                DefAlt::Given(inner) => inner.len(),
                DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_calculate<OR: Checksummer>(self, buf: &[u8], alt: Alt<OR>) -> Outcome<Vec<u8>> {
        match alt {
            Alt::Specific(Some(inner)) => inner.calculate(buf),
            Alt::Specific(None) => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match self.0 {
                DefAlt::Default(inner) => inner.calculate(buf),
                DefAlt::Given(inner) => inner.calculate(buf),
                DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_update<OR: Checksummer>(&mut self, buf: &[u8], mut alt: &mut Alt<OR>) -> Outcome<()> {
        match &mut alt {
            Alt::Specific(Some(inner)) => inner.update(buf),
            Alt::Specific(None) => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &mut self.0 {
                DefAlt::Default(inner) => inner.update(buf),
                DefAlt::Given(inner) => inner.update(buf),
                DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_finalize<OR: Checksummer>(self, alt: Alt<OR>) -> Outcome<Vec<u8>> {
        match alt {
            Alt::Specific(Some(inner)) => inner.finalize(),
            Alt::Specific(None) => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match self.0 {
                DefAlt::Default(inner) => inner.finalize(),
                DefAlt::Given(inner) => inner.finalize(),
                DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_copy<OR: Checksummer>(&self, buf: &[u8], alt: &Alt<OR>) -> Outcome<Vec<u8>>{
        match &alt {
            Alt::Specific(Some(inner)) => inner.copy(buf),
            Alt::Specific(None) => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.copy(buf),
                DefAlt::Given(inner) => inner.copy(buf),
                DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_append<OR: Checksummer>(self, buf: Vec<u8>, alt: Alt<OR>) -> Outcome<(Vec<u8>, Vec<u8>)> {
        match alt {
            Alt::Specific(Some(inner)) => inner.append(buf),
            Alt::Specific(None) => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match self.0 {
                DefAlt::Default(inner) => inner.append(buf),
                DefAlt::Given(inner) => inner.append(buf),
                DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_verify<OR: Checksummer>(self, buf: &[u8], alt: Alt<OR>) -> Outcome<Vec<u8>> {
        match alt {
            Alt::Specific(Some(inner)) => inner.verify(buf),
            Alt::Specific(None) => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match self.0 {
                DefAlt::Default(inner) => inner.verify(buf),
                DefAlt::Given(inner) => inner.verify(buf),
                DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }

    pub fn or_read_bytes<
        R: std::io::Read,
        OR: Checksummer,
    >(
        &self,
        r: &mut R,
        alt: &Alt<OR>,
    )
        -> Outcome<(Vec<u8>, usize)>
    {
        match &alt {
            Alt::Specific(Some(inner)) => inner.read_bytes(r),
            Alt::Specific(None) => Err(err!("{}", Self::HASHER_MISSING_MSG;
                Configuration, Missing)),
            Alt::Unspecified => match &self.0 {
                DefAlt::Default(inner) => inner.read_bytes(r),
                DefAlt::Given(inner) => inner.read_bytes(r),
                DefAlt::None => Err(err!("{}", Self::HASHER_MISSING_MSG;
                    Configuration, Missing)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_checksum() -> Outcome<()> {
        let byts = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let crc32 = ChecksumScheme::new_crc32();
        let (csum, _) = res!(crc32.read_bytes(&mut byts.as_slice()));
        msg!("csum = {:?}", csum);
        Ok(())
    }
}
