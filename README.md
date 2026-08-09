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
- Imperative API — properties are plain `Fn(&mut TestCaseContext) -> Result<(), Box<dyn Error>>` closures that use ordinary Rust control flow (`if` / `match` / `for`) instead of combinator DSLs
- Automatic value trace — each `noprop::sample_*` call is recorded at its source location and surfaced on failure, so the failing input is visible without extra plumbing
- Feedback-guided search (`Runner::run_feedback_guided`) over semantic feedback (`event` / `bucket` / `transition`); its design is documented in
  [docs/feedback-guided-search.md](https://github.com/sile/noprop/blob/main/docs/feedback-guided-search.md)

Example
-------

```rust
#[test]
fn addition_is_commutative() -> noprop::TestResult {
    noprop::Runner::new(0xDEAD_BEEF).run(1024, |ctx| {
        let a = noprop::sample_u32(ctx);
        let b = noprop::sample_u32(ctx);
        assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
        Ok(())
    })?;
    Ok(())
}
```

To mix boundary values into a draw with an exact probability, use
`sample_with_boundaries` — 10% of the time one of the candidates
(which may be domain-level values such as an MTU or a page size),
otherwise a uniform draw:

```rust
let x = noprop::sample_with_boundaries(
    ctx,
    &[0, 1500, u32::MAX],
    noprop::Ratio::ONE_TENTH,
    noprop::sample_u32,
);
```

More examples
-------------

The [`examples/`](examples/) directory contains runnable demos of the
larger recipes (each runs with `cargo run --example <name>`):

- [`basics.rs`](examples/basics.rs) — the minimal property shape
  against a real function, the common pitfalls (`Fn` closures and
  interior mutability, environment-controlled seeds via
  `seed_from_env_or_time`), and the short idioms for random-length
  collections and boundary values
- [`stateful.rs`](examples/stateful.rs) — model-based (stateful)
  property testing of an LRU cache with a bounded command loop
- [`feedback_guided.rs`](examples/feedback_guided.rs) — steering the
  search toward interesting inputs (long log lines that exercise
  truncation) with `event` / `bucket` / `transition`
- [`rejection.rs`](examples/rejection.rs) — `sample_with_rejection`
  for constrained draws and `reject_case` for whole-case
  preconditions, parsing `key=value` config lines
- [`reproduce.rs`](examples/reproduce.rs) — reproducing a failing
  seed via `NOPROP_SEED` and the failure report's reproduce hint
  (this one fails on purpose)

Benchmark
---------

The detection benchmark harness lives in the `benchmark/` workspace
crate; see its [`README.md`](benchmark/README.md) for how to run it.

