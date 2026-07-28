//! Imperative property-based testing library with no dependencies, no macros, and no unsafe.
#![forbid(unsafe_code)]

mod generator;
mod rng;

pub use generator::{
    gen_bool, gen_i8, gen_i16, gen_i32, gen_i64, gen_i128, gen_isize, gen_non_zero_i8,
    gen_non_zero_i16, gen_non_zero_i32, gen_non_zero_i64, gen_non_zero_i128, gen_non_zero_isize,
    gen_non_zero_u8, gen_non_zero_u16, gen_non_zero_u32, gen_non_zero_u64, gen_non_zero_u128,
    gen_non_zero_usize, gen_u8, gen_u16, gen_u32, gen_u64, gen_u128, gen_usize,
};
pub use rng::Rng;
