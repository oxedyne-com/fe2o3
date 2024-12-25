use crate::prelude::*;


pub trait ParseId<const L: usize> {
    fn parse_id(s: &str) -> Outcome<Self> where Self: Sized;
}

impl ParseId<1> for u8 {
    fn parse_id(s: &str) -> Outcome<Self> {
        Ok(res!(u8::from_str(s)))    
    }
}
impl ParseId<2> for u16 {
    fn parse_id(s: &str) -> Outcome<Self> {
        Ok(res!(u16::from_str(s)))    
    }
}
impl ParseId<4> for u32 {
    fn parse_id(s: &str) -> Outcome<Self> {
        Ok(res!(u32::from_str(s)))    
    }
}
impl ParseId<8> for u64 {
    fn parse_id(s: &str) -> Outcome<Self> {
        Ok(res!(u64::from_str(s)))    
    }
}
impl ParseId<16> for u128 {
    fn parse_id(s: &str) -> Outcome<Self> {
        Ok(res!(u128::from_str(s)))    
    }
}

