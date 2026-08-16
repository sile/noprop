//! Imperative property-based testing with no dependencies, macros,
//! unsafe code, or implicit I/O.
//!
//! A property is a plain Rust closure that samples values and asserts
//! on the results. Functions, `if`, `match`, and `for` compose simple
//! inputs, dependent inputs, and stateful operation sequences without
//! a combinator DSL or a separate stateful-testing framework. Every
//! `sample_*` call records its value at the caller's source location,
//! so a failure includes the generated inputs without extra plumbing.
//!
//! # Quick start
//!
//! ```
//! # fn body() -> noprop::TestResult {
//! let seed = noprop::seed_from_env_or_time("MYAPP_PROPTEST_SEED")?;
//! noprop::Runner::new(seed).run(1024, |ctx| {
//!     let a = noprop::sample_u32(ctx);
//!     let b = noprop::sample_u32(ctx);
//!     assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
//!     Ok(())
//! })?;
//! # Ok(())
//! # }
//! # body().expect("the example property must pass");
//! ```
//!
//! The seed is caller-supplied. [`seed_from_env_or_time`] is an
//! explicit convenience for the common test setup: an environment
//! variable overrides a clock-derived fallback. A reported hex seed
//! can be pasted into that variable and replayed.
//!
//! # Designing a property
//!
//! The search space is the set of inputs and operation sequences that
//! the property can generate. A bug-triggering case must be in that
//! set and receive enough probability to occur within the case budget.
//!
//! Draw dependent values in order. For example, draw a variant before
//! fields whose valid range depends on that variant, or inspect the
//! current model state before selecting the next valid command. Use
//! [`sample_with_boundaries`] to assign exact probability to domain
//! boundaries and [`sample_weighted_index`] to keep branch or command
//! weights visible in the property.
//!
//! For a stateful property, create a fresh model and system under test
//! inside each case, drive both through the same bounded command loop,
//! and compare results and state after every meaningful transition.
//! The [stateful example][stateful] demonstrates state-dependent
//! command selection and non-mutating per-transition observations.
//!
//! [`Runner::run`] accepts `Fn`, not `FnMut`, so ordinary mutable
//! captures cannot accidentally carry state between cases. When a
//! cross-case observation is intentional, such as a coverage gate,
//! use interior mutability (`Cell`, `RefCell`, or an atomic). Increment
//! the gate only after the relevant check passes, then assert after the
//! run that the gate was reached. The [search-space example][search-space]
//! shows the complete pattern.
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
//! Copy the seed into the runner (or its environment variable), then
//! rerun with the same case budget, property closure, and relevant
//! external configuration. The run reaches the same failure case
//! index. See [`Runner::run`]'s "Reproducibility" note for the full
//! determinism contract and the [reproduction example][reproduce]
//! for a deliberately failing walkthrough.
//!
//! # Requirements and constraints
//!
//! - **`panic=unwind`.** noprop catches property panics and uses
//!   panic-based unwinding for
//!   [`TestCaseContext::reject_case`], so the crate does not work
//!   under `panic=abort`.
//! - **No automatic shrinking.** Failures are reproduced from
//!   the seed and the case budget; to freeze a specific case as a
//!   regression test, simplify it by hand from the value trace and
//!   inline the witness as a regular `#[test]`. See
//!   [`docs::recipes`] ("Turn a trace into a hand-written regression
//!   test") for the pattern.
//! - **No file-based failure persistence.** The caller manages the
//!   seed and case budget; there is no on-disk seed corpus.
//! - **No type-derived generation.** noprop does not generate values
//!   for user-defined structs or enums from their definitions. Write
//!   an ordinary Rust sampling function so dependencies, boundaries,
//!   and distributions remain explicit.
//! - **No macros or combinator DSL.** Properties and reusable samplers
//!   are ordinary Rust closures and functions.
//!
//! If automatically minimizing a failing input is a requirement,
//! consider a property-testing library that provides shrinking.
//!
//! # Where to look next
//!
//! - [`docs::recipes`] — task-oriented recipes for common property
//!   shapes (sampling, collections, rejection scopes, stateful,
//!   coverage gates, reproduction).
//! - [`docs::generator_design`] — the small design decisions every
//!   `sample_*` generator has to make (support, distribution,
//!   termination, rejection scope, valid-by-construction).
//! - [`docs::generator_authoring`] — how-to guide for writing
//!   `sample_*` helpers: composing primitives, bounded rejection,
//!   `NonZero` recipes, and the finite-by-default float samplers.
//! - Runnable end-to-end demos: [`basics.rs`][basics] for the minimal
//!   shape, [`search_space.rs`][search-space] for explicit search-space
//!   design, [`stateful.rs`][stateful] for model-based command loops,
//!   and [`reproduce.rs`][reproduce] for failure replay. Each runs with
//!   `cargo run --example <name>`; `reproduce` fails on purpose.
//! - The [failure-diagnostics workflow][failure-diagnostics] explains
//!   how to reproduce, diagnose, reduce, and freeze a failure.
//!
//! [basics]: https://github.com/sile/noprop/blob/main/examples/basics.rs
//! [search-space]: https://github.com/sile/noprop/blob/main/examples/search_space.rs
//! [stateful]: https://github.com/sile/noprop/blob/main/examples/stateful.rs
//! [reproduce]: https://github.com/sile/noprop/blob/main/examples/reproduce.rs
//! [failure-diagnostics]: https://github.com/sile/noprop/blob/main/skills/noprop/references/failure-diagnostics.md
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod generator;
mod rng;
mod runner;
mod seed;

pub mod docs;

pub use error::{RunError, RunErrorKind, RunResult, TestResult};
pub use generator::{
    Ratio, sample_ascii_char, sample_ascii_printable_char, sample_ascii_printable_string,
    sample_ascii_string, sample_bool, sample_bytes, sample_bytes_vec, sample_char, sample_choice,
    sample_f32, sample_f32_in, sample_f64, sample_f64_in, sample_i8, sample_i16, sample_i32,
    sample_i64, sample_i128, sample_isize, sample_ratio, sample_string, sample_u8, sample_u16,
    sample_u32, sample_u64, sample_u64_in, sample_u128, sample_usize, sample_usize_in,
    sample_weighted_index, sample_with_boundaries, sample_with_rejection,
};
pub use rng::{GeneratedValue, TestCaseContext};
pub use runner::{Runner, Stats};
pub use seed::seed_from_env_or_time;
