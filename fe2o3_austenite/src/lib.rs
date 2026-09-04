//! Austenite: a streaming, two-pass typesetting engine.
//!
//! This crate began as the Phase 0 spine -- a walking skeleton, not a typesetter. It carries a
//! box/glue/penalty intermediate representation, a jdat ledger of anchors, and a two-pass driver
//! whose convergence loop is honest about how it terminates. Pages are emitted as SVG through
//! `fe2o3_graphics`. Phase 1 wires the metric seam to `fe2o3_font`: real text is shaped with
//! HarfBuzz, measured against a real face, and drawn as glyph outlines (see [`font`]). Knuth-Plass
//! line breaking and Pearl output remain later phases, stubbed behind clean seams here.
//!
//! The design is set out in `doc/Austenite/sec_architecture.typ` and `sec_decisions.typ`. Two
//! commitments from there shape every type below:
//!
//! * *Stratify hard.* Layout facts reach nothing outside the engine except through the ledger, and
//!   only through anchor classes known in advance. There is no query, no state, no `counter.at`.
//! * *Scaled integers.* All lengths are integers in scaled units, as in TeX, never floating point,
//!   so a break decision is exact and a build is reproducible. See [`ir::Sp`].
//!
//! A note on units and `fe2o3_geom`. The architecture mandates a *signed* scaled-integer length
//! (a kern, a glue shrink and a depth below the baseline are all naturally negative or paired with
//! a negative). `fe2o3_geom::dim::Dim` is an unsigned `usize` built for terminal and widget layout,
//! so it cannot carry that model. The typographic core therefore uses [`ir::Sp`]; `fe2o3_geom` is
//! kept for the device-space output boundary, where extents are non-negative (see
//! [`page::PageGeometry::media_box`]).
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod driver;
pub mod emit;
pub mod font;
pub mod ir;
pub mod ledger;
pub mod page;
