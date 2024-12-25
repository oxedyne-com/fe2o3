//! A JavaScript and TypeScript module bundler based on the SWC compiler.
//! 
//! This module provides functionality to bundle JavaScript and TypeScript files into a single 
//! minified output file. It handles module dependencies, import resolution, and code transformations.
use oxedize_fe2o3_core::{
    prelude::*,
};

use std::{
    collections::{
        HashMap,
        HashSet,
    },
    path::{
        Path,
        PathBuf,
    },
    str::FromStr,
    sync::Arc,
};

use swc_bundler::{
    Bundler,
    Hook,
    Load,
    ModuleData,
    ModuleRecord,
    ModuleType,
    Resolve,
};
use swc_common::{
    FileName,
    GLOBALS,
    Span,
    SourceMap,
};
use swc_ecma_ast::{
    EsVersion, 
    KeyValueProp, 
    PropName,
    Expr, 
    Lit, 
    Str,
    IdentName,
};
use swc_ecma_codegen::{
    text_writer::JsWriter,
    Emitter,
};
use swc_ecma_loader::{
    resolve::Resolution,
    resolvers::{
        lru::CachingResolver,
        node::NodeModulesResolver,
    },
    TargetEnv,
};
use swc_ecma_parser::{
    parse_file_as_module,
    Syntax,
    TsSyntax,
};


/// Supported file extensions for bundling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    JavaScript,
    JavaScriptModule,
    TypeScript,
    TypeScriptModule,
    TypeScriptReact,
}

impl FromStr for FileType {
    type Err = Error<ErrTag>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "js"    => Ok(Self::JavaScript),
            "mjs"   => Ok(Self::JavaScriptModule),
            "ts"    => Ok(Self::TypeScript),
            "mts"   => Ok(Self::TypeScriptModule),
            "tsx"   => Ok(Self::TypeScriptReact),
            _ => Err(err!(errmsg!(
                "Unknown file type extension: {}", s
            ), Invalid, Input, String, Conversion))
        }
    }
}

/// Module resolution allowing the use of javascript import aliases.
#[derive(Debug)]
pub struct AliasResolver {
    aliases:        Arc<Vec<(String, PathBuf)>>,
}

impl AliasResolver {
    pub fn new(
        aliases:        Vec<(String, PathBuf)>,
    )
        -> Self
    {
        Self {
            aliases: Arc::new(aliases),
        }
    }

    fn try_resolve_with_extensions(&self, path: &Path) -> Option<PathBuf> {
        // First try exact path.
        if path.exists() {
            return Some(path.to_path_buf());
        }
    
        // Try file extensions if path doesn't have one
        if path.extension().is_none() {
            // Regular file extensions
            let file_exts = [".mjs", ".js", ".ts"];
            for ext in file_exts {
                let with_ext = path.with_extension(ext.trim_start_matches('.'));
                if with_ext.exists() {
                    return Some(with_ext);
                }
            }
    
            // Check for index files if path is directory
            if path.is_dir() {
                for ext in file_exts {
                    let index_path = path.join(fmt!("index{}", ext));
                    if index_path.exists() {
                        return Some(index_path);
                    }
                }
            }
        }
    
        None
    }
}

impl Resolve for AliasResolver {

    fn resolve(
        &self,
        base:               &FileName,
        module_specifier:   &str,
    )
        -> Result<Resolution, anyhow::Error>
    {
        // First try to resolve using aliases.
        for (alias, path) in self.aliases.iter() {
            if module_specifier.starts_with(alias) {
                // Remove alias prefix and combine with alias path.
                if let Some(rel_path) = module_specifier.strip_prefix(alias) {
                    let full_path = path.join(rel_path.trim_start_matches('/'));

                    // Try with extensions.
                    if let Some(resolved_path) = self.try_resolve_with_extensions(&full_path) {
                        // Resolved alias paths.
                        return Ok(Resolution {
                            filename: FileName::Real(resolved_path),
                            slug: None,
                        });
                    }

                    // If no extension matches but the specified path exists, use it.
                    if full_path.exists() {
                        // Exact alias paths.
                        return Ok(Resolution {
                            filename: FileName::Real(full_path),
                            slug: None,
                        });
                    }
                }
            }
        }

        // If not an alias, fallback to node module resolution.
        let resolver = NodeModulesResolver::new(
            TargetEnv::Browser,
            Default::default(),
            true,
        );

        match resolver.resolve(base, module_specifier) {
            // Successful node resolution.
            Ok(resolution) => Ok(resolution),
            // Failed resolution.
            Err(_) => Ok(Resolution {
                filename: FileName::Real(PathBuf::new()),
                slug: None,
            }),
        }
    }
}

/// Configuration for the bundler.
#[derive(Debug, Clone)]
pub struct BundleConfig {
    /// Primary entry point files (if empty, all found files are entries).
    pub entry_points: Vec<PathBuf>,
    /// Supported file types to bundle.
    pub file_types: HashSet<FileType>,
    /// Target ECMAScript version.
    pub target_version: EsVersion,
    /// Enable TypeScript support.
    pub typescript: bool,
    /// Enable minification.
    pub minify: bool,
}

impl Default for BundleConfig {
    fn default() -> Self {
        let mut file_types = HashSet::new();
        file_types.insert(FileType::JavaScript);
        file_types.insert(FileType::JavaScriptModule);
        file_types.insert(FileType::TypeScript);
        file_types.insert(FileType::TypeScriptModule);
        file_types.insert(FileType::TypeScriptReact);

        Self {
            entry_points:   Vec::new(),
            file_types,
            target_version: EsVersion::Es2020,
            typescript:     true,
            minify:         true,
        }
    }
}

/// Custom hook for SWC bundler to handle imports.
struct BundleHook;

impl Hook for BundleHook {

    fn get_import_meta_props(
        &self,
        span:   Span,
        record: &ModuleRecord,
    )
        -> Result<Vec<KeyValueProp>, anyhow::Error>
    {
        info!("Processing imports for module: {}", record.file_name);
        
        // Convert filesystem path to web path
        let web_path = match &record.file_name {
            FileName::Real(path) => {
                // Find the "www" directory in the path and take everything after it
                path.to_string_lossy()
                    .split("/www/")
                    .nth(1)
                    .map(|p| fmt!("/{}", p))
                    .unwrap_or_else(|| "/".to_string())
            },
            _ => "/".to_string(),
        };

        let mut props = Vec::new();
        props.push(KeyValueProp {
            key: PropName::Ident(IdentName::new("url".into(), span)),
            value: Box::new(Expr::Lit(Lit::Str(Str {
                span,
                value: web_path.into(),
                raw: None,
            }))),
        });

        Ok(props)
    }
}

/// Loads JavaScript/TypeScript modules for the bundler.
struct ModuleLoader {
    /// Source map for code generation and error reporting.
    cm: Arc<SourceMap>,
    /// Configuration for parsing modules.
    config: BundleConfig,
}

impl ModuleLoader {
    fn new(cm: Arc<SourceMap>, config: BundleConfig) -> Self {
        Self { cm, config }
    }
}

impl Load for ModuleLoader {

    fn load(&self, filename: &FileName) -> Result<ModuleData, anyhow::Error> {
        let module_path = match filename {
            FileName::Real(path) => path.display().to_string(),
            _ => "<unknown>".to_string(),
        };
        info!("Attempting to load module: {}", module_path);

        // Load the source file.
        let fm = match filename {
            FileName::Real(path) => {
                if !path.exists() {
                    return Err(anyhow::anyhow!(err!(errmsg!(
                        "Module not found: {:?}. Check import paths in requesting module.",
                        path,
                    ), IO, File, Missing)));
                }
                match self.cm.load_file(path) {
                    Ok(fm) => {
                        debug!("Successfully loaded file: {}", module_path);
                        fm
                    },
                    Err(e) => return Err(anyhow::anyhow!(err!(e, errmsg!(
                        "Failed to load file: {:?}", path,
                    ), IO, File, Read))),
                }
            },
            _ => return Err(anyhow::anyhow!(err!(errmsg!(
                "Unsupported module type: {:?}", filename,
            ), IO, File))),
        };

        // Determine syntax based on file extension and config
        let syntax = if self.config.typescript {
            Syntax::Typescript(TsSyntax {
                tsx: fm.name.to_string().ends_with(".tsx"),
                decorators: true,
                dts: false,
                no_early_errors: false,
                disallow_ambiguous_jsx_like: false,
            })
        } else {
            Syntax::Es(Default::default())
        };

        debug!("Parsing module with syntax: {:?}", syntax);
        let module = match parse_file_as_module(
            &fm,
            syntax,
            self.config.target_version,
            None,
            &mut vec![],
        ) {
            Ok(m) => {
                debug!("Successfully parsed module");
                m
            },
            Err(e) => return Err(anyhow::anyhow!(err!(errmsg!(
                "Failed to parse module {}: {:?}", fm.name, e,
            ), IO, Format))),
        };

        Ok(ModuleData {
            fm,
            module,
            helpers: Default::default(),
        })
    }
}

/// Handles bundling of JavaScript and TypeScript files.
pub struct JsBundle {
    /// Bundle path map.
    bundles_map:    Vec<(PathBuf, PathBuf)>,
    /// Path aliases for module resolution.
    path_aliases:   Vec<(String, PathBuf)>,
    /// Bundling configuration.
    config:         BundleConfig,
}

impl JsBundle {

    /// Entry points must be specified to ensure correct bundling regardless of project structure.
    /// The bundler will follow imports from these entry points to build the dependency graph.
    pub fn new(
        bundles_map:    Vec<(PathBuf, PathBuf)>,
        path_aliases:   Vec<(String, PathBuf)>,
    )
        -> Self
    {
        let entry_points = bundles_map
            .clone()
            .into_iter()
            .map(|jsbm| jsbm.0)
            .collect();
        Self {
            bundles_map,
            path_aliases,
            config: BundleConfig {
                entry_points,
                ..Default::default()
            },
        }
    }

    pub fn bundle_entries(
        &self,
        src_dir: &Path,
    )
        -> Outcome<()>
    {
        info!("Starting JS bundling from directory: {:?}", src_dir);
        
        for js_bundle_map in &self.bundles_map {
            info!("Bundling entry point: {:?} -> {:?}", 
                js_bundle_map.0, js_bundle_map.1);
                
            // Verify entry file exists
            if !js_bundle_map.0.exists() {
                return Err(err!(errmsg!(
                    "Entry point file not found: {:?}", js_bundle_map.0,
                ), IO, File, Missing));
            }
    
            let mut entries = HashMap::new();
            entries.insert(
                fmt!("entry"),
                FileName::Real(js_bundle_map.0.clone()),
            );
    
            // Create output directory if needed
            if let Some(parent) = js_bundle_map.1.parent() {
                if !parent.exists() {
                    res!(std::fs::create_dir_all(parent));
                }
            }
    
            let result = GLOBALS.set(&swc_common::Globals::new(), || {
                let cm = Arc::new(SourceMap::default());
                let globals = GLOBALS.set(&swc_common::Globals::new(), || {
                    swc_common::Globals::new()
                });
                
                // Create custom resolver with aliases
                let resolver = CachingResolver::new(
                    4096,
                    AliasResolver::new(
                        self.path_aliases.clone(),
                    ),
                );

                let loader = ModuleLoader::new(cm.clone(), self.config.clone());

                let mut bundler = Bundler::new(
                    &globals,
                    cm.clone(),
                    loader,
                    resolver,
                    swc_bundler::Config {
                        require: false,
                        disable_inliner: false,
                        external_modules: vec![],
                        module: ModuleType::Es,
                        disable_fixer: false,
                        disable_hygiene: false,
                        disable_dce: false,
                    },
                    Box::new(BundleHook),
                );
    
                let bundles = catch_other!(bundler.bundle(entries), IO, Format);
                info!("Bundle generation complete, processing {} modules", bundles.len());
                
                let mut combined = String::new();
                for (idx, bundle) in bundles.iter().enumerate() {
                    info!("Processing bundle module {}/{}", idx + 1, bundles.len());
                    let mut buf = vec![];
                    {
                        let writer = JsWriter::new(
                            cm.clone(),
                            "\n",
                            &mut buf,
                            None,
                        );
                        let mut emitter = Emitter {
                            cfg: swc_ecma_codegen::Config::default(),
                            comments: None,
                            cm: cm.clone(),
                            wr: Box::new(writer),
                        };

                        if let Err(e) = emitter.emit_module(&bundle.module) {
                            return Err(err!(e, errmsg!(
                                "Failed to emit bundle module {} for entry point {:?}",
                                idx, js_bundle_map.0,
                            ), IO, Format));
                        }
                    }

                    match String::from_utf8(buf) {
                        Ok(s) => combined.push_str(&s),
                        Err(e) => return Err(err!(e, errmsg!(
                            "Generated invalid UTF-8 in bundle module {}",
                            idx,
                        ), IO, Format)),
                    };
                    combined.push('\n');
                }

                // Write bundle with error context
                if let Err(e) = std::fs::write(&js_bundle_map.1, combined) {
                    return Err(err!(e, errmsg!(
                        "Failed to write bundle to {:?}", js_bundle_map.1,
                    ), IO, File, Write));
                }

                info!("Successfully bundled {:?}", js_bundle_map.0);
                Ok(())
            });

            res!(result);
        }

        info!("JavaScript bundling completed successfully");
        Ok(())
    }

}
