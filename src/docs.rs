//! Supplemental documentation for noprop's design.
//!
//! Document bodies live in `docs/` as Markdown and are pulled in with
//! `include_str!`, so they are browsable on docs.rs and their Rust
//! code examples run as doctests.

/// Task-oriented recipes for common property shapes: the seed / run
/// scaffolding, sampling primitives and collections, rejection scopes,
/// stateful properties, feedback-guided search, and reproducing a
/// failing seed.
#[doc = include_str!("../docs/recipes.md")]
pub mod recipes {}

/// Design of the feedback-guided search policy: semantic features, the
/// feature registry, corpus admission and eviction, and scheduling.
#[doc = include_str!("../docs/feedback-guided-search.md")]
pub mod feedback_guided_search {}

/// Small design reference for writing `sample_*` generators: support,
/// distribution, termination, rejection scope, and
/// valid-by-construction.
#[doc = include_str!("../docs/generator-design.md")]
pub mod generator_design {}

/// Authoring guide for `sample_*` generators: composing primitives,
/// bounded rejection sampling, the shared `sample_below` migration
/// note, `NonZero<_>` recipes, and the finite-by-default float
/// samplers.
#[doc = include_str!("../docs/generator-authoring.md")]
pub mod generator_authoring {}
