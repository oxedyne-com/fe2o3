use oxedyne_fe2o3_core::prelude::*;

use crate::{
    scale::{
        Mag,
        Scale,
        ScaleBasis,
    },
    si::SI,
};

pub trait System: Clone + std::fmt::Display + PartialEq {
    fn base_symbol(&self) -> &'static str {
        ""
    }
}

#[derive(Clone, Debug)]
pub struct Units<D: System> {
    pub mag: Mag,
    pub dim: D,
}

//impl<D> fmt::Display for Units<D>
//    where D: Clone + fmt::Display + PartialEq
//{
//    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//        match self.system {
//            System::SI => {
//                match write!(f, "f"),
//            },
//
//        }
//    }
//}

impl<D: System> Units<D> {

    pub fn new(mag: Mag, dim: D) -> Self { 
        Self{
            mag: mag,
            dim: dim,
        }
    }

    pub fn bytes(val: f64, sf: u8) -> Outcome<Units::<SI>> {
        Ok(Units::new(
            Mag::new(val, Scale::One(ScaleBasis::Binary), sf)?,
            SI::bytes(),
        ))
    }

    pub fn mag(&self) -> &Mag {
        &self.mag
    }

    pub fn dim(&self) -> &D {
        &self.dim
    }

    pub fn val(&self) -> f64 {
        self.mag.val
    }

    pub fn prefix(&self) -> &'static str {
        self.mag.prefix()
    }

    /// Returns "" if the dimension does not represent a fundamental unit.
    pub fn symbol(&self) -> &'static str {
        self.dim.base_symbol()
    }

    pub fn unitise(&self) -> Self {
        Self::new(
            self.mag.unitise(),
            self.dim.clone(),
        )
    }

    pub fn humanise(&self) -> Self {
        Self::new(
            self.mag.humanise(),
            self.dim.clone(),
        )
    }

}

impl<D: System> PartialEq for Units<D> {

    fn eq(&self, other: &Self) -> bool {
        if self.dim.eq(&other.dim) {
            return self.mag.eq(&other.mag);
        }
        false
    }
}
