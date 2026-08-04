//! Supplemental documentation for noprop's design.
//!
//! Document bodies live in `docs/` as Markdown and are pulled in with
//! `include_str!`, so they are browsable on docs.rs and their Rust
//! code examples run as doctests.

/// Design of the targeted search policy: recording, exploratory
/// replay, corpus, and mutation.
#[doc = include_str!("../docs/targeted-search.md")]
pub mod targeted_search {}

/// Design of the corpus-guided search policy: semantic features, the
/// feature registry, corpus admission and eviction, and scheduling.
#[doc = include_str!("../docs/corpus-guided-search.md")]
pub mod corpus_guided_search {}
