//! A Sass/SCSS compiler using the Grass engine.
//! 
//! This module provides functionality to compile Sass/SCSS files into a single CSS output file.
//! It handles imports, resolves dependencies, and supports path aliases for `@use` and `@import` rules.
//!
//! # Usage Example
//! ```rust,no_run
//! use oxedyne_fe2o3_core::prelude::*;
//! use oxedyne_fe2o3_steel::app::dev::sass::SassBundle;
//! use std::path::Path;
//!
//! let mut bundler = SassBundle::new();
//!
//! // Optional: Configure load paths for Sass imports.
//! bundler.add_load_path("node_modules");
//!
//! // Compile all Sass/SCSS files.
//! res!(bundler.compile_directory(
//!     // Source directory containing entry point files.
//!     Path::new("src/styles"),
//!     // Output CSS file.
//!     Path::new("dist/styles.css"),
//! ));
//! ```

use oxedyne_fe2o3_core::prelude::*;

use std::{
    collections::{
        HashSet,
    },
    path::{
        Path,
        PathBuf,
    },
    fs,
};

use grass::{
    self,
    Options,
    OutputStyle,
};


#[derive(Debug)]
#[allow(dead_code)]
struct StyleInfo {
    path:       PathBuf,
    content:    String,
}

pub struct SassBundle {
    load_paths:     Vec<PathBuf>,
    processed:      HashSet<PathBuf>,
    styles:         Vec<StyleInfo>,
}

impl SassBundle {
    pub fn new() -> Self {
        Self {
            load_paths:     Vec::new(),
            processed:      HashSet::new(),
            styles:         Vec::new(),
        }
    }

    /// Add a path to search for Sass/SCSS imports.
    /// This is used to resolve `@use` and `@import` rules.
    pub fn add_load_path<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) {
        self.load_paths.push(path.as_ref().to_path_buf());
    }

    pub fn compile_directory(
        mut self,
        css_paths: &(PathBuf, PathBuf),
    ) 
        -> Outcome<()>
    {
        // Find entry points in input directory
        let mut entry_files = Vec::new();
        res!(self.collect_sass_files(&css_paths.0, &mut entry_files));

        // Process each entry point
        for entry in entry_files {
            res!(self.process_style(
                &entry,
            ));
        }

        // Bundle in order of processing
        let mut bundled = String::new();
        for style in &self.styles {
            bundled.push_str(&style.content);
            bundled.push('\n');
        }

        // Write bundled output
        res!(fs::write(css_paths.1.clone(), bundled));
        
        Ok(())
    }

    fn process_style(
        &mut self,
        file_path: &Path,
    )
        -> Outcome<()>
    {
        if self.processed.contains(file_path) {
            return Ok(());
        }
        debug!("Processing {:?}", file_path);
        let content = res!(fs::read_to_string(file_path));
        let mut options = Options::default().style(OutputStyle::Compressed);
    
        // Get directory containing the current file for import resolution.
        let import_dir = match file_path.parent() {
            Some(dir) => dir,
            None => return Err(err!(
                "Could not get parent directory of {:?}.",
                file_path;
                Path)),
        };
        options = options.load_path(import_dir);
    
        // Add all configured load paths to options.
        for load_path in &self.load_paths {
            if let Some(path_str) = load_path.to_str() {
                options = options.load_path(path_str);
            }
        }
    
        let css = match grass::from_string(
            content,
            &options,
        ) {
            Ok(css) => css,
            Err(e) => return Err(err!(e,
                "Why trying to compile Sass file {:?}", file_path;
                IO, Format)),
        };
    
        self.styles.push(StyleInfo {
            path: file_path.to_path_buf(),
            content: css,
        });
        self.processed.insert(file_path.to_path_buf());
        debug!(" Successful processing {:?}", file_path);
        Ok(())
    }

    fn collect_sass_files(
        &self,
        dir:    &Path,
        files:  &mut Vec<PathBuf>,
    )
        -> Outcome<()>
    {
        if dir.is_dir() {
            for entry in res!(fs::read_dir(dir)) {
                let entry = res!(entry);
                let path = entry.path();
                if path.is_dir() {
                    res!(self.collect_sass_files(&path, files));
                } else if let Some(ext) = path.extension() {
                    if ext == "sass" || ext == "scss" {
                        files.push(path);
                    }
                }
            }
        }
        Ok(())
    }
}
