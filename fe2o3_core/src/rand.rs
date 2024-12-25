use crate::byte::B32;

use std::cmp::PartialOrd;

use rand::{
    Rng,
    thread_rng,
};
use rand_core::{
    OsRng,
    RngCore,
};


/// Random default value.
pub trait RanDef {
    fn randef() -> Self where Self: Sized;
}

impl RanDef for u8 {
    fn randef() -> Self { Rand::rand_u8() }
}
impl RanDef for u16 {
    fn randef() -> Self { Rand::rand_u16() }
}
impl RanDef for u32 {
    fn randef() -> Self { Rand::rand_u32() }
}
impl RanDef for u64 {
    fn randef() -> Self { Rand::rand_u64() }
}
impl RanDef for u128 {
    fn randef() -> Self { Rand::rand_u128() }
}
impl RanDef for B32 {
    fn randef() -> Self {
        let mut a = [0; 32];
        Rand::fill_u8(&mut a);
        Self(a)
    }
}

pub struct Rand;

impl Rand {
    /// Credit: https://rust-lang-nursery.github.io/rust-cookbook/algorithms/randomness.html
    pub fn generate_random_string(len: usize, charset: &str) -> String {
        let charset = charset.as_bytes();
        let mut rng = rand::thread_rng();
        let pass: String = (0..len)
            .map(|_| {
                let idx = rng.gen_range(0..charset.len());
                charset[idx] as char
            })
            .collect();
        pass
    }

    pub fn value<T>() -> T
    where
        rand::distributions::Standard: rand::distributions::Distribution<T>,
    {
        let mut rng = rand::thread_rng();
        rng.gen()
    }

    pub fn in_range<T>(lower: T, upper: T) -> T 
    where
        T: PartialOrd + rand::distributions::uniform::SampleUniform
    {
        let mut rng = rand::thread_rng();
        rng.gen_range(lower..=upper)
    }    

    pub fn rand_u8() -> u8 {
        OsRng.next_u32() as u8
    }
    
    pub fn rand_u16() -> u16 {
        OsRng.next_u32() as u16
    }
    
    pub fn rand_u32() -> u32 {
        OsRng.next_u32()
    }
    
    pub fn rand_u64() -> u64 {
        OsRng.next_u64()
    }
    
    pub fn rand_u128() -> u128 {
        let a = OsRng.next_u64() as u128;
        let b = (OsRng.next_u64() as u128) << 64;
        a | b
    }

    pub fn fill_u8(a: &mut [u8]) {
        thread_rng().fill(&mut a[..]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    #[test]
    fn test_generate_random_string_00() -> Outcome<()> {
        let p1 = Rand::generate_random_string(3, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        let p2 = Rand::generate_random_string(3, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        let p3 = Rand::generate_random_string(3, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
        msg!("{}-{}-{}", p1, p2, p3);
        Ok(())
    }
}
