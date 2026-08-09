//! Supplemental documentation for noprop's design.
//!
//! Document bodies live in `docs/` as Markdown and are pulled in with
//! `include_str!`, so they are browsable on docs.rs and their Rust
//! code examples run as doctests.

/// Design of the feedback-guided search policy: semantic features, the
/// feature registry, corpus admission and eviction, and scheduling.
#[doc = include_str!("../docs/feedback-guided-search.md")]
pub mod feedback_guided_search {}
