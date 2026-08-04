//! Supplemental documentation for noprop's design.
//!
//! Document bodies live in `docs/` as Markdown and are pulled in with
//! `include_str!`, so they are browsable on docs.rs and their Rust
//! code examples run as doctests.

/// Design of the targeted search policy: recording, exploratory
/// replay, corpus, and mutation.
#[doc = include_str!("../docs/targeted-search.md")]
pub mod targeted_search {}
