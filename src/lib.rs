//! Imperative property-based testing library with no dependencies, no macros, and no unsafe.
#![forbid(unsafe_code)]

mod generator;
mod rng;

pub use generator::{
    Generate, bool, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize,
};
pub use rng::Rng;
