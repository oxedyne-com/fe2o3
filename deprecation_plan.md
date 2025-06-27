# Crates.io Migration Plan: oxedize_fe2o3_* → oxedyne_fe2o3_*

## Phase 1: Reserve New Names ✅
- [ ] Run `./reserve_crates.sh` to create placeholder packages
- [ ] Run `./publish_reservations.sh` to publish placeholders
- [ ] Verify all 25 package names are reserved on crates.io

## Phase 2: Deprecate Old Packages
For each existing `oxedize_fe2o3_*` package:

1. **Update Cargo.toml** to add deprecation notice:
```toml
[package]
name = "oxedize_fe2o3_core"
version = "0.5.1"  # Bump version
# ... existing fields ...

[package.metadata.docs.rs]
rustdoc-args = ["--html-in-header", "deprecated.html"]
```

2. **Add deprecation notice** to lib.rs:
```rust
#![deprecated(since = "0.5.1", note = "This package has moved to `oxedyne_fe2o3_core`. Please update your dependencies.")]

//! # ⚠️ DEPRECATED PACKAGE
//! 
//! This package has been moved to [`oxedyne_fe2o3_core`](https://crates.io/crates/oxedyne_fe2o3_core).
//! 
//! Please update your `Cargo.toml` to use the new package name:
//! 
//! ```toml
//! [dependencies]
//! oxedyne_fe2o3_core = "0.6.0"  # Use latest version
//! ```
//! 
//! ## Migration Guide
//! 
//! 1. Replace `oxedize_fe2o3_core` with `oxedyne_fe2o3_core` in Cargo.toml
//! 2. Update import statements: `use oxedize_fe2o3_core` → `use oxedyne_fe2o3_core`
//! 3. All APIs remain identical - only the package name changed
```

3. **Update README.md** with deprecation notice

4. **Publish deprecation version** (0.5.1)

## Phase 3: Publish Real Packages
1. Update version to 0.6.0 for all packages
2. Publish full implementations under `oxedyne_fe2o3_*` names
3. Add migration documentation

## Timeline
- **Immediate**: Reserve names (Phase 1)
- **Next 1-2 weeks**: Deprecate old packages (Phase 2)  
- **When ready**: Publish new packages (Phase 3)

## User Migration
Users will see clear deprecation warnings and can migrate by:
1. Changing package names in Cargo.toml
2. Updating use statements
3. No API changes required

## Benefits
- Clean namespace transition
- Clear migration path for users
- Maintains backward compatibility during transition
- Professional handling of the rebrand