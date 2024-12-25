use oxedize_fe2o3_core::prelude::*;
use oxedize_fe2o3_num::float;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScaleBasis {
    Decimal,
    Binary
}

impl ScaleBasis {

    // Decimal floating point form uses base 10 (e.g. 1.0 x 10^3), so no factor required
    pub const DEC_LOG_FACTOR: f64 = 1.0;
    pub const DEC_BASE: f64 = 10.0;
    // For binary decimal form using exponents which are a factor of three for engineering
    // notation, on the other hand, we can think of the base as being X, e.g. 1.0 x X^3 = 1024.
    // This gives X = 10^(log10(1024)/3) = 10.07936.. Next we want to be able to numbers into this
    // new basis.  If a = X^b, b = logX(a) [1] and log10(a) = log10(X^b) = b*log10(X) [2].
    // Substituting the b in [1] into [2] we get logX(a) = log10(a)/log10(X).  The factor below is
    // log10(X).  Using the value of X above, this can be simplified to log10(1024)/3, allowing us
    // to use the existing log10 functionality.  Exponents calculated this way will differ by +/-3
    // if the numbers they form are in the ratio 1024.  For example
    // 1024 = 1.0 x X^3, 1024^2 = 1.0 x X^6, etc.
    pub const BIN_LOG_FACTOR: f64 = 1.0034333188799373;
    pub const BIN_BASE: f64 = 10.0793683991589853;

    pub fn log_factor(&self) -> f64 {
        match self {
            Self::Decimal   => Self::DEC_LOG_FACTOR,
            Self::Binary    => Self::BIN_LOG_FACTOR,
        }
    }

    pub fn base(&self) -> f64 {
        match self {
            Self::Decimal   => Self::DEC_BASE,
            Self::Binary    => Self::BIN_BASE,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Scale {
    // decimal
    Atto,
    Femto,
    Pico,
    Nano,
    Micro,
    Milli,
    Centi,
    Deci,
    One(ScaleBasis),
    Deca,
    Hecto,
    Kilo,
    Mega,
    Giga,
    Tera,
    Peta,
    Exa,
    // binary
    Kibi,
    Mebi,
    Gibi,
    Tebi,
    Pebi,
    Exbi,
}

impl Scale {

    // decimal
    const ATTO_SCALE: u64   = 1_000_000_000_000_000_000;
    const FEMTO_SCALE: u64  = 1_000_000_000_000_000;    
    const PICO_SCALE: u64   = 1_000_000_000_000;        
    const NANO_SCALE: u64   = 1_000_000_000;            
    const MICRO_SCALE: u64  = 1_000_000;                
    const MILLI_SCALE: u64  = 1_000;                    
    const CENTI_SCALE: u64  = 100;                    
    const DECI_SCALE: u64   = 10;                    
    const ONE_SCALE: u64    = 1;
    const DECA_SCALE: u64   = 10;                    
    const HECTO_SCALE: u64  = 100;                    
    const KILO_SCALE: u64   = 1_000;                    
    const MEGA_SCALE: u64   = 1_000_000;                
    const GIGA_SCALE: u64   = 1_000_000_000;            
    const TERA_SCALE: u64   = 1_000_000_000_000;        
    const PETA_SCALE: u64   = 1_000_000_000_000_000;    
    const EXA_SCALE: u64    = 1_000_000_000_000_000_000;
    // binary                18_446_744_073_709_551_615 u64 max
    const KIBI_SCALE: u64   = 1_024;                    
    const MEBI_SCALE: u64   = 1_048_576;                
    const GIBI_SCALE: u64   = 1_073_741_824;            
    const TEBI_SCALE: u64   = 1_099_511_627_776;        
    const PEBI_SCALE: u64   = 1_125_899_906_842_624;    
    const EXBI_SCALE: u64   = 1_152_921_504_606_846_976;

    // decimal
    const ATTO_DEC_EXP: f64     = -18.0;
    const FEMTO_DEC_EXP: f64    = -15.0;    
    const PICO_DEC_EXP: f64     = -12.0;        
    const NANO_DEC_EXP: f64     = -9.0;            
    const MICRO_DEC_EXP: f64    = -6.0;                
    const MILLI_DEC_EXP: f64    = -3.0;                    
    const CENTI_DEC_EXP: f64    = -2.0;                    
    const DECI_DEC_EXP: f64     = -1.0;                    
    const ONE_DEC_EXP: f64      = 0.0;
    const DECA_DEC_EXP: f64     = 1.0;                    
    const HECTO_DEC_EXP: f64    = 2.0;                    
    const KILO_DEC_EXP: f64     = 3.0;                    
    const MEGA_DEC_EXP: f64     = 6.0;                
    const GIGA_DEC_EXP: f64     = 9.0;            
    const TERA_DEC_EXP: f64     = 12.0;        
    const PETA_DEC_EXP: f64     = 15.0;    
    const EXA_DEC_EXP: f64      = 18.0;
    // binary
    const KIBI_DEC_EXP: f64     = 3.01029995664;                   
    const MEBI_DEC_EXP: f64     = 6.02059991328;
    const GIBI_DEC_EXP: f64     = 9.03089986992;
    const TEBI_DEC_EXP: f64     = 12.0411998266;
    const PEBI_DEC_EXP: f64     = 15.0514997832;
    const EXBI_DEC_EXP: f64     = 18.0617997398;

    // decimal
    const ATTO_PREFIX: &'static str  =  "a";
    const FEMTO_PREFIX: &'static str =  "f";      
    const PICO_PREFIX: &'static str  =  "p";      
    const NANO_PREFIX: &'static str  =  "n";      
    const MICRO_PREFIX: &'static str =  "\u{00b5}";
    const MILLI_PREFIX: &'static str =  "m";      
    const CENTI_PREFIX: &'static str =  "c";      
    const DECI_PREFIX: &'static str  =  "d";      
    const ONE_PREFIX: &'static str   =  "";     
    const DECA_PREFIX: &'static str  =  "da";     
    const HECTO_PREFIX: &'static str =  "h";      
    const KILO_PREFIX: &'static str  =  "k";      
    const MEGA_PREFIX: &'static str  =  "M";      
    const GIGA_PREFIX: &'static str  =  "G";      
    const TERA_PREFIX: &'static str  =  "T";      
    const PETA_PREFIX: &'static str  =  "P";      
    const EXA_PREFIX: &'static str   =  "E";            
    // binary                                     
    const KIBI_PREFIX: &'static str  =  "Ki";     
    const MEBI_PREFIX: &'static str  =  "Mi";     
    const GIBI_PREFIX: &'static str  =  "Gi";     
    const TEBI_PREFIX: &'static str  =  "Ti";     
    const PEBI_PREFIX: &'static str  =  "Pi";     
    const EXBI_PREFIX: &'static str  =  "Ei";

    pub fn basis(&self) -> ScaleBasis {
        match self {
            Self::One(b) => b.clone(),
            Self::Kibi |
            Self::Mebi |
            Self::Gibi |
            Self::Tebi |
            Self::Pebi |
            Self::Exbi => ScaleBasis::Binary,
            _ => ScaleBasis::Decimal,
        }
    }

    pub fn as_u64(&self) -> u64 {
        match self {
            // decimal
            Self::Atto      => Self::ATTO_SCALE, 
            Self::Femto     => Self::FEMTO_SCALE,
            Self::Pico      => Self::PICO_SCALE, 
            Self::Nano      => Self::NANO_SCALE, 
            Self::Micro     => Self::MICRO_SCALE,
            Self::Milli     => Self::MILLI_SCALE,
            Self::Centi     => Self::CENTI_SCALE,
            Self::Deci      => Self::DECI_SCALE, 
            Self::One(_)    => Self::ONE_SCALE,
            Self::Deca      => Self::DECA_SCALE, 
            Self::Hecto     => Self::HECTO_SCALE,
            Self::Kilo      => Self::KILO_SCALE, 
            Self::Mega      => Self::MEGA_SCALE, 
            Self::Giga      => Self::GIGA_SCALE, 
            Self::Tera      => Self::TERA_SCALE, 
            Self::Peta      => Self::PETA_SCALE, 
            Self::Exa       => Self::EXA_SCALE,
            // binary
            Self::Kibi      => Self::KIBI_SCALE, 
            Self::Mebi      => Self::MEBI_SCALE, 
            Self::Gibi      => Self::GIBI_SCALE, 
            Self::Tebi      => Self::TEBI_SCALE, 
            Self::Pebi      => Self::PEBI_SCALE, 
            Self::Exbi      => Self::EXBI_SCALE, 
            //
            //_ => unimplemented!(),
        }
    }

    pub fn dec_exp(&self) -> f64 {
        match self {
            // decimal
            Self::Atto      => Self::ATTO_DEC_EXP, 
            Self::Femto     => Self::FEMTO_DEC_EXP,
            Self::Pico      => Self::PICO_DEC_EXP, 
            Self::Nano      => Self::NANO_DEC_EXP, 
            Self::Micro     => Self::MICRO_DEC_EXP,
            Self::Milli     => Self::MILLI_DEC_EXP,
            Self::Centi     => Self::CENTI_DEC_EXP,
            Self::Deci      => Self::DECI_DEC_EXP, 
            Self::One(_)    => Self::ONE_DEC_EXP,
            Self::Deca      => Self::DECA_DEC_EXP, 
            Self::Hecto     => Self::HECTO_DEC_EXP,
            Self::Kilo      => Self::KILO_DEC_EXP, 
            Self::Mega      => Self::MEGA_DEC_EXP, 
            Self::Giga      => Self::GIGA_DEC_EXP, 
            Self::Tera      => Self::TERA_DEC_EXP, 
            Self::Peta      => Self::PETA_DEC_EXP, 
            Self::Exa       => Self::EXA_DEC_EXP,
            // binary
            Self::Kibi      => Self::KIBI_DEC_EXP, 
            Self::Mebi      => Self::MEBI_DEC_EXP, 
            Self::Gibi      => Self::GIBI_DEC_EXP, 
            Self::Tebi      => Self::TEBI_DEC_EXP, 
            Self::Pebi      => Self::PEBI_DEC_EXP, 
            Self::Exbi      => Self::EXBI_DEC_EXP, 
            //
            //_ => unimplemented!(),
        }
    }

    pub fn dec_exp_lookup(&self, exp: i32) -> Self {
        match self.basis() {
            ScaleBasis::Decimal => {
                match exp {
                    -18 => Self::Atto,
                    -15 => Self::Femto, 
                    -12 => Self::Pico, 
                    -9  => Self::Nano, 
                    -6  => Self::Micro,
                    -3  => Self::Milli,
                    -2  => Self::Centi,
                    -1  => Self::Deci, 
                    0   => Self::One(ScaleBasis::Decimal), 
                    1   => Self::Deca, 
                    2   => Self::Hecto,
                    3   => Self::Kilo, 
                    6   => Self::Mega, 
                    9   => Self::Giga, 
                    12  => Self::Tera, 
                    15  => Self::Peta, 
                    18  => Self::Exa, 
                    _ => unimplemented!("{:?} decimal exponent lookup value {}", self, exp),
                }
            },
            ScaleBasis::Binary => {
                match exp {
                    0   => Self::One(ScaleBasis::Binary), 
                    3   => Self::Kibi,
                    6   => Self::Mebi,
                    9   => Self::Gibi,
                    12  => Self::Tebi,
                    15  => Self::Pebi,
                    18  => Self::Exbi,
                    _ => unimplemented!("{:?} decimal exponent lookup value {}", self, exp),
                }
            },
        }
    }

    pub fn prefix(&self) -> &'static str {
        match self {
            // decimal
            Self::Atto   => Self::ATTO_PREFIX,
            Self::Femto  => Self::FEMTO_PREFIX,
            Self::Pico   => Self::PICO_PREFIX,
            Self::Nano   => Self::NANO_PREFIX,
            Self::Micro  => Self::MICRO_PREFIX,
            Self::Milli  => Self::MILLI_PREFIX,
            Self::Centi  => Self::CENTI_PREFIX,
            Self::Deci   => Self::DECI_PREFIX,
            Self::One(_) => Self::ONE_PREFIX,
            Self::Deca   => Self::DECA_PREFIX, 
            Self::Hecto  => Self::HECTO_PREFIX,
            Self::Kilo   => Self::KILO_PREFIX,
            Self::Mega   => Self::MEGA_PREFIX,
            Self::Giga   => Self::GIGA_PREFIX,
            Self::Tera   => Self::TERA_PREFIX,
            Self::Peta   => Self::PETA_PREFIX,
            Self::Exa    => Self::EXA_PREFIX, 
            // binary                
            Self::Kibi   => Self::KIBI_PREFIX,
            Self::Mebi   => Self::MEBI_PREFIX,
            Self::Gibi   => Self::GIBI_PREFIX,
            Self::Tebi   => Self::TEBI_PREFIX,
            Self::Pebi   => Self::PEBI_PREFIX,
            Self::Exbi   => Self::EXBI_PREFIX,
            //_ => unimplemented!(),   
        }
    }
}

#[derive(Clone, Debug)]
pub struct Mag {
    pub val:    f64,
    pub scale:  Scale,
    pub sf:     u8, // significant figures
    pub zero:   bool,
}

impl Mag {

    pub fn new(val: f64, scale: Scale, sf: u8) -> Outcome<Self> {
        if sf == 0 {
            return Err(err!(errmsg!(
                "Number of significant figures must be > 0.",
            ), ErrTag::Input, ErrTag::Invalid));
        }
        Ok(Self {
            val:    val,
            scale:  scale,   
            sf:     sf,
            zero:   if val.abs() < std::f64::MIN_POSITIVE { true } else { false },
        })
    }

    // decimal
    pub fn atto(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Atto , sf) } 
    pub fn femto(val: f64, sf: u8)  -> Outcome<Self> { Self::new(val, Scale::Femto, sf) } 
    pub fn pico(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Pico , sf) } 
    pub fn nano(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Nano , sf) } 
    pub fn micro(val: f64, sf: u8)  -> Outcome<Self> { Self::new(val, Scale::Micro, sf) } 
    pub fn milli(val: f64, sf: u8)  -> Outcome<Self> { Self::new(val, Scale::Milli, sf) } 
    pub fn centi(val: f64, sf: u8)  -> Outcome<Self> { Self::new(val, Scale::Centi, sf) } 
    pub fn deci(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Deci , sf) } 
    pub fn deca(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Deca , sf) } 
    pub fn hecto(val: f64, sf: u8)  -> Outcome<Self> { Self::new(val, Scale::Hecto, sf) } 
    pub fn kilo(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Kilo , sf) } 
    pub fn mega(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Mega , sf) } 
    pub fn giga(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Giga , sf) } 
    pub fn tera(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Tera , sf) } 
    pub fn peta(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Peta , sf) } 
    pub fn exa(val: f64, sf: u8)    -> Outcome<Self> { Self::new(val, Scale::Exa  , sf) }
    
    // binary
    pub fn kibi(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Kibi , sf) } 
    pub fn mebi(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Mebi , sf) } 
    pub fn gibi(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Gibi , sf) } 
    pub fn tebi(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Tebi , sf) } 
    pub fn pebi(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Pebi , sf) } 
    pub fn exbi(val: f64, sf: u8)   -> Outcome<Self> { Self::new(val, Scale::Exbi , sf) } 

    pub fn one_decimal(val: f64, sf: u8) -> Outcome<Self> { Self::new(val, Scale::One(ScaleBasis::Decimal), sf) }
    pub fn one_binary(val: f64, sf: u8) -> Outcome<Self> { Self::new(val, Scale::One(ScaleBasis::Binary), sf)
    }

    pub fn basis(&self) -> ScaleBasis {
        self.scale.basis()
    }

    pub fn prefix(&self) -> &'static str {
        self.scale.prefix()
    }

    /// Adjust value to make the scale `Scale::One`.
    pub fn unitise(&self) -> Self {
        Self::new(
            if self.zero {
                0.0f64
            } else {
                self.val * (10u64 as f64).powf(self.scale.dec_exp())
            },
            Scale::One(self.basis()),
            self.sf,
        ).unwrap()
    }

    /// Return a value in the range (-10, -1) or (1, 10) rounded to the required number of
    /// significant figures, and the decimal exponent.
    pub fn normalise(&self) -> (f64, i32) {
        let exp = self.val.log10().floor() as i32;
        let mut val = self.val/(10.0f64.powi(exp));
        val = (val * (10.0f64.powi((self.sf - 1) as i32))).round();
        val = val / (10.0f64.powi((self.sf - 1) as i32));
        (val, exp)
    }

    /// Implements engineering notation (i.e that is, use of standard decimal exponent), and rounds
    /// the value according to the required number of significant figures.
    pub fn humanise(&self) -> Self {
        let n = self.unitise();
        if self.zero {
            return n;
        }
        let expbase = n.val.log10() / self.basis().log_factor();
        let engexp = (3.0 * (expbase / 3.0).floor()) as i32;
        let newscale = self.scale.dec_exp_lookup(engexp);
        let newval = n.val / (self.basis().base().powi(engexp));
        Self::new(
            float::round_to_sf(newval, self.sf),
            newscale,
            self.sf,
        ).unwrap()
    }

}

impl PartialEq for Mag {
    fn eq(&self, other: &Self) -> bool {
        let lhs = self.val/(10.0f64.powi(self.val.log10().floor() as i32));
        let rhs = other.val/(10.0f64.powi(other.val.log10().floor() as i32));
        if (lhs * (10.0f64.powi((self.sf - 1) as i32))).round() ==
            (rhs * (10.0f64.powi((self.sf - 1) as i32))).round() {
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_simple_mag_one_01() -> Outcome<()> {
        let a = res!(Mag::one_decimal(123456.0, 3));
        let b = res!(Mag::kilo(123.0, 3));
        assert_eq!(a.humanise(), b);
        Ok(())
    }

    #[test]
    fn test_simple_mag_dec_01() -> Outcome<()> {
        let a = res!(Mag::mega(1234.0, 4));
        let b = res!(Mag::giga(1.234, 4));
        assert_eq!(a, b);
        Ok(())
    }

    #[test]
    fn test_simple_mag_dec_02() -> Outcome<()> {
        let a = res!(Mag::mega(1234.0, 4));
        let h = a.humanise();
        let b = res!(Mag::giga(1.234, 4));
        assert_eq!(b, h);
        Ok(())
    }

    #[test]
    fn test_simple_mag_dec_03() -> Outcome<()> {
        let a = res!(Mag::micro(1234.0, 4));
        let b = res!(Mag::milli(1.234, 4));
        assert_eq!(a, b);
        Ok(())
    }

    #[test]
    fn test_simple_mag_bin_01() -> Outcome<()> {
        let a = res!(Mag::kibi(1.0, 4));
        let b = res!(Mag::one_binary(1024.0, 4));
        assert_eq!(a.unitise(), b);
        assert_eq!(a, b.humanise());
        Ok(())
    }

    #[test]
    fn test_simple_units_01() -> Outcome<()> {
        let a = Units::new(res!(Mag::one_binary(1024.0, 4)), SI::bytes());
        let b = a.humanise();
        assert_eq!(a, a.unitise());
        assert_eq!(a, b.unitise());
        Ok(())
    }

    #[test]
    fn test_simple_units_02() -> Outcome<()> {
        let a = res!(Units::<SI>::bytes(1024.0, 4));
        let b = a.humanise();
        assert_eq!(a, a.unitise());
        assert_eq!(a, b.unitise());
        Ok(())
    }
}
