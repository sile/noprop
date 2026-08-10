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
- Stateful (model-based) PBT without a separate framework
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

Choosing a search strategy
--------------------------

Start from the property and its semantic input domain. Make every relevant
behavior reachable, then bias boundaries and operation sequences that would
otherwise be too rare. Draw dependent values in order instead of sampling
independent primitives and filtering the combinations afterward. For example,
inside a property closure:

```rust
let len = noprop::sample_with_boundaries(
    ctx,
    &[0usize, 1, 64],
    noprop::Ratio::one_nth(5),
    |ctx| noprop::sample_usize_in(ctx, 0..=64),
);
let input = noprop::sample_bytes_vec(ctx, len);
let split_at = noprop::sample_usize_in(ctx, 0..=input.len());

let mut left = input.clone();
let right = left.split_off(split_at);
left.extend_from_slice(&right);
assert_eq!(left, input);
```

Use `Runner::run` as the baseline. Adjust boundary probabilities and command
weights before increasing the case budget. Switch to
`Runner::run_feedback_guided` only when the failure lies behind rare semantic
progress and the property can report stable, low-cardinality events, buckets,
or transitions. Feedback can steer within a generator's support; it cannot
make an unreachable value reachable.

When to use noprop
------------------

noprop is imperative-first and suits properties that are naturally
sequential — dependent generation, model-based (stateful) commands,
protocol traces — and where writing the generator as plain Rust reads
more clearly than a combinator DSL. The API stays small so a project
can adopt it as a dev-dependency without pulling in a graph of crates.

If you need automatic shrinking or file-based failure persistence,
another PBT library will fit better today; noprop deliberately leaves
those out.

Main constraints
----------------

- `panic=unwind` is required. noprop catches property panics and uses
  panic-based unwinding for `TestCaseContext::reject_case`;
  `panic=abort` is not supported.
- No automatic shrinking. The failure report instead carries an
  automatic value trace — primitive samplers record generated values
  at their source locations — so the failing input is visible without
  extra plumbing. Reproduce the failing case from the seed and case
  budget; if you want a frozen regression, hand-simplify the trace into
  a plain `#[test]`.
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

The four Markdown guides above also render as `docs::*` modules on docs.rs,
alongside the API rustdoc.

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

Agent Skills
------------

An [Agent Skills](https://agentskills.io/) bundle ships with the crate.
Install it with `gh skill install` so a supported AI agent can design
effective search spaces, choose between uniform and feedback-guided search,
and apply noprop's API conventions.

```bash
gh skill install sile/noprop noprop
```

The skill itself lives at
[`skills/noprop/SKILL.md`](skills/noprop/SKILL.md).
