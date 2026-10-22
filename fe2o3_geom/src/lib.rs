//! A geometry library: 2D layout and positioning, planar construction, and the sphere.
//!
//! On the sphere, `cell` is a global cell-index grid, `proj` holds map projections and a
//! viewport that draws clipped rings on a flat map or a globe, and `world` is an offline world
//! map with the simplifier that makes one.  `tile` is the Web Mercator tile grid with a
//! PMTiles reader, and `mvt` decodes the vector tiles street maps arrive in.
//!
//! The layout types focus on rectangles with absolute and relative positioning support,
//! making them suitable for UI layout systems. Key features include:
//! 
//! - Dimension types with saturating arithmetic for safe calculations
//! - Coordinate system with zero-based positioning
//! - Rectangle types supporting both absolute and relative positioning
//! - Flexible dimension system for fluid layouts
//! - Clipping and intersection support
//! - Position enums for common alignment scenarios (top-left, centre, etc.)
//! 
//! The crate is particularly useful for:
//! - Terminal user interfaces
//! - Widget layout systems  
//! - Window management
//! - Any application requiring 2D layout calculation
//!
#![forbid(unsafe_code)]
pub mod cell;
pub mod dim;
pub mod mvt;
pub mod planar;
pub mod proj;
pub mod rect;
pub mod rigid;
pub mod shape;
pub mod tile;
pub mod world;
