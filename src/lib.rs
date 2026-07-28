//! Imperative property-based testing library with no dependencies, no macros, and no unsafe.
#![forbid(unsafe_code)]

mod generator;
mod rng;

pub use generator::{
    Generate, bool, i8, i16, i32, i64, i128, isize, non_zero_i8, non_zero_i16, non_zero_i32,
    non_zero_i64, non_zero_i128, non_zero_isize, non_zero_u8, non_zero_u16, non_zero_u32,
    non_zero_u64, non_zero_u128, non_zero_usize, u8, u16, u32, u64, u128, usize,
};
pub use rng::Rng;
