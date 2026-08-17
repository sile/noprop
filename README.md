noprop
======

[![noprop](https://img.shields.io/crates/v/noprop.svg)](https://crates.io/crates/noprop)
[![Documentation](https://docs.rs/noprop/badge.svg)](https://docs.rs/noprop)
[![Actions Status](https://github.com/sile/noprop/workflows/CI/badge.svg)](https://github.com/sile/noprop/actions)
![License](https://img.shields.io/crates/l/noprop)

An imperative property-based testing library for Rust.

- Expressive with a small imperative API
  - A property samples values and checks results directly with ordinary
    Rust functions, control flow, state, and assertions. The same building
    blocks scale from simple properties to stateful tests.
  - No combinator DSL, derive macros, or separate stateful framework is
    required.
- Explicit control over the search space
  - A well-designed search space is essential: bug-triggering cases must
    be reachable and likely enough to occur.
  - noprop keeps search-space decisions in the property code: what can be
    generated, how often each path is chosen, and how later choices depend
    on earlier draws or the current state.
  - With the imperative API, a coverage gate is an ordinary Rust check. It
    verifies that the run exercised an important region of the search
    space, failing the test if no case reaches the relevant assertion.
- No dependencies

Quick start
-----------

```rust
#[test]
fn addition_is_commutative() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("MYAPP_PROPTEST_SEED")?;
    noprop::Runner::new(seed).run(1024, |ctx| {
        let a = noprop::sample_u32(ctx);
        let b = noprop::sample_u32(ctx);
        assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
        Ok(())
    })?;
    Ok(())
}
```

On failure, copy the reported seed into `MYAPP_PROPTEST_SEED` and rerun
the same test with the same case budget to reproduce the failing case.

When noprop fits
----------------

noprop is especially direct when a property has one or more of these
shapes:

- The next value depends on an earlier length, variant, setting, or the
  current state
- A command sequence compares a model with the system under test after
  every transition
- History and operation order affect a protocol, parser, or streaming API
- Empty, singleton, maximum, or other domain boundaries need explicit
  probability
- A coverage gate must prevent success when an important invariant never
  runs

Designing the search space
--------------------------

First make every relevant input and operation sequence reachable. Then
shape how often they occur: use `sample_with_boundaries` for domain
boundaries, `sample_weighted_index` for branches or commands, and ordinary
control flow for values that depend on earlier draws or the current state.

A property can pass vacuously when no case reaches an important assertion.
Count evidence only after the important check succeeds, then assert after
`Runner::run` that the count is non-zero. This post-run assertion is a
coverage gate.

Adjust support, boundary probabilities, and branch weights before
increasing the case budget. See [Recipes](docs/recipes.md) for concrete
patterns and [Generator design](docs/generator-design.md) for support,
distribution, termination, and rejection scope.

Reproducing failures and constraints
------------------------------------

A failure report contains the seed, case index, generated-value trace,
and the original case budget in a reproduce hint. Reuse the same seed,
case budget, property closure, and relevant external configuration. The
trace and semantic assertion message identify the failing inputs and
transition.

noprop deliberately has a small contract:

- Failing inputs are not automatically shrunk; simplify a reproduced
  witness by hand and freeze it as a regular regression test
- Failure seeds are not persisted to files; the caller owns the seed and
  case budget
- Values for user-defined structs and enums are not generated from their
  type definitions; write samplers as ordinary Rust functions so their
  dependencies, boundaries, and distributions stay explicit
- Macros and a combinator DSL are not provided

If automatically minimizing a failing input is a requirement, consider a
property-testing library that provides shrinking.

Where to look next
------------------

- **[Recipes](docs/recipes.md)** — task-oriented patterns for seed and
  run scaffolding, sampling, rejection, stateful properties, coverage
  gates, and failure diagnosis
- **[Generator design](docs/generator-design.md)** — how to choose a
  generator's support, distribution, termination, and rejection scope
- **[Generator authoring](docs/generator-authoring.md)** — how to write
  reusable `sample_*` functions from noprop's primitives
- **[API reference](https://docs.rs/noprop)** — every public function and
  type
- **[Runnable examples](examples/)** — end-to-end properties:
  - [`basics.rs`](examples/basics.rs) — the minimal property shape and an
    environment-controlled seed
  - [`search_space.rs`](examples/search_space.rs) — dependent draws,
    boundary probabilities, branch weights, and coverage gates
  - [`stateful.rs`](examples/stateful.rs) — state-dependent commands and
    per-transition model / SUT checks
  - [`reproduce.rs`](examples/reproduce.rs) — replaying a failure from its
    seed and report; this example fails on purpose
- **[noprop skill](skills/noprop/SKILL.md)** — search-space design and API
  guidance for supported AI coding agents; install it with
  `gh skill install sile/noprop noprop`

The Markdown guides also render as `docs::*` modules on docs.rs.
