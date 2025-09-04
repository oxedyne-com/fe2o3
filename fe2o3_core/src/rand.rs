use crate::{
    prelude::*,
    byte::B32,
};

use std::cmp::PartialOrd;

use rand::{
    thread_rng,
    Rng,
};
use rand_core::{
    OsRng,
    RngCore,
};


/// Sampling method for range generation.
#[derive(Clone, Copy, Debug)]
pub enum SamplingMethod {
    Uniform,
    GaussianClampedDerived,
    GaussianClampedExplicit { mean: f32, stdev: f32 },
}

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
    pub fn generate_random_string(
        len:	usize,
        charset:	&str,
    )
        -> String
    {
        let charset = charset.as_bytes();
        let mut rng = thread_rng();
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
        let mut rng = thread_rng();
        rng.gen()
    }

    pub fn in_range<T>(
        lower:	T,
        upper:	T,
    )
        -> T 
    where
        T: PartialOrd + rand::distributions::uniform::SampleUniform
    {
        let mut rng = thread_rng();
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
    
    pub fn normal(
        mean:	f32,
        stdev:	f32,
    )
        -> f32
    {
        let mut rng = thread_rng();
        let u1: f32 = loop {
            let val = rng.gen::<f32>();
            if val > 0.0 {
                break val;
            }
        };
        let u2: f32 = rng.gen();
        let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos();
        mean + stdev * z0
    }
    
    pub fn normal_f64(
        mean:	f64,
        stdev:	f64,
    )
        -> f64
    {
        let mut rng = thread_rng();
        let u1: f64 = loop {
            let val = rng.gen::<f64>();
            if val > 0.0 {
                break val;
            }
        };
        let u2: f64 = rng.gen();
        let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        mean + stdev * z0
    }

    // Sampling methods for all numeric types.
    
    pub fn sample_f32(
        min:	f32,
        max:	f32,
        method:	SamplingMethod,
    )
        -> Outcome<f32>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        if !min.is_finite() || !max.is_finite() {
            return Err(err!("Range bounds must be finite: [{}, {}]", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => {
                let scale = max - min;
                min + scale * Self::value::<f32>()
            }
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min + max) / 2.0;
                let stdev = (max - min) / 6.0;
                let sample = Self::normal(mean, stdev);
                sample.max(min).min(max)
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal(mean, stdev);
                sample.max(min).min(max)
            }
        })
    }
    
    pub fn sample_f64(
        min:	f64,
        max:	f64,
        method:	SamplingMethod,
    )
        -> Outcome<f64>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        if !min.is_finite() || !max.is_finite() {
            return Err(err!("Range bounds must be finite: [{}, {}]", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => {
                let scale = max - min;
                min + scale * Self::value::<f64>()
            }
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min + max) / 2.0;
                let stdev = (max - min) / 6.0;
                let sample = Self::normal_f64(mean, stdev);
                sample.max(min).min(max)
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64);
                sample.max(min).min(max)
            }
        })
    }

    pub fn sample_u8(
        min:	u8,
        max:	u8,
        method:	SamplingMethod,
    )
        -> Outcome<u8>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f32 + max as f32) / 2.0;
                let stdev = (max as f32 - min as f32) / 6.0;	// 6σ covers ~99.7%.
                let sample = Self::normal(mean, stdev).round();
                sample.max(min as f32).min(max as f32) as u8
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal(mean, stdev).round();
                sample.max(min as f32).min(max as f32) as u8
            }
        })
    }

    pub fn sample_u16(
        min:	u16,
        max:	u16,
        method:	SamplingMethod,
    )
        -> Outcome<u16>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f32 + max as f32) / 2.0;
                let stdev = (max as f32 - min as f32) / 6.0;
                let sample = Self::normal(mean, stdev).round();
                sample.max(min as f32).min(max as f32) as u16
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal(mean, stdev).round();
                sample.max(min as f32).min(max as f32) as u16
            }
        })
    }

    pub fn sample_u32(
        min:	u32,
        max:	u32,
        method:	SamplingMethod,
    )
        -> Outcome<u32>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f64 + max as f64) / 2.0;	// Use f64 for precision.
                let stdev = (max as f64 - min as f64) / 6.0;
                let sample = Self::normal_f64(mean, stdev).round();
                sample.max(min as f64).min(max as f64) as u32
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64).round();
                sample.max(min as f64).min(max as f64) as u32
            }
        })
    }

    pub fn sample_u64(
        min:	u64,
        max:	u64,
        method:	SamplingMethod,
    )
        -> Outcome<u64>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f64 + max as f64) / 2.0;
                let stdev = (max as f64 - min as f64) / 6.0;
                let sample = Self::normal_f64(mean, stdev).round();
                sample.max(min as f64).min(max as f64) as u64
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64).round();
                sample.max(min as f64).min(max as f64) as u64
            }
        })
    }

    pub fn sample_u128(
        min:	u128,
        max:	u128,
        method:	SamplingMethod,
    )
        -> Outcome<u128>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                // Careful handling for large values.
                let min_f64 = min as f64;
                let max_f64 = max as f64;
                let mean = (min_f64 + max_f64) / 2.0;
                let stdev = (max_f64 - min_f64) / 6.0;
                let sample = Self::normal_f64(mean, stdev).round();
                
                if sample < 0.0 {
                    min
                } else if sample > u128::MAX as f64 {
                    max
                } else {
                    sample.max(min_f64).min(max_f64) as u128
                }
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64).round();
                if sample < 0.0 {
                    min
                } else if sample > u128::MAX as f64 {
                    max
                } else {
                    sample.max(min as f64).min(max as f64) as u128
                }
            }
        })
    }

    pub fn sample_usize(
        min:	usize,
        max:	usize,
        method:	SamplingMethod,
    )
        -> Outcome<usize>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f64 + max as f64) / 2.0;
                let stdev = (max as f64 - min as f64) / 6.0;
                let sample = Self::normal_f64(mean, stdev).round();
                sample.max(min as f64).min(max as f64) as usize
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64).round();
                sample.max(min as f64).min(max as f64) as usize
            }
        })
    }

    pub fn sample_i8(
        min:	i8,
        max:	i8,
        method:	SamplingMethod,
    )
        -> Outcome<i8>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f32 + max as f32) / 2.0;
                let stdev = (max as f32 - min as f32) / 6.0;
                let sample = Self::normal(mean, stdev).round();
                sample.max(min as f32).min(max as f32) as i8
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal(mean, stdev).round();
                sample.max(min as f32).min(max as f32) as i8
            }
        })
    }

    pub fn sample_i16(
        min:	i16,
        max:	i16,
        method:	SamplingMethod,
    )
        -> Outcome<i16>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f32 + max as f32) / 2.0;
                let stdev = (max as f32 - min as f32) / 6.0;
                let sample = Self::normal(mean, stdev).round();
                sample.max(min as f32).min(max as f32) as i16
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal(mean, stdev).round();
                sample.max(min as f32).min(max as f32) as i16
            }
        })
    }

    pub fn sample_i32(
        min:	i32,
        max:	i32,
        method:	SamplingMethod,
    )
        -> Outcome<i32>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f64 + max as f64) / 2.0;
                let stdev = (max as f64 - min as f64) / 6.0;
                let sample = Self::normal_f64(mean, stdev).round();
                sample.max(min as f64).min(max as f64) as i32
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64).round();
                sample.max(min as f64).min(max as f64) as i32
            }
        })
    }

    pub fn sample_i64(
        min:	i64,
        max:	i64,
        method:	SamplingMethod,
    )
        -> Outcome<i64>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f64 + max as f64) / 2.0;
                let stdev = (max as f64 - min as f64) / 6.0;
                let sample = Self::normal_f64(mean, stdev).round();
                sample.max(min as f64).min(max as f64) as i64
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64).round();
                sample.max(min as f64).min(max as f64) as i64
            }
        })
    }

    pub fn sample_i128(
        min:	i128,
        max:	i128,
        method:	SamplingMethod,
    )
        -> Outcome<i128>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let min_f64 = min as f64;
                let max_f64 = max as f64;
                let mean = (min_f64 + max_f64) / 2.0;
                let stdev = (max_f64 - min_f64) / 6.0;
                let sample = Self::normal_f64(mean, stdev).round();
                
                if sample < i128::MIN as f64 {
                    min
                } else if sample > i128::MAX as f64 {
                    max
                } else {
                    sample.max(min_f64).min(max_f64) as i128
                }
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64).round();
                if sample < i128::MIN as f64 {
                    min
                } else if sample > i128::MAX as f64 {
                    max
                } else {
                    sample.max(min as f64).min(max as f64) as i128
                }
            }
        })
    }

    pub fn sample_isize(
        min:	isize,
        max:	isize,
        method:	SamplingMethod,
    )
        -> Outcome<isize>
    {
        if min > max {
            return Err(err!("Invalid range: {} > {}", min, max; Invalid, Range));
        }
        
        Ok(match method {
            SamplingMethod::Uniform => Self::in_range(min, max),
            SamplingMethod::GaussianClampedDerived => {
                let mean = (min as f64 + max as f64) / 2.0;
                let stdev = (max as f64 - min as f64) / 6.0;
                let sample = Self::normal_f64(mean, stdev).round();
                sample.max(min as f64).min(max as f64) as isize
            }
            SamplingMethod::GaussianClampedExplicit { mean, stdev } => {
                let sample = Self::normal_f64(mean as f64, stdev as f64).round();
                sample.max(min as f64).min(max as f64) as isize
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sample_ranges() -> Outcome<()> {
        // Test u32.
        for _ in 0..100 {
            let val = res!(Rand::sample_u32(10, 100, SamplingMethod::Uniform));
            req!(val >= 10 && val <= 100, true);
        }
        
        // Test i32 negative range.
        for _ in 0..100 {
            let val = res!(Rand::sample_i32(-50, 50, SamplingMethod::GaussianClampedDerived));
            req!(val >= -50 && val <= 50, true);
        }
        
        Ok(())
    }
    
    #[test]
    fn test_normal_distribution() -> Outcome<()> {
        let mean = 100.0;
        let stdev = 15.0;
        let mut samples = Vec::new();
        
        for _ in 0..1000 {
            samples.push(Rand::normal(mean, stdev));
        }
        
        let sample_mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let variance = samples.iter()
            .map(|x| (x - sample_mean).powi(2))
            .sum::<f32>() / (samples.len() - 1) as f32;
        let sample_stdev = variance.sqrt();
        
        msg!("Expected mean: {}, Sample mean: {}", mean, sample_mean);
        msg!("Expected stdev: {}, Sample stdev: {}", stdev, sample_stdev);
        
        assert!((sample_mean - mean).abs() < 1.0, "Sample mean too far from expected");
        assert!((sample_stdev - stdev).abs() < 1.0, "Sample stdev too far from expected");
        
        Ok(())
    }
}
