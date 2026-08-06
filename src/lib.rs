//! Imperative property-based testing library with no dependencies, no macros, no unsafe, and no implicit I/O.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod config;
mod error;
mod generator;
mod rng;
mod runner;

pub mod docs;

pub use config::{ConfigError, iterations_from_env, seed_from_env_or_time};
pub use error::{Error, Result};
pub use generator::{
    sample_ascii_char, sample_ascii_printable_char, sample_ascii_printable_string,
    sample_ascii_string, sample_bool, sample_bytes, sample_bytes_vec, sample_char, sample_choice,
    sample_f32, sample_f32_finite, sample_f64, sample_f64_finite, sample_i8, sample_i16,
    sample_i32, sample_i64, sample_i128, sample_isize, sample_ratio, sample_string, sample_u8,
    sample_u16, sample_u32, sample_u64, sample_u128, sample_usize, sample_usize_in,
    sample_weighted_index, sample_with_rejection,
};
pub use rng::{GeneratedValue, TestCaseContext};
pub use runner::{CorpusPolicy, Runner, Stats};
