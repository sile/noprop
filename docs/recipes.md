# Recipes

Short task-oriented recipes for writing properties with noprop. Each
recipe follows the same five-part shape:

- **Goal.** What the property is trying to do.
- **Uses.** The public API called by the recipe.
- **Code.** A minimal, self-contained example. Every code block is
  either a doctest verified by `cargo test --doc` or a link to a
  runnable example in `examples/`.
- **Notes.** Pitfalls, trade-offs, and things worth knowing before
  applying the recipe to a real property.
- **See also.** The API rustdoc entries and full-example files that go
  deeper.

Individual primitives (`sample_u8`, `sample_u16`, …) are documented in
their own rustdoc; this document lists property shapes, not primitive
signatures.

## Contents

- [Run a property with a seed](#run-a-property-with-a-seed)
- [Sample primitives, ranges, and strings](#sample-primitives-ranges-and-strings)
- [Build `Vec`, `Option`, maps, and sets](#build-vec-option-maps-and-sets)
- [Pick between enum variants and weight the choice](#pick-between-enum-variants-and-weight-the-choice)
- [Mix in domain boundaries with an exact probability](#mix-in-domain-boundaries-with-an-exact-probability)
- [Sample `NonZero` integers](#sample-nonzero-integers)
- [Write a dependent generator](#write-a-dependent-generator)
- [Bounded recursion and bounded loops](#bounded-recursion-and-bounded-loops)
- [Choose a rejection scope](#choose-a-rejection-scope)
- [Model-based (stateful) property](#model-based-stateful-property)
- [Steer the search with feedback](#steer-the-search-with-feedback)
- [Assert a coverage gate after the run](#assert-a-coverage-gate-after-the-run)
- [Reproduce a failing seed](#reproduce-a-failing-seed)
- [Turn a trace into a hand-written regression test](#turn-a-trace-into-a-hand-written-regression-test)
- [Observe cross-case state with interior mutability](#observe-cross-case-state-with-interior-mutability)
- [Keep the trace pointing at the user's call site](#keep-the-trace-pointing-at-the-users-call-site)

## Run a property with a seed

**Goal.** Execute a property closure against a fixed or
environment-controlled seed, so failures are reproducible.

**Uses.** [`Runner::new`](crate::Runner::new),
[`Runner::run`](crate::Runner::run),
[`seed_from_env_or_time`](crate::seed_from_env_or_time),
[`TestResult`](crate::TestResult).

```
# fn body() -> noprop::TestResult {
// A fixed seed keeps the run repeatable.
noprop::Runner::new(0xDEAD_BEEF).run(256, |ctx| {
    let x = noprop::sample_u32(ctx);
    assert_eq!(x, x);
    Ok(())
})?;

// An env-controlled seed lets a failure report be replayed by
// setting the variable. The env helper accepts decimal, `0x...`,
// `0b...`, and `0o...` with optional `_` separators, so the hex seed
// printed by a failure report can be pasted in verbatim.
let seed = noprop::seed_from_env_or_time("MYAPP_SEED")?;
noprop::Runner::new(seed).run(256, |_ctx| Ok(()))?;
# Ok(())
# }
# body().unwrap();
```

**Notes.** The seed is always caller-supplied; noprop never reads from
`SystemTime` or the OS on its own. Failures print a `reproduce with:`
hint that reuses the original case budget, so the rerun hits the same
rejection cap.

**See also.**
[`Runner::run`](crate::Runner::run),
[`seed_from_env_or_time`](crate::seed_from_env_or_time),
[`examples/basics.rs`](https://github.com/sile/noprop/blob/main/examples/basics.rs).

## Sample primitives, ranges, and strings

**Goal.** Draw the standard scalar and collection primitives that most
properties need.

**Uses.** [`sample_u32`](crate::sample_u32) (and the other integer
`sample_*` primitives), [`sample_usize_in`](crate::sample_usize_in),
[`sample_string`](crate::sample_string),
[`sample_ascii_printable_string`](crate::sample_ascii_printable_string),
[`sample_bool`](crate::sample_bool).

```
let mut ctx = noprop::TestCaseContext::new(0);

// Full-width integers.
let _n: u32 = noprop::sample_u32(&mut ctx);

// Bounded range: uniform, bias-free, no `% N` overflow at usize::MAX.
let idx = noprop::sample_usize_in(&mut ctx, 0..10);
assert!(idx < 10);
let day = noprop::sample_usize_in(&mut ctx, 1..=31);
assert!((1..=31).contains(&day));

// Length-taking string primitives: pick the length first, then draw.
let len = noprop::sample_usize_in(&mut ctx, 0..=32);
let _s = noprop::sample_string(&mut ctx, len);
let _ascii = noprop::sample_ascii_printable_string(&mut ctx, len);

let _b = noprop::sample_bool(&mut ctx);
```

**Notes.** Never write `sample_usize(ctx) % max` for a bounded draw —
it is biased and overflows at `usize::MAX`. Length-taking helpers take
an *exact* length; combine with `sample_usize_in` for a random length
(no `(min, max)` overload is provided so composition stays the one
shape).

**See also.** [`sample_usize_in`](crate::sample_usize_in),
[`sample_string`](crate::sample_string), the module docstrings on
`src/generator.rs`.

## Build `Vec`, `Option`, maps, and sets

**Goal.** Assemble collections with ordinary Rust control flow, without
combinator DSLs.

**Uses.** [`sample_usize_in`](crate::sample_usize_in),
[`sample_bool`](crate::sample_bool),
[`sample_u32`](crate::sample_u32),
[`sample_string`](crate::sample_string).

```
use std::collections::{HashMap, HashSet};

let mut ctx = noprop::TestCaseContext::new(0);

// Vec of random length.
let len = noprop::sample_usize_in(&mut ctx, 0..=8);
let v: Vec<u32> = (0..len).map(|_| noprop::sample_u32(&mut ctx)).collect();
assert_eq!(v.len(), len);

// Option: draw the discriminant first.
let opt: Option<u32> = if noprop::sample_bool(&mut ctx) {
    Some(noprop::sample_u32(&mut ctx))
} else {
    None
};
let _ = opt;

// HashMap of random size, distinct keys by construction (draw from a
// small pool so a set actually forms).
let n = noprop::sample_usize_in(&mut ctx, 0..=4);
let map: HashMap<u8, u32> = (0..n)
    .map(|_| {
        let key = noprop::sample_usize_in(&mut ctx, 0..=15) as u8;
        (key, noprop::sample_u32(&mut ctx))
    })
    .collect();
assert!(map.len() <= n);

// HashSet from the same pattern.
let set: HashSet<u8> = (0..n)
    .map(|_| noprop::sample_usize_in(&mut ctx, 0..=15) as u8)
    .collect();
assert!(set.len() <= n);
```

**Notes.** Composition is just Rust: `for`, `iter().map(...)`, `if`,
`match`. `HashMap` / `HashSet` will silently deduplicate colliding
keys, so if the property needs *exactly* `n` distinct keys, draw them
from a set-shaped pool (a permutation of a small range) rather than a
uniform space.

**See also.**
[`sample_usize_in`](crate::sample_usize_in),
[`sample_bool`](crate::sample_bool).

## Pick between enum variants and weight the choice

**Goal.** Branch between code paths (each producing a different value),
optionally with unequal weights.

**Uses.** [`sample_usize_in`](crate::sample_usize_in),
[`sample_weighted_index`](crate::sample_weighted_index),
[`sample_choice`](crate::sample_choice).

```
let mut ctx = noprop::TestCaseContext::new(0);

// Uniform one-of-N branching.
let _x: u32 = match noprop::sample_usize_in(&mut ctx, 0..3) {
    0 => 0,
    1 => noprop::sample_u32(&mut ctx),
    _ => u32::MAX,
};

// Weighted one-of-N branching: weight 5 / 3 / 2.
let _y: u32 = match noprop::sample_weighted_index(&mut ctx, &[5, 3, 2]) {
    0 => 0,
    1 => noprop::sample_u32(&mut ctx),
    _ => u32::MAX,
};

// Fixed list of *values* (not branches): use sample_choice.
let _n = noprop::sample_choice(&mut ctx, &[1u32, 2, 3, 5, 8]);
let _digit = noprop::sample_choice(&mut ctx, b"0123456789") as char;
```

**Notes.** Use `match sample_usize_in(...)` / `sample_weighted_index`
for *branches* (calling different generators); use `sample_choice` for
one value from a fixed *list*. Mixing the two conflates "which code
runs" with "which value is picked".

**See also.**
[`sample_weighted_index`](crate::sample_weighted_index),
[`sample_choice`](crate::sample_choice).

## Mix in domain boundaries with an exact probability

**Goal.** Sample a value that is uniform most of the time but hits a
few caller-chosen boundary values (0, `u16::MAX`, an MTU, a page size)
with an exact probability.

**Uses.** [`sample_with_boundaries`](crate::sample_with_boundaries),
[`Ratio`](crate::Ratio).

```
let mut ctx = noprop::TestCaseContext::new(0);
let port = noprop::sample_with_boundaries(
    &mut ctx,
    &[0u16, 1500, u16::MAX],
    noprop::Ratio::ONE_TENTH,
    noprop::sample_u16,
);
// 10% of the time `port` is one of {0, 1500, u16::MAX};
// otherwise a uniform u16.
let _ = port;
```

**Notes.** The probability is exact rational (`Ratio::new(1, 10)` is
one-in-ten, not `0.10`-close). Choose boundaries that map to distinct
outcomes in the code under test — otherwise the extra probability
mass on the boundary set does nothing observable.

**See also.**
[`sample_with_boundaries`](crate::sample_with_boundaries),
[`Ratio`](crate::Ratio),
[`examples/basics.rs`](https://github.com/sile/noprop/blob/main/examples/basics.rs)
("Idiom 2: boundary values").

## Sample `NonZero` integers

**Goal.** Produce a `NonZero<_>` value from a plain integer primitive
without shipping a dedicated helper.

**Uses.**
[`sample_with_rejection`](crate::sample_with_rejection) (uniform),
[`sample_u32`](crate::sample_u32) + explicit remap (biased).

```
# use std::num::NonZeroU32;
// Uniform: rejects the case if 64 attempts all draw 0 (astronomically
// unreachable for u32; requires a Runner around it).
# let _: noprop::RunResult = noprop::Runner::new(0).run(1, |ctx| {
let n = noprop::sample_with_rejection(ctx, 64, |ctx| {
    NonZeroU32::new(noprop::sample_u32(ctx))
});
assert_ne!(n.get(), 0);
# Ok(())
# });

// Biased: always terminates in one draw, shifts a small mass onto 1.
let mut ctx = noprop::TestCaseContext::new(0);
let v = noprop::sample_u32(&mut ctx);
let n = NonZeroU32::new(if v == 0 { 1 } else { v })
    .expect("v was remapped away from zero");
assert_ne!(n.get(), 0);
```

**Notes.** The two recipes trade distribution uniformity against
unconditional termination. `wrapping_add(1)` is *not* a valid
substitute — it wraps `u32::MAX` back to `0`. For signed types the
full `NonZero<i_>` domain is `MIN..=-1 ∪ 1..=MAX`, so only the
rejection recipe covers both signs uniformly.

**See also.** The "Sampling non-zero integers" section of the
`src/generator.rs` module docstring.

## Write a dependent generator

**Goal.** Let a later draw's domain depend on an earlier draw.

**Uses.** Whatever primitives the domain needs — the pattern is
"draw, then branch".

```
let mut ctx = noprop::TestCaseContext::new(0);

// Draw a version first, then a payload whose length is bounded by it.
let version = noprop::sample_usize_in(&mut ctx, 1..=3);
let max_len = match version {
    1 => 8,
    2 => 32,
    _ => 128,
};
let len = noprop::sample_usize_in(&mut ctx, 0..=max_len);
let payload = noprop::sample_bytes_vec(&mut ctx, len);
assert!(payload.len() <= max_len);
```

**Notes.** No combinator DSL is required — dependent generators are
plain Rust functions that call the primitives sequentially. Keep the
drawing pattern deterministic on the accumulated state: changing the
number or type of draws mid-way shifts every subsequent draw for the
same seed.

**See also.**
[`sample_usize_in`](crate::sample_usize_in), the "Composing
generators" section of the `src/generator.rs` module docstring.

## Bounded recursion and bounded loops

**Goal.** Generate a tree, a nested value, or a command sequence
without risking unbounded work per case.

**Uses.** [`sample_usize_in`](crate::sample_usize_in), an explicit
depth or count parameter.

```
use noprop::TestCaseContext;

#[derive(Debug)]
enum Tree {
    Leaf(u8),
    Node(Box<Tree>, Box<Tree>),
}

fn sample_tree(ctx: &mut TestCaseContext, depth: usize) -> Tree {
    // depth == 0 forces a leaf: bounded recursion by construction.
    if depth == 0 || noprop::sample_bool(ctx) {
        Tree::Leaf(noprop::sample_u32(ctx) as u8)
    } else {
        Tree::Node(
            Box::new(sample_tree(ctx, depth - 1)),
            Box::new(sample_tree(ctx, depth - 1)),
        )
    }
}

let mut ctx = TestCaseContext::new(0);
let _tree = sample_tree(&mut ctx, 4);

// Bounded command loop: use a for loop with a fixed upper bound
// instead of a while loop that could spin.
let steps = noprop::sample_usize_in(&mut ctx, 0..=16);
for _ in 0..steps {
    let _cmd = noprop::sample_usize_in(&mut ctx, 0..3);
    // apply the command
}
```

**Notes.** Any recursion or loop needs a decreasing budget. Without
one, a hostile choice sequence could keep the case running past any
per-case draw cap. The `for _ in 0..n` shape is enough for a command
sequence; a `while` loop needs an explicit maximum step count.

**See also.** [`sample_usize_in`](crate::sample_usize_in),
[`sample_bool`](crate::sample_bool).

## Choose a rejection scope

**Goal.** Redraw either a single value or the whole case, depending on
where the constraint sits.

**Uses.** [`sample_with_rejection`](crate::sample_with_rejection) (per
draw), [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case)
(per case). Prefer *valid-by-construction* over either when
practical.

```
# let _: noprop::RunResult = noprop::Runner::new(0).run(1, |ctx| {
// Per-draw rejection: an identifier (a-z_ start, then a-z0-9_).
let is_identifier = |s: &str| {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
};
let key = noprop::sample_with_rejection(ctx, 16, |ctx| {
    let k = noprop::sample_ascii_string(ctx, 2);
    is_identifier(&k).then_some(k)
});

// Per-case rejection: reject after a whole-case precondition fails.
let line = noprop::sample_ascii_printable_string(ctx, 12);
if line.is_empty() || line.starts_with('#') {
    ctx.reject_case();
}
let _ = key;
# Ok(())
# });
```

**Notes.** Rejected cases are retried and do not count toward the
`cases` budget, but are bounded internally so a generator that always
rejects still terminates with `TooManyRejections`. When the accept
rate would be very low, redesign the generator to be valid by
construction (draw the constraint first, then a value that satisfies
it) — [`docs::generator_design`](crate::docs::generator_design)
covers the trade-off.

**See also.**
[`sample_with_rejection`](crate::sample_with_rejection),
[`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case),
[`examples/rejection.rs`](https://github.com/sile/noprop/blob/main/examples/rejection.rs).

## Model-based (stateful) property

**Goal.** Drive a model and a system-under-test with the same command
sequence, comparing them at every step.

**Uses.** [`Runner::run`](crate::Runner::run) or
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided),
plus [`sample_usize_in`](crate::sample_usize_in) and the primitives
the commands need.

```
# use noprop::TestCaseContext;
# fn body() -> noprop::TestResult {
noprop::Runner::new(0).run(64, |ctx| {
    let mut model: Vec<u32> = Vec::new();       // reference
    let mut sut: Vec<u32> = Vec::new();         // system under test
    let mut history: Vec<String> = Vec::new();

    let steps = noprop::sample_usize_in(ctx, 0..=16);
    for step in 0..steps {
        let cmd = noprop::sample_usize_in(ctx, 0..2);
        match cmd {
            0 => {
                let v = noprop::sample_u32(ctx);
                history.push(format!("push {v}"));
                model.push(v);
                sut.push(v);
            }
            _ => {
                history.push("pop".to_string());
                let m = model.pop();
                let s = sut.pop();
                assert_eq!(
                    m, s,
                    "step {step}: pop mismatch (model={m:?}, sut={s:?})\n\
                     history: {history:#?}"
                );
            }
        }
    }
    Ok(())
})?;
# Ok(())
# }
# body().unwrap();
```

**Notes.** Failure messages must include enough to reconstruct the
mismatch: the step index, the mismatched values, and either the whole
command history or the abstract model state at the point of failure.
The stateful `examples/stateful.rs` example shows this pattern applied
to a realistic subject (an LRU cache with a plausible-looking bug).

**See also.**
[`examples/stateful.rs`](https://github.com/sile/noprop/blob/main/examples/stateful.rs).

## Steer the search with feedback

**Goal.** Concentrate sampling on inputs that reach a semantic region
of interest, rather than sampling uniformly.

**Uses.**
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided),
[`TestCaseContext::event`](crate::TestCaseContext::event),
[`TestCaseContext::bucket`](crate::TestCaseContext::bucket),
[`TestCaseContext::transition`](crate::TestCaseContext::transition).

```
# fn body() -> noprop::TestResult {
noprop::Runner::new(0xC0FFEE).run_feedback_guided(64, |ctx| {
    let len = noprop::sample_usize_in(ctx, 0..=24);
    let _line = noprop::sample_string(ctx, len);
    // Report a finite occurrence: the search will steer toward the
    // input region that reaches it.
    if len > 12 {
        ctx.event("long-line");
    }
    // Report a state value pre-bucketed by the caller: a raw value
    // that never repeats would defeat the corpus.
    ctx.bucket("len-bucket", (len / 4) as u64);
    // Report an abstract state change: the (from, to) pair is part of
    // the feature identity.
    ctx.transition("ingest", 0, (len % 3) as u64);
    Ok(())
})?;
# Ok(())
# }
# body().unwrap();
```

**Notes.** Feedback is not mandatory — a case that reports no feature
is simply not interesting, and the run continues. Bucket *before*
reporting; a raw value that differs every case (a timestamp, a random
byte count) makes every case novel and defeats the corpus. The
[`docs::feedback_guided_search`](crate::docs::feedback_guided_search)
design doc explains the corpus admission and eviction rules.

**See also.**
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided),
[`docs::feedback_guided_search`](crate::docs::feedback_guided_search),
[`examples/feedback_guided.rs`](https://github.com/sile/noprop/blob/main/examples/feedback_guided.rs).

## Assert a coverage gate after the run

**Goal.** Make "the search must reach this region" a test failure, not
a silent pass.

**Uses.** A `std::cell::Cell` counter, the feedback method that steers
toward the region, and a run-after assert. The `Runner`'s `Display`
embeds the seed and stats so the failure is reproducible.

```
# use std::cell::Cell;
# fn body() -> noprop::TestResult {
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
# Ok(())
# }
# body().unwrap();
```

**Notes.** A time-derived seed makes rare-region gates unstable — the
region may be reached under most seeds but not this one, and the test
is then a flake. Gate on regions the *generator* can reach reliably
(build the generator so the region is inside its support), and keep
the accepted-only count separate from the total attempt count so the
assertion counts what it means.

**See also.**
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided),
[`examples/feedback_guided.rs`](https://github.com/sile/noprop/blob/main/examples/feedback_guided.rs).

## Reproduce a failing seed

**Goal.** Turn a failure report into a repeatable local run.

**Uses.** The failing report itself
([`RunError::seed`](crate::RunError::seed),
[`RunError::case_index`](crate::RunError::case_index),
[`RunError::generated`](crate::RunError::generated)) and the
`reproduce with:` hint.

The failure report prints, on both `Debug` and `Display`:

```text
noprop failure at case 3 (seed=0x00ff00ff00ff00ff): high bit set: 0xd7...
reproduce with: noprop::Runner::new(0x00ff00ff00ff00ff).run(64, |ctx| ...)
stats: accepted=3, rejected=0, total_samples=4, discovered_features=0, max_corpus_size=0
Generated values:
  <call-site>: 0xd7abcdef
```

Recovery is manual and mechanical:

1. Copy the hex seed from the report.
2. Rerun the property with `Runner::new(<seed>)` and the *same* case
   budget printed in the hint. The rerun hits the same case index, so
   the failure surfaces again.
3. Under `run_feedback_guided`, the same seed replays the exact
   sequence of accepted, rejected, and mutated candidates — the
   candidate index in the report identifies which one failed.

**Notes.** The rerun budget must match the original: a smaller budget
can shrink the rejection cap and turn the same failure into
`TooManyRejections`. Keep the property and generator identical between
runs — a change to either shifts the choice sequence for the same
seed.

**See also.**
[`RunError`](crate::RunError),
[`examples/reproduce.rs`](https://github.com/sile/noprop/blob/main/examples/reproduce.rs).

## Turn a trace into a hand-written regression test

**Goal.** Freeze a failure as a small deterministic test after
reproducing it, since v0.1 does not ship automatic shrinking.

The failure trace shows *the actual values* the primitives produced,
labelled with their call sites. Instead of storing the choice sequence
(which noprop deliberately does not expose), pick the smallest hand
witness that still triggers the bug and inline it as a regular
`#[test]`:

```
#[test]
fn commutativity_holds_on_the_shrunk_witness() {
    // Values pulled from the failure trace of case 3 (seed 0x00...ff).
    let a: u32 = 0xd7ab_cdef;
    let b: u32 = 0;
    assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
}
```

**Notes.** The witness should be minimal in whatever dimension the
property cares about (length, value magnitude, edge-case discriminant),
not in the raw RNG bytes. The trace tells you what the property saw;
manual simplification tells you what actually matters. Keep the seeded
property test as well — the regression test guards the specific case,
and the property test keeps looking for related ones.

**See also.**
[`RunError::generated`](crate::RunError::generated),
[`docs::generator_design`](crate::docs::generator_design).

## Observe cross-case state with interior mutability

**Goal.** Count a rare event, capture a sample of failing inputs, or
build any aggregate that spans multiple cases — despite the property
closure being `Fn`, not `FnMut`.

**Uses.** `std::cell::Cell` (for `Copy` counters) or `std::cell::RefCell`
(for non-`Copy` accumulators).

```
# use std::cell::Cell;
# fn body() -> noprop::TestResult {
let long_names: Cell<usize> = Cell::new(0);
noprop::Runner::new(0).run(64, |ctx| {
    let name = noprop::sample_string(ctx, 16);
    if name.chars().count() > 12 {
        long_names.set(long_names.get() + 1);
    }
    Ok(())
})?;
let _ = long_names.get();
# Ok(())
# }
# body().unwrap();
```

**Notes.** Most properties need no shared state at all — everything
belongs in local variables inside the closure. Reach for interior
mutability only when a cross-case observation is genuinely part of the
test (a coverage gate, a debug report, a threshold sanity check).
`RefCell` panics on borrow conflicts; keep the borrow scope minimal.

**See also.**
[`examples/basics.rs`](https://github.com/sile/noprop/blob/main/examples/basics.rs)
("Pitfall 1: the closure is `Fn`, not `FnMut`").

## Keep the trace pointing at the user's call site

**Goal.** When wrapping a primitive in a user-defined `sample_*`
helper, keep the trace pointing at the caller, not at the wrapper's
body.

**Uses.** `#[track_caller]` on the wrapper.

```
use std::num::NonZeroU32;
use noprop::TestCaseContext;

#[track_caller]
fn sample_non_zero_u32(ctx: &mut TestCaseContext) -> NonZeroU32 {
    let v = noprop::sample_u32(ctx);
    NonZeroU32::new(if v == 0 { 1 } else { v })
        .expect("v was remapped away from zero")
}

let mut ctx = TestCaseContext::new(0);
let _n = sample_non_zero_u32(&mut ctx);
```

**Notes.** Without `#[track_caller]` the trace shows the wrapper's
source line, which is the same for every call site — that defeats the
purpose of the trace. Add it to every user-defined `sample_*` helper
so a failure trace stays actionable.

**See also.** The `Location` behavior of
[`sample_u32`](crate::sample_u32) and other primitives, which use
`#[track_caller]` for the same reason.
