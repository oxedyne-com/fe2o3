use crate::{
    prelude::*,
    string::{
        dec::DecoderConfig,
        enc::EncoderConfig,
    },
    usr::{
        UsrKind,
        UsrKindId,
        UsrKindCode,
    },
};

use oxedize_fe2o3_core::{
    prelude::*,
    map::MapMut,
};
use oxedize_fe2o3_text::string::Stringer;

use std::{
    fmt,
    fs,
    io::Write,
    path::{
        Path,
        PathBuf,
    },
};


/// `JdatFile` is suitable for more complex `struct`s with manual implementations of `FromDat` and
/// `ToDat`.
pub trait JdatFile: FromDat + ToDat {

    fn load<
        P: AsRef<Path>,
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        path:           P,
        dec_cfg_opt:    Option<DecoderConfig<M1, M2>>,
    )
        -> Outcome<Self> where Self: Sized
    {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(s) => {
                let dat = if let Some(cfg) = dec_cfg_opt {
                    res!(Dat::decode_string_with_config(s, &cfg))
                } else {
                    res!(Dat::decode_string(s))
                };
                Self::from_dat(dat)
            },
            Err(e) => return Err(err!(e, errmsg!(
                "While trying to read file '{}' as a Dat.", path.display(),
            ), IO, File, Read)),
        }
    }

    fn save<
        P: AsRef<Path>,
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        path:           P,
        tab:            &str,
        //print_kinds:    bool,
        enc_cfg_opt:    Option<EncoderConfig<M1, M2>>,
    )
        -> Outcome<()>
    {
        let path = path.as_ref();
        let mut file = res!(fs::File::create(&path));
        let dat = res!(self.to_dat());
        let s = if let Some(cfg) = enc_cfg_opt {
            res!(dat.encode_string_with_config(&cfg))
        } else {
            fmt!("{:?}", dat)
        };
        for mut line in Stringer::new(s).to_lines(tab) {
            line.push_str("\n");
            res!(file.write(line.as_bytes()));
        }
        Ok(())
    }

}

/// `JdatMapFile` is suitable for simpler `struct`s that have derived `FromDatMap` and `ToDatMap`.
pub trait JdatMapFile: FromDatMap + ToDatMap + Clone {

    fn load<P: AsRef<Path>>(path: P) -> Outcome<Self> {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(s) => {
                let dat = res!(Dat::decode_string(s)).normalise();
                if let Dat::Map(map) = dat {
                    let s = res!(Self::from_datmap(map));
                    Ok(s)
                } else {
                    return Err(err!(errmsg!(
                        "Expected a daticle map at '{}', found a {:?}",
                        path.display(), dat.kind(),
                    ), Input, Invalid));
                }
            },
            Err(e) => return Err(err!(e, errmsg!(
                "While trying to read file '{}' as a Dat.", path.display(),
            ), IO, File)),
        }
    }

    fn save<P: AsRef<Path>>(
        &self,
        path:           P,
        tab:            &str,
        print_kinds:    bool,
    )
        -> Outcome<()>
    {
        let path = path.as_ref();
        let mut file = res!(fs::File::create(&path));
        let dat = Self::to_datmap(self.clone());
        for mut line in dat.to_lines(tab, print_kinds) {
            line.push_str("\n");
            res!(file.write(line.as_bytes()));
        }
        Ok(())
    }

}

/// Allows data to be specified either directly or via a file.
#[derive(Debug)]
pub enum LoadableJdat<J: JdatFile> {
    Data(J),
    Path(PathBuf),
}

/// Allows data to be specified either directly or via a file.
#[derive(Debug)]
pub enum LoadableJdatMap<J: JdatMapFile> {
    Data(J),
    Path(PathBuf),
}
