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

Targeted search (`Runner::run_targeted`) is available behind the same
imperative API; its design is documented in
[docs/targeted-search.md](https://github.com/sile/noprop/blob/main/docs/targeted-search.md).

Example
-------

```rust
#[test]
fn addition_is_commutative() -> noprop::Result<()> {
    noprop::Runner::new(0xDEAD_BEEF, 1024).run(|ctx| {
        let a = noprop::sample_u32(ctx);
        let b = noprop::sample_u32(ctx);
        assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
        Ok(())
    })?;
    Ok(())
}
```

Status
------

Early development (`v0.0.x`). API is unstable and may change without notice.

Detection benchmark
-------------------

`examples/detection_benchmark` measures how many iterations noprop
needs to detect known mutants of five small workloads (high-frequency,
boundary, combination, dependent, bst) under different generator
variants, and how broad the generated inputs are (semantic buckets).

```
# Run a single task: print one raw-result JSON line.
cargo run --example detection_benchmark -- run \
    --workload bst --mutant insert_duplicate_key --variant uniform --seed 0

# Run every task over a seed cohort: one raw-result JSON line per trial.
cargo run --example detection_benchmark -- run-all \
    --iterations 1000 --seeds 0,1,2,3,4,5,6,7 > raw.jsonl

# Regenerate the bucket summary from raw results.
cargo run --example detection_benchmark -- summary < raw.jsonl
```

The `base` variant (ground-truth SUT) completes every property and is
used to verify the workloads; the comparison variants are `uniform`
and `biased`. Raw results are written as format-versioned JSON lines,
so summaries can always be regenerated from a saved cohort. Smoke
tests live in `tests/detection_benchmark.rs`.

These numbers measure only the chosen workloads, mutants, seed
cohort, and iteration budget. They are not a complete measure of
generator quality: a generator that wins on one target may lose on
another, and detection speed says nothing about shrinking quality.
