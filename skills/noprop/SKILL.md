---
name: noprop
description: >
  Reference for the noprop crate — imperative property-based testing in Rust
  with no macros, no combinator DSL, and no dependencies. Use when the task
  mentions noprop, or when writing property tests / stateful (model + SUT)
  checks / feedback-guided search / rejection scopes in Rust with `Runner`,
  `TestCaseContext`, `Ratio`, or any `sample_*` primitive, or when reading
  the bundled docs (`docs::recipes`, `docs::generator_authoring`,
  `docs::generator_design`, `docs::feedback_guided_search`).
license: MIT
---

# noprop

Property-based testing written as imperative closures. Properties are plain
`Fn(&mut TestCaseContext) -> Result<(), Box<dyn Error>>` values that use
ordinary Rust control flow — `if` / `match` / `for` — to draw values and
check them, so a test reads like the code under test.

## What this crate is like

- **Imperative closure style.** No macros, no combinator DSL. Properties
  are closures; sequencing is plain Rust.
- **One generator shape.** Every `sample_*` primitive is
  `fn sample_X(ctx: &mut TestCaseContext) -> X`. User-defined generators
  (`fn sample_person(ctx: &mut TestCaseContext) -> Person`) follow the same
  shape and compose by plain call.
- **Static argument errors panic, external input errors return `Result`.**
  Caller-bug inputs (`Ratio::new(2, 1)` on an unchecked value,
  `sample_with_rejection(ctx, 0, _)`) panic with a `#[track_caller]`
  message. Environment / config values (`MYAPP_SEED` parse failure) return
  a `Result`.
- **Rejection scope is explicit.** `sample_with_rejection(ctx, N, f)`
  redraws bounded within one draw; `TestCaseContext::reject_case()` throws
  the whole case away. They do not overlap.
- **Reproduction is seed-driven.** v0.1 has no automatic shrinking and no
  on-disk failure corpus. `RunError`'s `Display` / `Debug` embeds a
  `reproduce with: noprop::Runner::new(0x...).run(N, |ctx| ...)` hint that
  reruns the identical failure case index.
- **No `unsafe`, no implicit I/O, no dependencies.** `panic = "unwind"` is
  required (`reject_case` uses panic-based unwind).

## Version info

- crate: `noprop`
- version: 0.0.4 (approaching v0.1)
- Rust edition: 2024
- MSRV: 1.88
- license: MIT
- dependencies: none (workspace root; only a `benchmark` member crate lives
  alongside)

## Bundled documentation

`src/lib.rs` re-exports `pub mod docs;`, and each entry pulls a Markdown
file in via `#[doc = include_str!]`. All four render on docs.rs alongside
the API rustdoc.

| Module | What it covers |
|--------|---------------|
| `docs::recipes` | Task-oriented recipes: seed / run scaffolding, sampling primitives and collections, rejection scopes, dependent / stateful / cluster / streaming properties, feedback-guided search, coverage gate, reproducing failures, turning a trace into a hand-written regression test. |
| `docs::generator_authoring` | How-to guide for writing `sample_*` helpers: composing primitives, bounded rejection, the shared `sample_below` migration note, the two `NonZero` recipes, and the finite-by-default float samplers. |
| `docs::generator_design` | Small design reference every `sample_*` generator has to satisfy: support, distribution, termination, rejection scope, valid-by-construction. |
| `docs::feedback_guided_search` | Design of `Runner::run_feedback_guided`: the three feedback methods, corpus admission and eviction, the global feature registry cap, the per-case cap. |

For any "how do I write X with noprop", the fastest path is
`docs::recipes` first, then drop down to the API rustdoc.

## Core types

### `Runner`

| Method | Description |
|--------|-------------|
| `Runner::new(seed: u64) -> Self` | Construct with a fixed seed. noprop never reads system time or environment on its own. |
| `runner.run(cases, |ctx| { ... Ok(()) }) -> RunResult` | Uniform sampling (default). Use unless the property has a rare region uniform sampling would only hit by luck. |
| `runner.run_feedback_guided(cases, |ctx| { ... }) -> RunResult` | Steer the search toward inputs that report new semantic features via `ctx.event` / `bucket` / `transition`. |
| `runner.stats() -> Stats` | Observation counters for the most recent run. |

The property closure returns `Result<(), Box<dyn Error>>` (`TestResult`).
Both a returned `Err` and a panic are treated as a failure and produce a
`RunError`.

### `Stats`

| Field | Description |
|-------|-------------|
| `accepted_cases: usize` | Cases that were kept (the counter that closes the `cases` budget). |
| `rejected_cases: usize` | Cases dropped via `reject_case` or a `sample_with_rejection` exhaustion. |
| `total_samples: usize` | `sample_*` calls made, including those in rejected cases. |
| `discovered_features: usize` | Distinct semantic features registered during a feedback-guided run (capped at 1024, currently). |
| `max_corpus_size: usize` | Peak combined size (accepted + rejected) of the feedback-guided corpus. |

### `TestCaseContext`

The only argument the property closure receives. Draws values, reports
feedback, and can skip the current case.

| Method | Description |
|--------|-------------|
| `TestCaseContext::new(seed: u64) -> Self` | Direct construction (for doctests / experiments outside a `Runner`). |
| `ctx.reject_case() -> !` | Unwind out of the current case; the runner discards it and moves on. Panic-based. |
| `ctx.event(label: &'static str)` | Report a bounded-count occurrence. Counts saturate into buckets (1 / 2-3 / 4-7 / 8+). |
| `ctx.bucket(label: &'static str, value: u64)` | Report a pre-discretized state value. Aim for roughly 3-10 buckets per label. |
| `ctx.transition(label: &'static str, from: u64, to: u64)` | Report an abstract state transition; the `(from, to)` pair is part of the feature identity. |

`event` / `bucket` / `transition` are allocation-free no-ops under
`Runner::run`, so the same property closure can be exercised under both
`run` and `run_feedback_guided`.

### `RunError` and `RunErrorKind`

Failure value returned by `Runner::run` / `run_feedback_guided`.

| Method | Description |
|--------|-------------|
| `err.seed() -> u64` | The seed the failing runner was constructed with. |
| `err.case_index() -> usize` | Zero-based index of the failing case. |
| `err.generated() -> &[GeneratedValue]` | Value trace recorded during the failing case (source location, type name, `Debug` representation where available). |
| `err.stats() -> Stats` | Counters at the point of failure. |
| `err.kind() -> RunErrorKind` | `PropertyFailure` (the closure panicked or returned `Err`) or `TooManyRejections` (the internal global rejection limit was reached before the budget). |

`Display` and `Debug` print the seed, case index, value trace, and a
`reproduce with: noprop::Runner::new(0x...).run(N, |ctx| ...)` hint that
reruns the identical failure case.

### `GeneratedValue`

One entry in `RunError::generated()`.

| Method | Description |
|--------|-------------|
| `type_name() -> &'static str` | Runtime type name of the recorded value (`"u32"`, `"String"`, …). |
| `location() -> &'static Location<'static>` | Source location of the `sample_*` call. |
| `value_repr() -> Option<String>` | `Debug` representation of the value (may be absent when the type has no useful `Debug`). |
| `is_elided() -> bool` | Whether the trace entry was elided for output-size control. |
| `elided_count() -> Option<usize>` | For elided entries, how many inner items were dropped (bytes / chars / …). |

### `Ratio`

Exact rational probability. Passed to `sample_ratio` and
`sample_with_boundaries`.

| Method | Description |
|--------|-------------|
| `Ratio::new(numerator: u32, denominator: u32) -> Self` | General `m/n` form for compile-time literals. Panics (`#[track_caller]`) on `denominator == 0` or `numerator > denominator`. |
| `Ratio::one_nth(n: u32) -> Self` | Single-argument shortcut for `1/N`. Panics on `n == 0`. |

Both are `const fn`. For runtime values, clamp or validate yourself before
calling `Ratio::new`; noprop deliberately does not ship a fallible or
clamping constructor.

## Sampling primitives

All follow `fn sample_X(ctx: &mut TestCaseContext) -> X` and carry
`#[track_caller]`.

### Integers

| Function | Output |
|----------|--------|
| `sample_bool(ctx)` | uniform `bool` |
| `sample_u8` / `u16` / `u32` / `u64` / `u128` / `usize` | uniform of the named type |
| `sample_i8` / `i16` / `i32` / `i64` / `i128` / `isize` | uniform of the named type |
| `sample_usize_in(ctx, range)` | uniform in a `Range` / `RangeInclusive` / `RangeFrom` / etc. (bias-free bounded rejection) |

`sample_usize_in(ctx, 0..n)` is the correct alternative to
`sample_usize(ctx) % n` (the latter is biased and overflows at
`usize::MAX`).

### Floats

| Function | Output |
|----------|--------|
| `sample_f32(ctx)` / `sample_f64(ctx)` | uniform **finite** value by default (`NaN` and `±∞` excluded via a small bounded rejection loop) |
| `sample_f32_in(ctx, min, max)` / `sample_f64_in(ctx, min, max)` | uniform in `[min, max)` |

To sample an arbitrary bit pattern (including `NaN`, infinities,
subnormals), build it explicitly: `f32::from_bits(noprop::sample_u32(ctx))`.
See `docs::generator_authoring` ("Sampling floats").

### Bytes, chars, strings

| Function | Output |
|----------|--------|
| `sample_bytes::<N>(ctx) -> [u8; N]` | fixed-length byte array |
| `sample_bytes_vec(ctx, len) -> Vec<u8>` | byte buffer of the given length |
| `sample_char(ctx) -> char` | any valid Unicode scalar (surrogates excluded via bounded rejection) |
| `sample_ascii_char(ctx) -> char` | `0x00..=0x7F` (control characters included) |
| `sample_ascii_printable_char(ctx) -> char` | `0x20..=0x7E` |
| `sample_string(ctx, len)` / `sample_ascii_string(ctx, len)` / `sample_ascii_printable_string(ctx, len)` | strings of exactly `len` code points |

Length is measured in code points, not UTF-8 bytes. For random-length
strings, pair with `sample_usize_in` (length first, string second).

### Selection, weighting, bias

| Function | Description |
|----------|-------------|
| `sample_choice(ctx, &[T]) -> T` | uniform pick from a slice (`T: Clone`) |
| `sample_weighted_index(ctx, &[u32]) -> usize` | index chosen with probability proportional to each weight; panics on empty slice or all-zero weights |
| `sample_ratio(ctx, Ratio) -> bool` | returns `true` with probability `Ratio` |
| `sample_with_boundaries(ctx, &[T], Ratio, sample) -> T` | with probability `Ratio` pick uniformly from the boundary slice, otherwise call `sample(ctx)` |

### Bounded rejection sampling

| Function | Description |
|----------|-------------|
| `sample_with_rejection(ctx, max_attempts, |ctx| Option<T>) -> T` | Redraw until the closure returns `Some`, up to `max_attempts` tries. On exhaustion the enclosing case is rejected via `TestCaseContext::reject_case()`, so this recipe requires a `Runner` around it. |

An internal rejection sampler shared by `sample_usize_in`, `sample_ratio`,
`sample_weighted_index`, `sample_choice`, and `sample_with_boundaries` is
bounded at 64 attempts per call. The exhaustion probability is under
`2⁻⁶⁴`; inside `Runner::run` the case is rejected, outside a runner the
call panics.

### Feedback (feedback-guided runs)

`TestCaseContext::event` / `bucket` / `transition`; see the
`TestCaseContext` table above and `docs::feedback_guided_search` for the
design.

### Seed helper

| Function | Description |
|----------|-------------|
| `seed_from_env_or_time(var: &str) -> TestResult<u64>` | Read `var` (hex `0x…`, decimal, `0b…`, `0o…` with `_` separators); fall back to the system clock when unset. The only place noprop touches env / time. |

## Typical patterns

### Fixed seed, plain `Runner::run`

```rust
noprop::Runner::new(0xDEAD_BEEF).run(256, |ctx| {
    let a = noprop::sample_u32(ctx);
    let b = noprop::sample_u32(ctx);
    assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
    Ok(())
})?;
```

### Reproducible seed via env variable

```rust
let seed = noprop::seed_from_env_or_time("MYAPP_SEED")?;
noprop::Runner::new(seed).run(256, |ctx| {
    // property body
    Ok(())
})?;
```

Paste the hex seed printed by a failure report into `MYAPP_SEED` to hit
the same failing case index on the next run.

### Stateful (model-based)

```rust
noprop::Runner::new(0).run(64, |ctx| {
    let mut model: Vec<u32> = Vec::new();
    let mut sut: Vec<u32> = Vec::new();
    let steps = noprop::sample_usize_in(ctx, 0..=16);
    for _ in 0..steps {
        match noprop::sample_usize_in(ctx, 0..2) {
            0 => {
                let v = noprop::sample_u32(ctx);
                model.push(v);
                sut.push(v);
            }
            _ => {
                assert_eq!(model.pop(), sut.pop(), "pop mismatch");
            }
        }
    }
    Ok(())
})?;
```

Cluster-level invariants, bounded run-to-quiescence, cross-step
invariants with an append-only history, and stateful streaming APIs live
as separate recipes in `docs::recipes`.

### Feedback-guided run with a coverage gate

```rust
use std::cell::Cell;

let long_line_hits: Cell<usize> = Cell::new(0);
let mut runner = noprop::Runner::new(0xFEED);
runner.run_feedback_guided(256, |ctx| {
    let len = noprop::sample_usize_in(ctx, 0..=24);
    let _line = noprop::sample_string(ctx, len);
    if len > 12 {
        ctx.event("long-line");
        long_line_hits.set(long_line_hits.get() + 1);
    }
    Ok(())
})?;
assert!(
    long_line_hits.get() > 0,
    "the `long-line` region must be reached at least once\n{runner}"
);
```

The same `Cell` counter + run-after assert applies to `Runner::run`
(uniform) whenever a state-dependent invariant would otherwise pass
vacuously. See `docs::recipes` ("Assert a coverage gate after the run").

### Choosing a rejection scope

```rust
// Per-draw bounded rejection: sample_with_rejection.
noprop::Runner::new(0).run(64, |ctx| {
    let odd_u32 = noprop::sample_with_rejection(ctx, 64, |ctx| {
        let v = noprop::sample_u32(ctx);
        (v % 2 == 1).then_some(v)
    });
    assert_eq!(odd_u32 % 2, 1);
    Ok(())
})?;

// Case-wide skip: TestCaseContext::reject_case.
noprop::Runner::new(0).run(64, |ctx| {
    let n = noprop::sample_u32(ctx);
    if n < 100 {
        ctx.reject_case(); // not counted toward accepted_cases
    }
    // Only cases with n >= 100 reach here.
    Ok(())
})?;
```

## Conventions and gotchas

- **Do not introduce macros or combinator DSLs.** Properties stay plain
  Rust closures; branching is `if` / `match`, loops are `for`. `proptest!`
  / `arbitrary`-style DSLs are outside noprop's design.
- **Use `Ratio::new` / `Ratio::one_nth` at literal sites.** Both panic on
  invalid inputs. noprop deliberately does not ship a fallible or
  clamping constructor; clamp runtime values yourself before calling
  `new`.
- **`sample_with_rejection`'s `max_attempts` is always explicit.** There
  is no library-wide default (crate internals happen to use 64
  everywhere, but the user API restates it at every call).
- **Use `sample_usize_in`, not `sample_usize(ctx) % max`.** The modulo
  form is biased and overflows at `usize::MAX`.
- **Failure messages carry the context, not just the value.** Include
  the step index, mismatched values, and — for stateful properties —
  the command history or the model state at the point of failure.
- **Feedback-reporting closures also run under uniform `Runner::run`.**
  `event` / `bucket` / `transition` are no-ops there, so a single
  property body can be exercised under both entry points.
- **`NonZero<_>` is a two-recipe pick, not a built-in primitive.** The
  uniform recipe (rejection loop, `Runner`-only) and the biased recipe
  (`if v == 0 { 1 } else { v }`, always terminates in one draw) trade
  distribution uniformity against unconditional termination. See
  `docs::generator_authoring` ("Sampling non-zero integers").
- **Value trace comes from `Debug`.** `sample_*` records the produced
  value's `Debug` representation. `Cell<_>` and other non-`Debug` types
  render as elided entries in the trace.

## Known limitations (v0.1)

- **No automatic shrinking.** Reproduction is seed + case budget only;
  regressions are hand-simplified from the value trace and inlined as
  `#[test]`.
- **No on-disk failure persistence.** The caller manages seeds. Recovery
  is via printout or env variable.
- **`panic = "unwind"` required.** `reject_case` uses panic-based
  unwinding; `panic = "abort"` builds do not work.
- **Feedback-guided caps are internal constants.** Currently 1024
  distinct features globally and 64 features per case.
- **`sample_with_rejection` restates `max_attempts` per call.** No
  library-wide default is provided.

## Where to look next

- **crate rustdoc** (`https://docs.rs/noprop/`): the `pub` API and all
  four doc modules render there.
- **`docs::recipes`** (source: `docs/recipes.md`): task-oriented recipes.
  Start here whenever the question is "how do I do X".
- **`docs::generator_authoring`** (source: `docs/generator-authoring.md`):
  authoring guide for user-defined `sample_*` helpers.
- **`docs::generator_design`** (source: `docs/generator-design.md`):
  support / distribution / termination decisions each generator has to
  make.
- **`docs::feedback_guided_search`** (source:
  `docs/feedback-guided-search.md`): internal model of feedback-guided
  runs (feature identity, corpus, registry cap).
- **`examples/`**: `basics.rs`, `stateful.rs`, `feedback_guided.rs`,
  `rejection.rs`, `reproduce.rs`. Each runs with
  `cargo run --example <name>`.
