use crate::{
    prelude::*,
    file::JdatMapFile,
};

use oxedyne_fe2o3_core::{
    prelude::*,
};


pub trait Config:
    Clone
    + std::fmt::Debug
    + Default
    + Eq
    + PartialEq
    + FromDatMap
    + ToDatMap
{
    // Required.
    fn check_and_fix(&mut self) -> Outcome<()> {
        Err(err!(
            "Don't forget to implement checks on the configuration.";
        Unimplemented, Configuration))
    }

    // Provided.
    fn dump(self) -> Outcome<()> {
        let dat = Self::to_datmap(self);
        for line in dat.to_lines("    ", true) {
            info!("{}", line);
        }
        Ok(())
    }

}

impl<T: Config> JdatMapFile for T {}
