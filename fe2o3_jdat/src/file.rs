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

use oxedyne_fe2o3_core::{
    prelude::*,
    file as core_file,
    map::MapMut,
};
use oxedyne_fe2o3_text::string::Stringer;

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
            Err(e) => return Err(err!(e,
                "While trying to read file '{}' as a Dat.", path.display();
            IO, File, Read)),
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

    /// As [`save`](Self::save), but for a file holding key material: the
    /// write is atomic and the file ends at mode 0600 whatever the caller's
    /// umask, even when it already existed at a more permissive mode.
    /// `save` itself is untouched, so every other caller keeps its current
    /// permissions behaviour.
    fn save_secret<
        P: AsRef<Path>,
        M1: MapMut<UsrKindCode, UsrKind> + Clone + fmt::Debug + Default,
        M2: MapMut<String, UsrKindId> + Clone + fmt::Debug + Default,
    >(
        &self,
        path:           P,
        tab:            &str,
        enc_cfg_opt:    Option<EncoderConfig<M1, M2>>,
    )
        -> Outcome<()>
    {
        let path = path.as_ref();
        let dat = res!(self.to_dat());
        let s = if let Some(cfg) = enc_cfg_opt {
            res!(dat.encode_string_with_config(&cfg))
        } else {
            fmt!("{:?}", dat)
        };
        let mut text = String::new();
        for mut line in Stringer::new(s).to_lines(tab) {
            line.push_str("\n");
            text.push_str(&line);
        }
        res!(core_file::save_secret(path, text.as_bytes()));
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
                    return Err(err!(
                        "Expected a daticle map at '{}', found a {:?}",
                        path.display(), dat.kind();
                    Input, Invalid));
                }
            },
            Err(e) => return Err(err!(e,
                "While trying to read file '{}' as a Dat.", path.display();
            IO, File)),
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


#[cfg(all(test, unix))]
mod tests {
    use super::*;

    use std::{
        os::unix::fs::PermissionsExt,
        sync::atomic::{
            AtomicU64,
            Ordering,
        },
    };

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A minimal `JdatFile` implementor, in the shape of `Wallet` in
    /// `fe2o3_crypto` -- a thin wrapper that hands its `Dat` straight
    /// through -- so `save`/`save_secret` can be exercised here without a
    /// dependency this crate cannot take (`fe2o3_crypto` depends on
    /// `fe2o3_jdat`, not the other way round).
    #[derive(Clone, Debug)]
    struct TestDoc(Dat);

    impl ToDat for TestDoc {
        fn to_dat(&self) -> Outcome<Dat> { Ok(self.0.clone()) }
    }

    impl FromDat for TestDoc {
        fn from_dat(dat: Dat) -> Outcome<Self> { Ok(Self(dat)) }
    }

    impl JdatFile for TestDoc {}

    fn scratch_path(label: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(fmt!(
            "fe2o3_jdat_file_test_{}_{}_{}", std::process::id(), n, label,
        ))
    }

    fn mode_of(path: &Path) -> Outcome<u32> {
        match fs::metadata(path) {
            Ok(m) => Ok(m.permissions().mode() & 0o777),
            Err(e) => Err(err!(e, "Could not stat {:?}.", path; Test, File, IO, Read)),
        }
    }

    /// `save` is the path every existing caller relies on for non-secret
    /// files, e.g. `ServerConfig`. It must keep leaving a file's mode alone,
    /// so an already-permissive config file stays exactly as permissive.
    #[test]
    fn test_ordinary_save_does_not_restrict_an_existing_files_mode() -> Outcome<()> {
        let path = scratch_path("ordinary_save");
        if let Err(e) = fs::write(&path, b"placeholder") {
            return Err(err!(e, "Could not pre-seed {:?}.", path; Test, File, IO, Write));
        }
        if let Err(e) = fs::set_permissions(&path, fs::Permissions::from_mode(0o644)) {
            return Err(err!(e, "Could not set 0644 on {:?}.", path; Test, File, IO));
        }

        let doc = TestDoc(Dat::Str("not a secret".to_string()));
        let save_res = doc.save(&path, "  ", Some(EncoderConfig::<(), ()>::default()));

        let mode = mode_of(&path);
        let _ = fs::remove_file(&path);
        res!(save_res);
        if res!(mode) != 0o644 {
            return Err(err!(
                "The ordinary JdatFile::save changed {:?}'s mode away from 0644; \
                a non-secret save must leave permissions alone.", path;
                Test, Mismatch));
        }
        Ok(())
    }

    /// `save_secret` is the path key material -- the wallet, TLS and DKIM
    /// keys -- must go through instead: the file must end at 0600 even
    /// though nothing here asked for a restrictive mode explicitly.
    #[test]
    fn test_save_secret_restricts_a_new_files_mode() -> Outcome<()> {
        let path = scratch_path("save_secret");
        let doc = TestDoc(Dat::Str("top secret".to_string()));
        let save_res = doc.save_secret(&path, "  ", Some(EncoderConfig::<(), ()>::default()));

        let mode = mode_of(&path);
        let _ = fs::remove_file(&path);
        res!(save_res);
        if res!(mode) != 0o600 {
            return Err(err!(
                "save_secret left {:?} at a mode other than 0600.", path;
                Test, Mismatch));
        }
        Ok(())
    }
}
