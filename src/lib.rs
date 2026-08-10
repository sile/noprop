//! Imperative property-based testing library with no dependencies, no
//! macros, no unsafe, and no implicit I/O.
//!
//! Properties are plain
//! `Fn(&mut TestCaseContext) -> Result<(), Box<dyn Error>>` closures
//! that use ordinary Rust control flow — `if` / `match` / `for` — to
//! draw values and check them, so a test reads like the code it
//! exercises. Every `noprop::sample_*` call records the produced
//! value at its source location, so a failure surfaces the actual
//! generated input without extra plumbing.
//!
//! # Quick start
//!
//! ```
//! # fn body() -> noprop::TestResult {
//! noprop::Runner::new(0xDEAD_BEEF).run(256, |ctx| {
//!     let a = noprop::sample_u32(ctx);
//!     let b = noprop::sample_u32(ctx);
//!     assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
//!     Ok(())
//! })?;
//! # Ok(())
//! # }
//! # body().unwrap();
//! ```
//!
//! The seed is always caller-supplied; noprop never reads the system
//! clock or environment on its own. For a `#[test]` that stays
//! reproducible from a failure report, read the seed from an
//! environment variable and fall back to the clock:
//!
//! ```
//! # fn body() -> noprop::TestResult {
//! let seed = noprop::seed_from_env_or_time("MYAPP_SEED")?;
//! noprop::Runner::new(seed).run(256, |_ctx| Ok(()))?;
//! # Ok(())
//! # }
//! # body().unwrap();
//! ```
//!
//! Pick a project-specific variable name — the hex seed printed by a
//! failure report can be pasted into it verbatim.
//!
//! # Choosing a runner
//!
//! - [`Runner::run`] samples inputs uniformly. This is the default;
//!   use it unless a property has a rare region that uniform sampling
//!   would only reach by luck.
//! - [`Runner::run_feedback_guided`] steers the search toward inputs
//!   that report new semantic features via
//!   [`TestCaseContext::event`], [`bucket`](TestCaseContext::bucket),
//!   and [`transition`](TestCaseContext::transition). The property
//!   closure has the same shape as `run`, and the feedback methods are
//!   allocation-free no-ops under `Runner::run`, so the same property
//!   can be exercised under both entry points. The design is
//!   documented in [`docs::feedback_guided_search`].
//!
//! # Reproducing a failure
//!
//! A property failure (panic, returned `Err`, or too-many-rejections
//! exit) becomes a [`RunError`] carrying the seed, the case index, the
//! recorded value trace, and observability [`Stats`]. Both `Debug` and
//! `Display` include a reproduce hint that reuses the original case
//! budget:
//!
//! ```text
//! reproduce with: noprop::Runner::new(0x...).run(N, |ctx| ...)
//! ```
//!
//! Copy the seed into the runner (or into the env variable read by
//! `seed_from_env_or_time`) and rerun with the same case budget and
//! the same property closure — the run reproduces the identical
//! failure case index. The [`examples/reproduce.rs`][reproduce] file
//! walks through the full workflow with a deliberately failing
//! property.
//!
//! [reproduce]: https://github.com/sile/noprop/blob/main/examples/reproduce.rs
//!
//! # Requirements and constraints
//!
//! - **`panic=unwind`.** noprop catches property panics and uses
//!   panic-based unwinding for
//!   [`TestCaseContext::reject_case`], so the crate does not work
//!   under `panic=abort`.
//! - **No automatic shrinking in v0.1.** Failures are reproduced from
//!   the seed and the case budget; to freeze a specific case as a
//!   regression test, simplify it by hand from the value trace and
//!   inline the witness as a regular `#[test]`. See
//!   [`docs::recipes`] ("Turn a trace into a hand-written regression
//!   test") for the pattern.
//! - **No file-based failure persistence.** The caller manages the
//!   seed and case budget; there is no on-disk seed corpus.
//! - **Static argument errors panic, external input errors return
//!   `Result`.** Generator misuse
//!   (`sample_with_rejection(ctx, 0, _)`, `Ratio::new(2, 1)` called
//!   on an unchecked input) is a caller bug and panics with a
//!   `#[track_caller]` message. Environment or config errors that a
//!   user can hit at runtime (a mistyped `MYAPP_SEED`) return
//!   `Result`.
//! - **`sample_with_rejection` attempt budgets are explicit.** Every
//!   call names its own `max_attempts`; there is no library-wide
//!   default.
//!
//! noprop is imperative-first and does not replace a general PBT
//! library where automatic shrinking or seed persistence is the
//! priority. It fits well when the property is naturally sequential
//! (dependent generation, stateful commands, protocol traces) and
//! reads more clearly as plain Rust than as a combinator DSL.
//!
//! # Where to look next
//!
//! - [`docs::recipes`] — task-oriented recipes for common property
//!   shapes (sampling, collections, rejection scopes, stateful,
//!   feedback-guided, reproduction).
//! - [`docs::generator_design`] — the small design decisions every
//!   `sample_*` generator has to make (support, distribution,
//!   termination, rejection scope, valid-by-construction).
//! - [`docs::generator_authoring`] — how-to guide for writing
//!   `sample_*` helpers: composing primitives, bounded rejection,
//!   the shared `sample_below` migration note, `NonZero` recipes,
//!   and the finite-by-default float samplers.
//! - [`docs::feedback_guided_search`] — the design of
//!   [`Runner::run_feedback_guided`], the corpus admission and
//!   eviction rules, and how the feature registry is bounded.
//! - The [`examples/`][examples] directory ships runnable end-to-end
//!   demos: `basics.rs`, `stateful.rs`, `feedback_guided.rs`,
//!   `rejection.rs`, `reproduce.rs`. Each runs with
//!   `cargo run --example <name>`.
//!
//! [examples]: https://github.com/sile/noprop/tree/main/examples
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
mod error;
mod generator;
mod rng;
mod runner;

pub mod docs;

pub use config::seed_from_env_or_time;
pub use error::{RunError, RunErrorKind, RunResult, TestResult};
pub use generator::{
    Ratio, sample_ascii_char, sample_ascii_printable_char, sample_ascii_printable_string,
    sample_ascii_string, sample_bool, sample_bytes, sample_bytes_vec, sample_char, sample_choice,
    sample_f32, sample_f32_in, sample_f64, sample_f64_in, sample_i8, sample_i16, sample_i32,
    sample_i64, sample_i128, sample_isize, sample_ratio, sample_string, sample_u8, sample_u16,
    sample_u32, sample_u64, sample_u128, sample_usize, sample_usize_in, sample_weighted_index,
    sample_with_boundaries, sample_with_rejection,
};
pub use rng::{GeneratedValue, TestCaseContext};
pub use runner::{Runner, Stats};
