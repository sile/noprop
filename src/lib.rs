//! Imperative property-based testing library with no dependencies, no macros, no unsafe, and no implicit I/O.
#![forbid(unsafe_code)]

mod error;
mod generator;
mod rng;
mod runner;

pub use error::{Error, Result};
pub use generator::{
    gen_ascii_char, gen_ascii_printable_char, gen_bool, gen_char, gen_choice, gen_f32, gen_f64,
    gen_i8, gen_i16, gen_i32, gen_i64, gen_i128, gen_isize, gen_non_zero_i8, gen_non_zero_i16,
    gen_non_zero_i32, gen_non_zero_i64, gen_non_zero_i128, gen_non_zero_isize, gen_non_zero_u8,
    gen_non_zero_u16, gen_non_zero_u32, gen_non_zero_u64, gen_non_zero_u128, gen_non_zero_usize,
    gen_u8, gen_u16, gen_u32, gen_u64, gen_u128, gen_usize,
};
pub use rng::{GeneratedValue, Rng};
pub use runner::Runner;
