noprop
======

[![noprop](https://img.shields.io/crates/v/noprop.svg)](https://crates.io/crates/noprop)
[![Documentation](https://docs.rs/noprop/badge.svg)](https://docs.rs/noprop)
[![Actions Status](https://github.com/sile/noprop/workflows/CI/badge.svg)](https://github.com/sile/noprop/actions)
![License](https://img.shields.io/crates/l/noprop)

An imperative property-based testing library for Rust.

- No dependencies
- No macros
- No `unsafe` code (`#![forbid(unsafe_code)]`)
- No implicit I/O — seeds are always caller-supplied, so every run is fully reproducible
- Imperative API — properties are plain `Fn(&mut Rng) -> Result<(), Box<dyn Error>>` closures that use ordinary Rust control flow (`if` / `match` / `for`) instead of combinator DSLs
- Automatic value trace — each `noprop::gen_*` call is recorded at its source location and surfaced on failure, so the failing input is visible without extra plumbing

Example
-------

```rust
use noprop::{Runner, gen_u32};

#[test]
fn addition_is_commutative() -> noprop::Result<()> {
    Runner { seed: 0xDEAD_BEEF, iterations: 1024 }.run(|rng| {
        let a = gen_u32(rng);
        let b = gen_u32(rng);
        assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
        Ok(())
    })
}
```

Status
------

Early development (`v0.0.x`). API is unstable and may change without notice.
