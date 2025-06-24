use crate::system::System;

//use oxedyne_fe2o3_core::prelude::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SI {
    // dim fields
    time:       i8,
    length:     i8,
    mass:       i8,
    current:    i8,
    temp:       i8,
    amount:     i8,
    luminosity: i8,
    data:       i8,
    base:       bool, // is this a base unit? (i.e. only one dim field has a value of 1, the rest 0)
    base_sym:   &'static str,
}

impl SI {

    const TIME_UNIT: &'static str       =  "s";
    const LENGTH_UNIT: &'static str     =  "m";
    const MASS_UNIT: &'static str       =  "kg";
    const CURRENT_UNIT: &'static str    =  "A";
    const TEMP_UNIT: &'static str       =  "K";
    const AMOUNT_UNIT: &'static str     =  "mol";
    const LUMINOSITY_UNIT: &'static str =  "cd";
    const DATA_UNIT: &'static str       =  "B";

    pub fn meters() -> Self {
        Self {
            length:     1,
            base:       true,
            base_sym:   Self::LENGTH_UNIT,
            ..Default::default()
        }
    }

    pub fn bytes() -> Self {
        Self {
            data:       1,
            base:       true,
            base_sym:   Self::DATA_UNIT,
            ..Default::default()
        }
    }

    pub fn secs() -> Self {
        Self {
            time:       1,
            base:       true,
            base_sym:   Self::TIME_UNIT,
            ..Default::default()
        }
    }

}

impl System for SI {
    fn base_symbol(&self) -> &'static str {
        self.base_sym
    }
}

impl std::fmt::Display for SI {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.time != 0 {
            write!(f, "[{}]^{}", Self::TIME_UNIT, self.time)?;
        }
        if self.length != 0 {
            write!(f, "[{}]^{}", Self::LENGTH_UNIT, self.length)?;
        }
        if self.mass != 0 {
            write!(f, "[{}]^{}", Self::MASS_UNIT, self.mass)?;
        }
        if self.current != 0 {
            write!(f, "[{}]^{}", Self::CURRENT_UNIT, self.current)?;
        }
        if self.temp != 0 {
            write!(f, "[{}]^{}", Self::TEMP_UNIT, self.temp)?;
        }
        if self.amount != 0 {
            write!(f, "[{}]^{}", Self::AMOUNT_UNIT, self.amount)?;
        }
        if self.luminosity != 0 {
            write!(f, "[{}]^{}", Self::LUMINOSITY_UNIT, self.luminosity)?;
        }
        if self.data != 0 {
            write!(f, "[{}]^{}", Self::DATA_UNIT, self.data)?;
        }
        Ok(())
    }
}
