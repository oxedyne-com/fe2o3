use oxedize_fe2o3_core::{
    prelude::*,
    file::PathState,
    path::{
        NormalPath,
        NormPathBuf,
    },
};
use oxedize_fe2o3_jdat::{
    prelude::*,
    cfg::Config,
};

use std::{
    collections::{
        BTreeMap,
        //HashMap,
    },
    path::{
        Path,
        PathBuf,
    },
};


#[derive(Clone, Debug, Eq, PartialEq, FromDatMap, ToDatMap)]
pub struct DevConfig {
    pub src_path_rel:           String,
    pub js_bundles_rel:         DaticleMap,
    pub css_source_dir_rel:     String,
    pub css_bundle_rel:         String,
    pub js_import_aliases_rel:  DaticleMap,
}

impl Config for DevConfig {}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            src_path_rel:           fmt!("./www/src"),
            js_bundles_rel:         mapdat!{
                "./www/src/js/pages/main/index.mjs" => "./www/public/bundles/js/main.bundle.js",
                "./www/src/js/pages/admin/index.mjs" => "./www/public/bundles/js/admin.bundle.js",
            }.get_map().unwrap_or(DaticleMap::new()),
            css_source_dir_rel:     fmt!("./www/src/styles"),
            css_bundle_rel:         fmt!("./www/public/styles.css"),
            js_import_aliases_rel:  mapdat!{
                "@utils" => "./www/src/js/utils",
                "@components" => "./www/src/js/components",
            }.get_map().unwrap_or(DaticleMap::new()),
        }
    }
}

impl DevConfig {

    pub fn validate(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<()>
    {
        // Ensure that the source directory exists.
        res!(PathState::DirMustExist.validate(
            root,
            &self.src_path_rel,
        ));

        let _ = res!(self.get_js_bundles_map(root));
        let _ = res!(self.get_css_paths(root));
        let _ = res!(self.get_js_import_aliases(root));

        Ok(())
    }

    pub fn get_js_bundles_map(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<Vec<(PathBuf, PathBuf)>>
    {
        let mut result = Vec::new(); 
        for (entry_dat, bundle_dat) in &self.js_bundles_rel {
            
            // Javascript/Typescript entry point file.
            let entry_str = try_extract_dat!(entry_dat, Str);
            let entry = Path::new(&entry_str).normalise();
            if entry.escapes() {
                return Err(err!(errmsg!(
                    "DevConfig: javascipt/typescript entry point {} escapes the directory {:?}.",
                    entry_str, root,
                ), Invalid, Input, Path));
            }
            let entry = root.clone().join(entry).normalise().absolute();
            res!(PathState::FileMustExist.validate(
                root,
                entry_str,
            ));

            // Target javascript bundle file.
            let bundle_str = try_extract_dat!(bundle_dat, Str);
            // Ensure that the file into which the javascript will be bundled stays within the root
            // directory.
            let bundle = Path::new(&bundle_str).normalise();
            if bundle.escapes() {
                return Err(err!(errmsg!(
                    "DevConfig: javascript bundle entry {} maps to a bundle path {} \
                    that escapes the directory {:?}.",
                    entry_str, bundle_str, root,
                ), Invalid, Input, Path));
            }
            let bundle = root.clone().join(bundle).normalise().absolute();
            result.push((entry.as_pathbuf(), bundle.as_pathbuf()));
        }
        Ok(result)
    }

    pub fn get_js_import_aliases(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<Vec<(String, PathBuf)>>
    {
        let mut result = Vec::new(); 
        for (alias_dat, path_dat) in &self.js_import_aliases_rel {
            let alias = try_extract_dat!(alias_dat, Str).clone();
            let path_str = try_extract_dat!(path_dat, Str);
            // Ensure that the file into which the javascript will be bundled stays within the root
            // directory.
            let path = Path::new(&path_str).normalise();
            if path.escapes() {
                return Err(err!(errmsg!(
                    "DevConfig: javascript import alias entry {} maps to a path {} \
                    that escapes the directory {:?}.",
                    alias, path_str, root,
                ), Invalid, Input, Path));
            }
            let path = root.clone().join(path).normalise().absolute();
            result.push((alias, path.as_pathbuf()));
        }
        Ok(result)
    }

    pub fn get_css_paths(
        &self,
        root: &NormPathBuf,
    )
        -> Outcome<(PathBuf, PathBuf)> // (source, target).
    {
        // Sass source directory.
        let src_str = &self.css_source_dir_rel;
        if src_str.is_empty() {
            return Err(err!(errmsg!(
                "DevConfig: Css source directory path is empty.",
            ), Invalid, Input, Path));
        }
        let src = Path::new(&src_str).normalise();
        if src.escapes() {
            return Err(err!(errmsg!(
                "DevConfig: Css source directory path {} escapes the directory {:?}.",
                src_str, root,
            ), Invalid, Input, Path));
        }
        let src = root.clone().join(src).normalise().absolute().as_pathbuf();
        res!(PathState::DirMustExist.validate(
            root,
            &src_str,
        ));

        // Target css bundle file.
        let trg_str = &self.css_bundle_rel;
        if trg_str.is_empty() {
            return Err(err!(errmsg!(
                "DevConfig: Css target bundle file is empty.",
            ), Invalid, Input, Path));
        }
        let trg = Path::new(&trg_str).normalise();
        if trg.escapes() {
            return Err(err!(errmsg!(
                "DevConfig: Css target bundle file {} escapes the directory {:?}.",
                trg_str, root,
            ), Invalid, Input, Path));
        }
        let trg = root.clone().join(trg).normalise().absolute().as_pathbuf();

        Ok((src, trg))
    }
}
