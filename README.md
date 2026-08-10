noprop
======

[![noprop](https://img.shields.io/crates/v/noprop.svg)](https://crates.io/crates/noprop)
[![Documentation](https://docs.rs/noprop/badge.svg)](https://docs.rs/noprop)
[![Actions Status](https://github.com/sile/noprop/workflows/CI/badge.svg)](https://github.com/sile/noprop/actions)
![License](https://img.shields.io/crates/l/noprop)

An imperative property-based testing library for Rust.

- No dependencies
- Expressive without a DSL
  - A small, orthogonal imperative API — the property is a plain Rust
    function that samples values ("generators" in PBT parlance) and
    asserts on the results.
  - Ordinary Rust control flow (`if` / `match` / `for` / recursion)
    and interior mutability express any generator directly — no
    combinator DSL or derive macros to learn.
- Stateful (model-based) PBT built in
  - The same API covers one-line properties, dependent generators,
    and command loops that compare a model against a system under
    test — no separate stateful framework or dependent-generation
    syntax for the harder cases.
- Guided sampling for hard-to-reach states
  - Feedback-guided search steers sampling toward the semantic states
    the property reports as interesting — deep protocol phases,
    hard-to-hit branches, long command sequences a uniform run would
    only find by chance.

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

The seed is caller-supplied, so a failure is reproducible: rerunning
with the seed from the failure report reproduces the identical case.
See [`docs/recipes.md`](docs/recipes.md) for the seed / env-variable
scaffolding, sampling patterns, stateful properties, feedback-guided
search, and the failure-reproduction workflow.

When to use noprop
------------------

noprop is imperative-first and suits properties that are naturally
sequential — dependent generation, model-based (stateful) commands,
protocol traces — and where writing the generator as plain Rust reads
more clearly than a combinator DSL. The API stays small so a project
can adopt it as a dev-dependency without pulling in a graph of crates.

If you need automatic shrinking or file-based failure persistence,
another PBT library will fit better today; noprop deliberately leaves
those out in v0.1 (see below).

Main constraints
----------------

- `panic=unwind` is required. noprop catches property panics and uses
  panic-based unwinding for `TestCaseContext::reject_case`;
  `panic=abort` is not supported.
- No automatic shrinking in v0.1. The failure report instead carries
  an automatic value trace — every `noprop::sample_*` call recorded
  at its source location — so the failing input is visible without
  extra plumbing. Reproduce the failing case from the seed and case
  budget; if you want a frozen regression, hand-simplify the trace
  into a plain `#[test]`.
- No file-based failure persistence. The caller manages the seed and
  case budget; there is no on-disk seed corpus.

Documentation
-------------

- **[Recipes](docs/recipes.md)** — task-oriented recipes for common
  property shapes: seed / run scaffolding, sampling primitives and
  collections, rejection scopes, dependent generators, stateful
  properties, feedback-guided search, and reproducing a failing seed.
- **[Generator design](docs/generator-design.md)** — the small design
  decisions every `sample_*` generator has to make (support,
  distribution, termination, rejection scope, valid-by-construction).
- **[Generator authoring](docs/generator-authoring.md)** — how-to guide
  for writing `sample_*` helpers: composing primitives, bounded
  rejection, the shared `sample_below` migration note, `NonZero`
  recipes, and the finite-by-default float samplers.
- **[Feedback-guided search design](docs/feedback-guided-search.md)** —
  the design of `Runner::run_feedback_guided`, the corpus admission
  and eviction rules, and how the feature registry is bounded.
- **[API reference](https://docs.rs/noprop)** — every function and
  type on docs.rs.

The doc modules ([`docs::recipes`](https://docs.rs/noprop/latest/noprop/docs/recipes/),
[`docs::generator_design`](https://docs.rs/noprop/latest/noprop/docs/generator_design/),
[`docs::generator_authoring`](https://docs.rs/noprop/latest/noprop/docs/generator_authoring/),
[`docs::feedback_guided_search`](https://docs.rs/noprop/latest/noprop/docs/feedback_guided_search/))
render the same Markdown on docs.rs, so the recipes and design notes
appear alongside the API rustdoc.

Examples
--------

The [`examples/`](examples/) directory contains runnable end-to-end
demos of the larger recipes (each runs with `cargo run --example
<name>`):

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
