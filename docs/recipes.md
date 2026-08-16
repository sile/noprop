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
- [Adjust case budget for a campaign run](#adjust-case-budget-for-a-campaign-run)
- [Sample primitives, ranges, and strings](#sample-primitives-ranges-and-strings)
- [Build `Vec`, `Option`, maps, and sets](#build-vec-option-maps-and-sets)
- [Pick between enum variants and weight the choice](#pick-between-enum-variants-and-weight-the-choice)
- [Mix in domain boundaries with an exact probability](#mix-in-domain-boundaries-with-an-exact-probability)
- [Sample `NonZero` integers](#sample-nonzero-integers)
- [Write a dependent generator](#write-a-dependent-generator)
- [Bounded recursion and bounded loops](#bounded-recursion-and-bounded-loops)
- [Choose a rejection scope](#choose-a-rejection-scope)
- [Model-based (stateful) property](#model-based-stateful-property)
- [Cluster-level invariant across multiple actors](#cluster-level-invariant-across-multiple-actors)
- [Bounded run-to-quiescence](#bounded-run-to-quiescence)
- [Cross-step invariant with append-only history](#cross-step-invariant-with-append-only-history)
- [Stateful streaming API driven by a command loop](#stateful-streaming-api-driven-by-a-command-loop)
- [Round-trip a value through configurable text](#round-trip-a-value-through-configurable-text)
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

```rust
# fn body() -> noprop::TestResult {
// A fixed seed keeps the run repeatable.
noprop::Runner::new(0xDEAD_BEEF).run(256, |ctx| {
    let x = noprop::sample_u32(ctx);
    assert_eq!(x, x);
    Ok(())
})?;

// An env-controlled seed lets a failure report be replayed by
// setting the variable. The env helper accepts decimal and
// `0x`-prefixed hex with optional `_` separators, so the hex seed
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

## Adjust case budget for a campaign run

**Goal.** Let each property keep its own default case budget for
normal `cargo test`, and give an occasional wide-search campaign
(nightly run, weekly nightly, hand-triggered exploration) a single
knob to raise every property's budget at once — without inventing
a crate-wide helper that would suggest one budget fits every
property.

**Uses.** [`Runner::run`](crate::Runner::run), a project-local env
parser, and per-property `const` defaults. noprop itself never
reads the process environment for the case budget; the helper below
lives in the consuming project's test harness.

```rust
use std::env;
use std::num::NonZeroUsize;

/// Read a project-specific env var and return the case budget it
/// asks for, or fall back to the property's own default.
///
/// Behavior by input:
/// - variable unset (or value is not valid UTF-8) → `default`
/// - positive `usize` → that value
/// - `0`, negative, or non-integer → immediate `panic!` with the
///   raw text, so a mistyped `MYAPP_CASES=hello` fails at the
///   start of the run instead of silently reverting to `default`.
fn case_budget(var: &str, default: usize) -> usize {
    let Ok(s) = env::var(var) else { return default };
    let value: NonZeroUsize = s.parse().unwrap_or_else(|e| {
        panic!("{var}={s:?} is not a positive integer (unset it or pick a positive case budget): {e}")
    });
    value.get()
}

/// Baseline budget for a cheap scalar property. Deliberately small
/// so a normal `cargo test` stays fast; a campaign raises this via
/// `MYAPP_CASES` without touching the source.
const CHEAP_CASES: usize = 16;

# fn body() -> noprop::TestResult {
noprop::Runner::new(0xDEAD_BEEF)
    .run(case_budget("MYAPP_CASES", CHEAP_CASES), |ctx| {
        let a = noprop::sample_u32(ctx);
        let b = noprop::sample_u32(ctx);
        assert_eq!(a.wrapping_add(b), b.wrapping_add(a));
        Ok(())
    })?;
# Ok(())
# }
# body().unwrap();
```

A stateful property whose one case runs a bounded command loop
against a model costs orders of magnitude more per case than the
cheap scalar above. Keep that property's own `const` at, say, 128 so
a normal `cargo test` still finishes, and let the same campaign env
raise it:

```text
const STATEFUL_CASES: usize = 128;

noprop::Runner::new(seed)
    .run(case_budget("MYAPP_CASES", STATEFUL_CASES), |ctx| { /* ... */ })?;
```

Both properties read the same `MYAPP_CASES`; unset, each uses its
own default. `MYAPP_CASES=10000 cargo test` raises both to 10 000.
The per-property scale stays intact while one campaign knob
multiplies everything at once.

**Notes.**

*Reach for this recipe only when a campaign exists.* Do not wire
`case_budget(...)` into a property just because the pattern is
documented. Each property's `const` default is its contract with a
normal `cargo test` — keep it there until an actual wide-search
campaign (nightly, weekly, hand-triggered) needs a per-run
override. If a property is *always* under-budgeted for normal
runs, raise its `const` and commit that; do not paper over it with
a default env value the whole team is expected to set. And do not
fan out into per-property env vars (`MYAPP_STATEFUL_CASES`,
`MYAPP_QUICK_CASES`, …) — the point of the pattern is one env var
multiplying every property, so the per-property scale stays as the
`const`s. Raising only one property's budget is a `const` bump,
not a new env var.

*The env var name is a placeholder.* Pick a name that fits the
consuming project (`MYAPP_CASES` here matches the `MYAPP_SEED`
placeholder used by "Run a property with a seed" above). The
minimum, the maximum, and how CI sets it are the project's call,
not noprop's.

*A crate-wide `cases_from_env` is deliberately not provided.* The
right case budget depends on how large a property's search space
is and how expensive one of its cases is — both are per-property
judgments. A crate helper would push the shape "one number fits
everything", which is exactly what the per-property `const`
defaults are set up to avoid. [`seed_from_env_or_time`](crate::seed_from_env_or_time)
is in the crate because the failure report prints the seed as
`{:#018x}` hex for direct copy-paste reuse — that reproduction
path is a first-class contract. There is no equivalent reproduction
path for the case budget: the failure hint already prints the
run's original `N` in the `reproduce with:` line, and reading the
budget from an env var would set up a drift between "whatever the
hint printed" and "whatever the env is when the reproducer runs
it."

*Fix the generator before raising the budget.* A property that
misses its target with probability `(1 - p)^N` shrinks by only `p`
in the exponent per extra case; a small `p` needs many more cases
to compensate. When a coverage gate or an assertion asks for more
cases, first walk through the checks in the "Assert a coverage
gate after the run" recipe: is the target class in support at all,
is it a first-class branch, does it have an explicit weight or
[`Ratio`](crate::Ratio)? Raising `N` is the last lever, not the
first.

*Reproduction reads the hint, not the env.* When a failure fires
mid-campaign, take the case budget from the `reproduce with:` line
of the failure report and pass it explicitly, not from the current
env value. The hint records what the failing run actually used; the
env value can change between then and now.

**See also.** The "Run a property with a seed" recipe (env-controlled
seed via [`seed_from_env_or_time`](crate::seed_from_env_or_time),
same `MYAPP_*` placeholder convention), the "Reproduce a failing
seed" recipe (why the hint's `N` is authoritative for reproduction),
the "Assert a coverage gate after the run" recipe (the "first fix
the generator, then raise `N`" decision order).

## Sample primitives, ranges, and strings

**Goal.** Draw the standard scalar and collection primitives that most
properties need.

**Uses.** [`sample_u32`](crate::sample_u32) (and the other integer
`sample_*` primitives), [`sample_usize_in`](crate::sample_usize_in),
[`sample_u64_in`](crate::sample_u64_in),
[`sample_string`](crate::sample_string),
[`sample_ascii_printable_string`](crate::sample_ascii_printable_string),
[`sample_bool`](crate::sample_bool).

```rust
let mut ctx = noprop::TestCaseContext::new(0);

// Full-width integers.
let _n: u32 = noprop::sample_u32(&mut ctx);

// Bounded range: uniform, bias-free, no `% N` overflow at usize::MAX.
let idx = noprop::sample_usize_in(&mut ctx, 0..10);
assert!(idx < 10);
let day = noprop::sample_usize_in(&mut ctx, 1..=31);
assert!((1..=31).contains(&day));

// u64 range: no `as u64` cast or bit mask needed (on 32-bit targets
// the range does not even fit in usize).
let ts = noprop::sample_u64_in(&mut ctx, 0..(1u64 << 33));
assert!(ts < (1u64 << 33));

// Length-taking string primitives: pick the length first, then draw.
let len = noprop::sample_usize_in(&mut ctx, 0..=32);
let _s = noprop::sample_string(&mut ctx, len);
let _ascii = noprop::sample_ascii_printable_string(&mut ctx, len);

let _b = noprop::sample_bool(&mut ctx);
```

**Notes.** Never write `sample_usize(ctx) % max` for a bounded draw —
it is biased and overflows at `usize::MAX`. For `u64` ranges use
[`sample_u64_in`](crate::sample_u64_in) instead of masking with
`& ((1 << n) - 1)`, which can only express zero-offset power-of-two
ranges and is biased for any other width.
Length-taking helpers take an *exact* length; combine with
`sample_usize_in` for a random length (no `(min, max)` overload is
provided so composition stays the one shape).

**See also.** [`sample_usize_in`](crate::sample_usize_in),
[`sample_u64_in`](crate::sample_u64_in),
[`sample_string`](crate::sample_string),
[`crate::docs::generator_authoring`].

## Build `Vec`, `Option`, maps, and sets

**Goal.** Assemble collections with ordinary Rust control flow, without
combinator DSLs.

**Uses.** [`sample_usize_in`](crate::sample_usize_in),
[`sample_bool`](crate::sample_bool),
[`sample_u32`](crate::sample_u32),
[`sample_string`](crate::sample_string).

```rust
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

```rust
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

```rust
let mut ctx = noprop::TestCaseContext::new(0);
let port = noprop::sample_with_boundaries(
    &mut ctx,
    &[0u16, 1500, u16::MAX],
    noprop::Ratio::one_nth(10),
    noprop::sample_u16,
);
// 10% of the time `port` is one of {0, 1500, u16::MAX};
// otherwise a uniform u16.
let _ = port;
```

**Notes.** The probability is exact rational (`Ratio::one_nth(10)` is
one-in-ten, not `0.10`-close). Choose boundaries that map to distinct
outcomes in the code under test — otherwise the extra probability
mass on the boundary set does nothing observable.

**See also.**
[`sample_with_boundaries`](crate::sample_with_boundaries),
[`Ratio`](crate::Ratio),
[`examples/search_space.rs`](https://github.com/sile/noprop/blob/main/examples/search_space.rs).

## Sample `NonZero` integers

**Goal.** Produce a `NonZero<_>` value from a plain integer primitive
without shipping a dedicated helper.

**Uses.**
[`sample_with_rejection`](crate::sample_with_rejection) (uniform),
[`sample_u32`](crate::sample_u32) + explicit remap (biased).

```rust
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

**See also.** The "Sampling non-zero integers" section of
[`crate::docs::generator_authoring`].

## Write a dependent generator

**Goal.** Let a later draw's domain depend on an earlier draw.

**Uses.** Whatever primitives the domain needs — the pattern is
"draw, then branch".

```rust
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
generators" section of [`crate::docs::generator_authoring`].

## Bounded recursion and bounded loops

**Goal.** Generate a tree, a nested value, or a command sequence
without risking unbounded work per case.

**Uses.** [`sample_usize_in`](crate::sample_usize_in), an explicit
depth or count parameter.

```rust
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

**Notes.** Any recursion or loop needs a decreasing budget: `Runner::run`
does not enforce a per-case draw cap, so a hostile choice sequence could
otherwise keep the case running without termination. The `for _ in 0..n`
shape is enough for a command sequence; a `while` loop needs an explicit
maximum step count.

**See also.** [`sample_usize_in`](crate::sample_usize_in),
[`sample_bool`](crate::sample_bool).

## Choose a rejection scope

**Goal.** Redraw either a single value or the whole case, depending on
where the constraint sits.

**Uses.** [`sample_with_rejection`](crate::sample_with_rejection) (per
draw), [`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case)
(per case). Prefer *valid-by-construction* over either when
practical.

```rust
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
[`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case).

## Model-based (stateful) property

**Goal.** Drive a model and a system-under-test with the same command
sequence, comparing them at every step.

**Uses.** [`Runner::run`](crate::Runner::run), plus
[`sample_usize_in`](crate::sample_usize_in) and the primitives the
commands need.

```rust
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
to a bounded FIFO queue with state-dependent command selection and a
non-mutating per-transition state comparison.

When an invariant fires only after the state reaches a specific shape
(a non-empty pop, a committed entry, a fully drained buffer), a case
that ends before that shape leaves the invariant unchecked and the run
passes silently. Gate such invariants with the "Assert a coverage gate
after the run" recipe below — a `Cell` counter incremented at the
invariant site plus a run-after assert turns silent success into a
failure.

**See also.** The "Assert a coverage gate after the run" recipe
(guarding a state-dependent invariant against silent success),
the "Cluster-level invariant across multiple actors" recipe
(the multi-SUT extension), the "Bounded run-to-quiescence" recipe
(finalising a converging protocol), the "Cross-step invariant with
append-only history" recipe (tracking a history alongside the SUT),
the "Stateful streaming API driven by a command loop" recipe
(the model-free variant),
[`examples/stateful.rs`](https://github.com/sile/noprop/blob/main/examples/stateful.rs).
For the assertion-message shape a stateful failure needs (step index,
command, model / SUT state, bounded command history), see the
"Semantic assertion patterns → Stateful command loops" section of
[`skills/noprop/references/failure-diagnostics.md`](https://github.com/sile/noprop/blob/main/skills/noprop/references/failure-diagnostics.md).

## Cluster-level invariant across multiple actors

**Goal.** Drive several instances of the same SUT together and assert
a system-level invariant (broadcast reach, consensus, distributed
state agreement) instead of a single-actor property.

**Uses.** A `Vec<SUT>` (or similar) that holds every actor in one
case, plus [`sample_usize_in`](crate::sample_usize_in) to pick which
actor a step touches. Per-step advances the whole cluster and checks
the invariant.

```rust
# fn body() -> noprop::TestResult {
noprop::Runner::new(0).run(64, |ctx| {
    let n = noprop::sample_usize_in(ctx, 2..=5);
    // One "node" per actor; the cluster is the Vec.
    let mut nodes: Vec<u32> = vec![0; n];

    let broadcasts = noprop::sample_usize_in(ctx, 0..=8);
    for round in 0..broadcasts {
        let v = noprop::sample_u32(ctx);
        // Advance every node together (an atomic broadcast, as a
        // stand-in for whatever the real protocol does per step).
        for node in nodes.iter_mut() {
            *node = v;
        }
        // System-level invariant: after each broadcast every node
        // agrees on the value.
        assert!(
            nodes.iter().all(|&x| x == v),
            "round {round}: nodes disagree ({nodes:?})"
        );
    }
    Ok(())
})?;
# Ok(())
# }
# body().unwrap();
```

**Notes.** Keep the actor type simple (a plain integer, a `Vec<u8>`
buffer) so the recipe stays focused on the cluster-level shape rather
than on any specific protocol. Real protocols usually deliver messages
asynchronously — pair this recipe with the "Bounded run-to-quiescence"
recipe below to drain a message queue before checking the invariant,
and with the "Cross-step invariant with append-only history" recipe
when a per-step invariant needs to observe how state evolved over
time. Gossip protocols, broadcast trees, consensus algorithms (leader
election, log replication), replicated state stores (CRDTs), and
other peer-to-peer or clustered systems typically follow this shape.

**See also.** The "Model-based (stateful) property" recipe (the
single-actor baseline), the "Bounded run-to-quiescence" recipe below,
the "Cross-step invariant with append-only history" recipe (per-step
invariants over a persistent log), the "Stateful streaming API driven
by a command loop" recipe (model-free streaming variant).

## Bounded run-to-quiescence

**Goal.** Drive a protocol until it settles, but never longer than a
declared bound — the run terminates for every seed, and a run that
would have looped forever fails with a clear message.

**Uses.** A `for _ in 0..max_rounds` loop with a `did_something` flag,
an early `break` when nothing changes, and a final assert that the
loop exited through the quiescence branch rather than the bound.

```rust
# fn body() -> noprop::TestResult {
noprop::Runner::new(0).run(64, |ctx| {
    let n = noprop::sample_usize_in(ctx, 2..=5);
    // Toy cluster: each slot is "pending" and clears itself when
    // processed. A real protocol would enqueue / deliver messages.
    let mut pending: Vec<bool> = (0..n).map(|_| noprop::sample_bool(ctx)).collect();

    let max_rounds = 16;
    let mut converged = false;
    for _round in 0..max_rounds {
        let mut did_something = false;
        for slot in pending.iter_mut() {
            if *slot {
                *slot = false;
                did_something = true;
            }
        }
        if !did_something {
            converged = true;
            break;
        }
    }
    // The bound is a safety net; the tiny protocol above must settle
    // well within max_rounds. A run that fails this assert has hit
    // either an infinite loop or a bound that is too tight.
    assert!(
        converged,
        "did not reach quiescence within {max_rounds} rounds"
    );
    Ok(())
})?;
# Ok(())
# }
# body().unwrap();
```

**Notes.** Pick `max_rounds` well above the worst-case round count
the protocol needs — the point of the bound is to catch runaway
loops, not to prune legitimate long convergences. Report the state
that failed to settle in the assert message so the failure is
diagnosable without a rerun.

**See also.** The "Model-based (stateful) property" recipe (the
single-actor baseline), the "Cluster-level invariant across multiple
actors" recipe above (the multi-SUT shape this recipe finalises),
the "Cross-step invariant with append-only history" recipe (tracking
what changed at each round), the "Stateful streaming API driven by a
command loop" recipe (a different way to bound a long-running SUT).

## Cross-step invariant with append-only history

**Goal.** Check an invariant that spans time — "once committed, an
entry never changes" — by keeping an append-only history next to the
SUT and re-checking it at every step.

**Uses.** A local `BTreeMap` (or similar) that mirrors the
append-only view of the SUT, updated inside the step loop, and a
per-step assert that every previously recorded entry is still
present unchanged.

```rust
# use std::collections::BTreeMap;
# fn body() -> noprop::TestResult {
noprop::Runner::new(0).run(64, |ctx| {
    // history: seq -> value. Append-only: once written, never
    // changed. Kept in the closure alongside the SUT so the invariant
    // can look back at earlier steps.
    let mut history: BTreeMap<u64, u32> = BTreeMap::new();
    let mut next_seq: u64 = 0;
    let mut current_value: u32 = 1;

    let steps = noprop::sample_usize_in(ctx, 0..=16);
    for _ in 0..steps {
        match noprop::sample_usize_in(ctx, 0..3) {
            0 => {
                // Append the next entry at the current value.
                history.insert(next_seq, current_value);
                next_seq += 1;
            }
            1 => {
                // Rotate the value for the next append (no append this step).
                current_value += 1;
            }
            _ => {
                // No-op step — invariant still runs below.
            }
        }
        // Cross-step invariant: every previously appended (seq, value)
        // pair is still present unchanged. A bug that silently
        // rewrote an entry would fail here on the very next step, not
        // at the end of the run.
        for (&seq, &value) in &history {
            assert_eq!(
                history.get(&seq),
                Some(&value),
                "entry at {seq} changed"
            );
        }
    }
    Ok(())
})?;
# Ok(())
# }
# body().unwrap();
```

**Notes.** The history lives inside the closure, alongside the SUT —
not as a second SUT to compare against. Re-checking every recorded
entry at every step is quadratic in the number of appended entries,
which is fine for the typical case counts of a property test
(`steps` bounded, `history` bounded by `steps`); for a very long
history keep only the entries the invariant actually needs. Because
the invariant only fires when history is non-empty, gate the run
with the "Assert a coverage gate after the run" recipe to make sure
at least one case reached an appended entry — otherwise the run may
silently pass on cases where nothing ever got appended.

The "once appended, never rewritten" invariant appears wherever a
system keeps an append-only log: event sourcing / audit logs,
write-ahead and commit logs (Kafka, PostgreSQL WAL), version control
(a git commit hash never changes), LSM-tree SSTables and immutable
memtables, CRDT operation logs, blockchain blocks, and consensus
committed logs (Raft's `committed_history` is a common example).
The recipe applies unchanged; only the entry type and the meaning of
"an append" branch differ.

**See also.** The "Assert a coverage gate after the run" recipe
(force a run to fail when history stays empty), the "Model-based
(stateful) property" recipe (the single-actor baseline), the
"Cluster-level invariant across multiple actors" recipe (multi-SUT
extension), the "Bounded run-to-quiescence" recipe (bounding step
count for a settling protocol), the "Stateful streaming API driven
by a command loop" recipe (model-free variant). For picking the
bounded suffix of history to include in the assertion message on
failure, see the "Semantic assertion patterns → Stateful command
loops" section of
[`skills/noprop/references/failure-diagnostics.md`](https://github.com/sile/noprop/blob/main/skills/noprop/references/failure-diagnostics.md).

## Stateful streaming API driven by a command loop

**Goal.** Test a streaming API — one that does not have a single
"correct model" to compare against, but that buffers writes and
emits committed output only at flush boundaries — by driving it
with a random command loop. Assert either the final round-trip
after `finish` (when only the eventual output shape matters), or
a *cumulative* invariant at each flush boundary (when flush is a
public contract of the SUT and per-boundary drift would otherwise
hide behind the final compare).

**Uses.** A random command loop over feed / flush, a single
`finish` outside the loop, a cumulative model of the bytes fed so
far, and (for the cumulative variant) a coverage gate — a
[`Cell<usize>`](std::cell::Cell) plus a case-internal `bool`
flag — that fails the run if no case ever reached a meaningful
flush boundary.

Both examples below share the same toy SUT. `BufferedSink` holds
writes in `pending`, moves them into `emitted` on `flush()`, and
`observed()` returns the running total of *committed* bytes. A
`flush()` with nothing pending is a no-op — no bytes move, no
delta is added — which is the shape a real streaming API must
tolerate on consecutive flushes.

```rust
use std::cell::Cell;

#[derive(Default)]
struct BufferedSink {
    pending: Vec<u8>,
    emitted: Vec<u8>,
}

impl BufferedSink {
    fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }
    fn flush(&mut self) {
        // Move pending → emitted, leaving pending empty. A second
        // flush with nothing pending moves zero bytes.
        self.emitted.append(&mut self.pending);
    }
    fn observed(&self) -> &[u8] {
        &self.emitted
    }
    fn finish(mut self) -> Vec<u8> {
        self.flush();
        self.emitted
    }
}

# fn body() -> noprop::TestResult {
// === Final-only variant ===
//
// Only the round-trip after finish() is checked. Reach for this
// when flush is not part of the SUT's public contract (a plain
// Write-wrapping buffer that only guarantees byte order at drop)
// or when only the eventual output shape matters.
noprop::Runner::new(0).run(64, |ctx| {
    let mut sink = BufferedSink::default();
    let mut fed: Vec<u8> = Vec::new();
    let steps = noprop::sample_usize_in(ctx, 0..=8);
    for _ in 0..steps {
        match noprop::sample_usize_in(ctx, 0..2) {
            0 => {
                let n = noprop::sample_usize_in(ctx, 0..=4);
                let bytes = noprop::sample_bytes_vec(ctx, n);
                fed.extend_from_slice(&bytes);
                sink.feed(&bytes);
            }
            _ => sink.flush(),
        }
    }
    let output = sink.finish();
    assert_eq!(output, fed, "final round-trip mismatch");
    Ok(())
})?;

// === Cumulative variant ===
//
// Compare observed() against fed at every flush, and gate the run
// on at least one case actually reaching a flush that committed a
// non-empty delta. A naive "delta of observed matches delta of
// fed" check would silently pass on empty consecutive flushes;
// comparing the running totals absorbs those cases correctly, and
// the gate keeps the run from passing trivially when no flush
// ever moved bytes.
let meaningful: Cell<usize> = Cell::new(0);
let mut runner = noprop::Runner::new(0);
runner.run(64, |ctx| {
    let mut sink = BufferedSink::default();
    let mut fed: Vec<u8> = Vec::new();
    let mut saw_meaningful_flush = false;
    let steps = noprop::sample_usize_in(ctx, 0..=8);
    for _ in 0..steps {
        match noprop::sample_usize_in(ctx, 0..2) {
            0 => {
                let n = noprop::sample_usize_in(ctx, 0..=4);
                let bytes = noprop::sample_bytes_vec(ctx, n);
                fed.extend_from_slice(&bytes);
                sink.feed(&bytes);
            }
            _ => {
                let before = sink.observed().len();
                sink.flush();
                let added = sink.observed().len() - before;
                // Cumulative compare at every flush, including
                // consecutive no-op flushes (added == 0 and the
                // running totals trivially agree). The gate only
                // records the case when a flush moved bytes.
                assert_eq!(
                    sink.observed(),
                    fed.as_slice(),
                    "cumulative flush mismatch (delta={added})",
                );
                if added > 0 {
                    saw_meaningful_flush = true;
                }
            }
        }
    }
    if saw_meaningful_flush {
        meaningful.set(meaningful.get() + 1);
    }
    // finish() must still round-trip the whole feed.
    let output = sink.finish();
    assert_eq!(output, fed, "final round-trip mismatch");
    Ok(())
})?;
assert!(
    meaningful.get() > 0,
    "no case executed a meaningful cumulative comparison\n{runner}"
);
# Ok(())
# }
# body().unwrap();
```

**Notes.**

*Cumulative vs delta at flush boundaries.* Compare `observed()` —
the SUT's running total of committed bytes — against the running
total of `fed`. Do not compare a delta of `observed()` between
two flushes against a delta of `fed` since the last flush: an
empty consecutive flush would compare `0 == 0` and pass regardless
of whether the SUT ever committed anything, and a delta-driven
invariant "the SUT emitted N bytes this flush, so the model must
have fed exactly N bytes since the last one" breaks for any SUT
that buffers input across boundaries before committing.

*Consecutive no-op flushes.* Keep the empty-flush case in the
command support. The cumulative compare passes on such a flush
(the emitted length did not change), which is the correct
behavior — no bytes moved, so no new agreement was required. The
gate below explicitly excludes them from the meaningful count.

*Gate on meaningful flushes, not on flush selection.* The gate
answers "did at least one case exercise a flush that committed
bytes?" — not "did any case ever call `flush()`", which any run
with more than a handful of steps trivially satisfies. Flip a
case-internal `bool` when a flush's delta is non-empty, then bump
the [`Cell<usize>`](std::cell::Cell) once at the end of the case
(matching the "Assert a coverage gate after the run" recipe's
per-case pattern). Include `{runner}` in the run-after assertion
so the seed and stats reach the failure report.

*The bounded loop terminates by construction.* `for _ in 0..steps`
with `steps` drawn from `sample_usize_in(ctx, 0..=8)` cannot loop
forever; there is no separate assertion for "the loop finished".
The final `assert_eq!` after `finish()` covers the last
cumulative agreement.

*Choose the variant by what the SUT contracts.* The final-only
variant is the minimum — reach for it when flush is an internal
optimisation the caller does not observe. Reach for the
cumulative variant when flush is part of the public contract and
the SUT is expected to commit bytes at that boundary (a streaming
compressor's `Z_SYNC_FLUSH`, an incremental hasher whose
intermediate `finalize_reset` returns a stable digest, a TLS
record layer whose `send_close_notify` must emit before the
socket is torn down). The two-stage shape — random command loop +
one `finish` outside the loop — matters in both cases: an API
that requires `finish` to emit its trailing bytes will fail the
round-trip if the terminator moves into the loop.

Streaming compressors and encoders (deflate, gzip), incremental
hashers (`update` / `finalize`), buffered writers, and network
protocol codecs that feed bytes in and emit framed messages
(HTTP, TLS record layer, WebSocket, RTMP) typically follow this
shape.

**See also.** The "Model-based (stateful) property" recipe (the
model-driven counterpart), the "Cluster-level invariant across
multiple actors" recipe (multi-SUT extension), the "Bounded
run-to-quiescence" recipe (bounding a settling protocol), the
"Cross-step invariant with append-only history" recipe (recipes
that keep per-step state alongside the SUT), and the "Assert a
coverage gate after the run" recipe (the `Cell<usize>` +
case-internal flag + `{runner}` pattern the cumulative variant
reuses). For the bounded metrics and state snapshot to include in
the assertion message on failure (queue length, cumulative bytes,
ordered event suffix), see the "Semantic assertion patterns →
Streaming and simulation" section of
[`skills/noprop/references/failure-diagnostics.md`](https://github.com/sile/noprop/blob/main/skills/noprop/references/failure-diagnostics.md).

## Round-trip a value through configurable text

**Goal.** Verify that a value → text → value round-trip agrees
for every combination of the SUT's textual output settings —
quote style, delimiter, escape shape — instead of only the
default settings. Every setting change is a fresh chance for the
serializer's escape rules and the parser's tokenizer to drift;
drawing the settings alongside the value forces both directions
to stay consistent across the whole configuration surface.

**Uses.** [`Runner::run`](crate::Runner::run),
[`sample_bool`](crate::sample_bool),
[`sample_usize_in`](crate::sample_usize_in),
[`sample_choice`](crate::sample_choice), one
[`Cell<usize>`](std::cell::Cell) per output-setting class, and a
toy serializer / parser pair defined inside the doctest.

The toy grammar below is a self-contained placeholder — a real
SUT has its own value type, output-setting axes, and grammar;
keep the recipe shape and swap those for what the SUT actually
uses.

```rust
use std::cell::Cell;

// Semantic value: a list of arbitrary Rust strings.
// Output-setting axes:
//   quote     ∈ { '"', '\'' }
//   delimiter ∈ { ',', ';' }
// = 4 classes. Grammar: each element is enclosed in `quote`,
// separated by `delimiter`. Inside an element, only `quote` and
// `\` are escaped as `\<char>`; the delimiter needs no escape
// because the quotes already protect it.

fn serialize(items: &[String], quote: char, delimiter: char) -> String {
    let mut out = String::new();
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            out.push(delimiter);
        }
        out.push(quote);
        for c in s.chars() {
            if c == quote || c == '\\' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push(quote);
    }
    out
}

fn parse(text: &str, quote: char, delimiter: char) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    let mut chars = text.chars();
    loop {
        if !items.is_empty() {
            match chars.next() {
                Some(c) if c == delimiter => {}
                None => break,
                Some(c) => panic!("expected delimiter, got {c:?}"),
            }
        }
        match chars.next() {
            Some(c) if c == quote => {}
            None if items.is_empty() => break, // empty text = empty list
            Some(c) => panic!("expected opening quote, got {c:?}"),
            None => panic!("unterminated after delimiter"),
        }
        let mut elem = String::new();
        loop {
            match chars.next() {
                Some('\\') => elem.push(chars.next().expect("dangling escape")),
                Some(c) if c == quote => break,
                Some(c) => elem.push(c),
                None => panic!("unterminated element"),
            }
        }
        items.push(elem);
    }
    items
}

// Element character pool. Deliberately mixes:
//   - plain ASCII (`a`, `b`, `c`);
//   - the backslash and every candidate quote / delimiter, so
//     element strings actually stress the escape rules;
//   - a control character (`\n`);
//   - a non-ASCII scalar (`あ`).
// A short pool that covers every escape class in every case beats
// sample_string over the full Unicode range, which almost never
// draws these specific characters.
const POOL: [char; 10] = ['a', 'b', 'c', '\\', '"', '\'', ',', ';', '\n', 'あ'];

# fn body() -> noprop::TestResult {
// One gate per (quote, delimiter) class. A case is meaningful for
// its class only after the round-trip has actually passed for
// that class in that case, so each Cell lives outside the closure
// and is bumped from the assertion site (per case, at most once).
let gate_dq_comma: Cell<usize> = Cell::new(0);
let gate_dq_semi: Cell<usize> = Cell::new(0);
let gate_sq_comma: Cell<usize> = Cell::new(0);
let gate_sq_semi: Cell<usize> = Cell::new(0);

let mut runner = noprop::Runner::new(0);
runner.run(64, |ctx| {
    // Draw the output setting alongside the value: a single seed
    // then carries the setting into the reproduce hint.
    let quote = if noprop::sample_bool(ctx) { '"' } else { '\'' };
    let delimiter = if noprop::sample_bool(ctx) { ',' } else { ';' };

    // Build a small list of strings from the mixed pool.
    let n = noprop::sample_usize_in(ctx, 0..=3);
    let items: Vec<String> = (0..n)
        .map(|_| {
            let len = noprop::sample_usize_in(ctx, 0..=6);
            (0..len)
                .map(|_| noprop::sample_choice(ctx, &POOL))
                .collect()
        })
        .collect();

    // value → text → value.
    let text = serialize(&items, quote, delimiter);
    let parsed = parse(&text, quote, delimiter);
    assert_eq!(
        parsed, items,
        "round-trip mismatch (quote={quote:?}, delimiter={delimiter:?}): text = {text:?}",
    );

    // Gate: record the class only after the round-trip actually
    // agreed. One case exercises exactly one (quote, delimiter)
    // class, so pick the right cell and bump it once.
    let gate = match (quote, delimiter) {
        ('"', ',') => &gate_dq_comma,
        ('"', ';') => &gate_dq_semi,
        ('\'', ',') => &gate_sq_comma,
        _ => &gate_sq_semi,
    };
    gate.set(gate.get() + 1);
    Ok(())
})?;

// Assert each class independently so a failure names which
// (quote, delimiter) never got exercised.
assert!(
    gate_dq_comma.get() > 0,
    "no case exercised (quote='\"', delimiter=',')\n{runner}",
);
assert!(
    gate_dq_semi.get() > 0,
    "no case exercised (quote='\"', delimiter=';')\n{runner}",
);
assert!(
    gate_sq_comma.get() > 0,
    "no case exercised (quote='\\'', delimiter=',')\n{runner}",
);
assert!(
    gate_sq_semi.get() > 0,
    "no case exercised (quote='\\'', delimiter=';')\n{runner}",
);
# Ok(())
# }
# body().unwrap();
```

**Notes.**

*Draw the output setting alongside the value.* The whole point
of this recipe is that the SUT's serializer and parser can
disagree under one setting while still agreeing under the
default. Drawing `quote` and `delimiter` inside the closure —
instead of running one big loop per setting — lets a single
failing seed carry both the setting and the value into the
reproduce hint, and lets the coverage gate report exactly which
class was never observed.

*Do not reuse Rust's `Debug` output as the target grammar.*
`format!("{s:?}")` emits Rust literal escapes (`\n`, `\u{XX}`,
`\"`, …) that almost never match the target language's own
escape grammar. Write the target grammar's serializer
explicitly, even when it "looks like `Debug`". The one place
`Debug` belongs is inside the failure message (the
`text = {text:?}` above), which is Rust code reading the string
for a human.

*Pool the pathological characters instead of drawing from the
full Unicode range.* [`sample_string`](crate::sample_string)
draws from ~1.1 M code points, which almost never hits the six
or so characters that stress escape and delimiter handling. A
small explicit pool that includes the backslash, every candidate
quote and delimiter, at least one control character, and at
least one non-ASCII scalar reaches every escape class in every
case with just a few draws — see the "Mix in domain boundaries
with an exact probability" recipe for the general principle.
Extend the pool when the SUT's grammar has more escape classes;
do not remove characters just to make the toy pass.

*Gate at the pass site, not at the draw site.* Move the counter
increment to after the round-trip has already succeeded for that
class. A case whose serializer panics or whose parse fails must
not count as evidence that its class was exercised; otherwise a
mutant that breaks one class only will still satisfy the gate.
The `Cell<usize>` + per-case-max-1 pattern matches the "Assert a
coverage gate after the run" recipe.

*Variable-length escapes need a bound.* This grammar uses only
single-character escapes (`\<char>`). A grammar with variable-
length escapes — `\xNN`, `\u{X…}`, `\NNN` — must either fix a
maximum length or use an explicit terminating delimiter, or the
scanner will greedily eat characters that were meant to be the
next literal (`\x12` followed by `3` would scan as `\x123`). The
toy above sidesteps that class of bugs by construction; a real
SUT's recipe should keep the same pool and gate shape and add
whatever escape kinds the SUT actually emits.

*Decode-only oracle when no serializer exists.* When the SUT
exposes a `parse` but no round-trippable `serialize`, replace
the value → text → value chain with a test generator that
builds a matched `(expected_value, valid_text)` pair (the test
constructs the text against the same grammar) and asserts
`parse(text) == expected_value`. Keep the same one-gate-per-class
shape at the point where the parse-and-equal succeeds. This is a
different test shape from round-trip — the serializer is not
being checked — so pick it deliberately, not as a fallback for
"we forgot to add a serialize function".

**See also.** The "Assert a coverage gate after the run" recipe
(the `Cell<usize>` + per-case + `{runner}` pattern this recipe
reuses), the "Mix in domain boundaries with an exact probability"
recipe (biasing a generator toward rare classes), the "Sample
primitives, ranges, and strings" recipe
([`sample_choice`](crate::sample_choice) over a fixed pool).
For the assertion-message shape when the round-trip fires
against a real parser, see the "Semantic assertion patterns →
Parser, scanner, serializer" section of
[`skills/noprop/references/failure-diagnostics.md`](https://github.com/sile/noprop/blob/main/skills/noprop/references/failure-diagnostics.md).

## Assert a coverage gate after the run

**Goal.** Force at least one case to exercise the invariant (or the
region) that the run is meant to check, so a run where no case reached
it fails instead of silently passing.

**Uses.** `std::cell::Cell<bool>` for "did we ever reach it" gates,
`std::cell::Cell<usize>` for count- or rate-based gates, incremented
at the site where the invariant actually runs, and a run-after
`assert!(counter > 0, ...)` (or `> N`) that turns a zero count into
a failure. The [`Runner`](crate::Runner)'s `Display` embeds the
seed and run stats so the failure is reproducible.

```rust
# use std::cell::Cell;
# fn body() -> noprop::TestResult {
let non_empty_pops: Cell<usize> = Cell::new(0);
let mut runner = noprop::Runner::new(0);
runner.run(64, |ctx| {
    let mut model: Vec<u32> = Vec::new();
    let mut sut: Vec<u32> = Vec::new();
    let mut reached_non_empty_pop = false;

    let steps = noprop::sample_usize_in(ctx, 0..=16);
    for _ in 0..steps {
        match noprop::sample_usize_in(ctx, 0..2) {
            0 => {
                let v = noprop::sample_u32(ctx);
                model.push(v);
                sut.push(v);
            }
            _ => {
                let m = model.pop();
                let s = sut.pop();
                assert_eq!(m, s, "pop mismatch");
                if m.is_some() {
                    reached_non_empty_pop = true;
                }
            }
        }
    }
    if reached_non_empty_pop {
        non_empty_pops.set(non_empty_pops.get() + 1);
    }
    Ok(())
})?;
assert!(
    non_empty_pops.get() > 0,
    "no case reached a non-empty pop; the pop-mismatch invariant was vacuous\n{runner}"
);
# Ok(())
# }
# body().unwrap();
```

**Notes.**

*Choose the cell shape by what the gate means.* Use `Cell<bool>`
when the gate only asks whether the invariant was exercised at least
once; use `Cell<usize>` when the count itself matters (a minimum
number of hits, or a rate check like "at least a quarter of accepted
cases hit a non-empty pop"). `RefCell<T>` is for gates that need a
non-`Copy` aggregate across cases (a bounded history of witnesses,
a set of reached buckets); do not reach for it when a `Cell<bool>`
or `Cell<usize>` would fit, and never push a per-case temporary
through interior mutability — a plain `let mut` inside the closure
is the right home for that (as `reached_non_empty_pop` shows above).

*Record evidence at the invariant-eval site, not upstream of it.*
The counter must increment where the invariant actually runs — not
where the generator drew the target value, not where a branch was
selected. A case that picked the right shape but never reached the
check leaves the gate un-hit for the right reason; inflating the
counter earlier hides that. For the same reason, order the closure
so every
[`ctx.reject_case()`](crate::TestCaseContext::reject_case) and
every [`sample_with_rejection`](crate::sample_with_rejection) exit
sits strictly before any gate update — a case that increments a
counter and *then* gets rejected leaves the counter bumped by a
discarded case, which the run-after assert would count as evidence.

*Include the runner in the failure message.* The gate assertion
runs outside the property closure, so its message must carry the
seed and run stats itself. Use `{runner}` — the
[`Runner`](crate::Runner) implements `Display` but not `Debug`, so
`{runner:?}` will not compile. Give each gate its own assertion and
its own message so a failure names the un-hit condition; do not
fold several gates into one boolean.

*Paired gates when both sides matter.* An invariant that only fires
in one branch (empty vs non-empty, success vs error, growing vs
shrinking) is not gated by a single counter — a run that only
reaches the opposite side still passes silently. Pair the counter
with a second one for the other side and assert both. Do not gate
every enum variant mechanically; gate the classes where a missing
observation would leave the invariant vacuous.

*Distinguish coverage from a rejection post-condition.* An
`assert_eq!(runner.stats().rejected_cases, 0, "{runner}")` after
the run is a *valid-by-construction check on the generator*: it
asserts that no case needed to be discarded, which says nothing
about whether the invariant ran. Keep it separate from the coverage
gates above; do not treat rejected-case count as evidence of
coverage.

*Estimate the miss probability.* If a case reaches the target
region with probability `p`, a `Runner::run(N, ...)` misses it
entirely with probability `(1 - p)^N`. Get a rough `p` from the
generator's branch weights (or measure it under a fixed seed) and
compare `(1 - p)^N` to what you can tolerate. Raising `N` is the
last lever, not the first: the miss probability decays exponentially
in `N` but only at rate `p`, so a small `p` still needs many extra
cases to compensate. Fix the generator instead when `p` is small:

1. Confirm the target class is inside the generator's support at all
   (add it to the boundary set, the choice slice, or the command
   list).
2. Restructure so the target is a first-class branch — a
   [`sample_weighted_index`](crate::sample_weighted_index) arm or a
   dedicated sampler — instead of a lucky outcome of independent
   draws.
3. Give that branch an explicit weight or an exact
   [`Ratio`](crate::Ratio) via
   [`sample_with_boundaries`](crate::sample_with_boundaries).
4. Only after 1-3, if `(1 - p)^N` is still too high, raise `N`.

When an outer `sample_weighted_index` fixes the branch probability,
adding elements to an inner
[`sample_choice`](crate::sample_choice) pool does **not** change
the outer branch's probability. Adjust weights at the level whose
probability actually needs to shift.

Record the `p` estimate (and the branch weights it came from) in a
comment next to each gate. Every time you change a branch weight,
a boundary set, a choice pool, or a bounded range in the generator,
walk through the gates and re-check each `p`: a gate whose region
has become unreachable turns green silently, and a gate whose
region has become saturating adds noise but no coverage.

*Seed-derived flakes.* When the seed comes from
[`seed_from_env_or_time`](crate::seed_from_env_or_time), gate only
on regions the generator can reach reliably. A gate whose miss
probability is small but non-negligible turns into a flake — reached
under most seeds but not the one this run drew.

**See also.** The "Model-based (stateful) property" recipe (the
gated invariant is the same pop-mismatch check as its main example),
and
[`examples/search_space.rs`](https://github.com/sile/noprop/blob/main/examples/search_space.rs)
for boundary probabilities, branch weights, dependent draws, and two
coverage gates in one runnable property.

## Reproduce a failing seed

**Goal.** Turn a failure report into a repeatable local run.

**Uses.** The failing report itself
([`RunError::seed`](crate::RunError::seed),
[`RunError::case_index`](crate::RunError::case_index),
[`RunError::generated`](crate::RunError::generated)) and the
`reproduce with:` hint.

The failure report prints, on both `Debug` and `Display`:

```text
noprop failure at case 3 (seed=0x00ff00ff00ff00ff): high bit set: 0xe268430a
reproduce with: noprop::Runner::new(0x00ff00ff00ff00ff).run(64, |ctx| ...)
stats: accepted=3, rejected=0, total_samples=4
Generated values:
  - u32 = 3798483722  (at examples/reproduce.rs:17)
```

Recovery is manual and mechanical:

1. Copy the hex seed from the report.
2. Rerun the property with `Runner::new(<seed>)` and the *same* case
   budget printed in the hint. The rerun hits the same case index, so
   the failure surfaces again.

**Notes.** The rerun budget must match the original: a smaller budget
can shrink the rejection cap and turn the same failure into
`TooManyRejections`. Keep the property and generator identical between
runs — a change to either shifts the choice sequence for the same
seed.

**See also.**
[`RunError`](crate::RunError),
[`examples/reproduce.rs`](https://github.com/sile/noprop/blob/main/examples/reproduce.rs).
For the full "reproduce → diagnose → reduce → freeze" workflow,
including the external configuration a `sample_*` closure may
implicitly read, see
[`skills/noprop/references/failure-diagnostics.md`](https://github.com/sile/noprop/blob/main/skills/noprop/references/failure-diagnostics.md).

## Turn a trace into a hand-written regression test

**Goal.** Freeze a failure as a small deterministic test after
reproducing it. noprop does not ship automatic shrinking, so the
regression test is the hand-simplified witness.

The failure trace shows *the actual values* the primitives produced,
labelled with their call sites. Instead of storing the choice sequence
(which noprop deliberately does not expose), pick the smallest hand
witness that still triggers the bug and inline it as a regular
`#[test]`:

```rust
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
For the 5-step manual reduction checklist (drop setup, drop
prefix / suffix, drop collection elements, replace with domain
boundaries, confirm the same observable consequence), see the
"Reduce and freeze as a regular regression test" section of
[`skills/noprop/references/failure-diagnostics.md`](https://github.com/sile/noprop/blob/main/skills/noprop/references/failure-diagnostics.md).

## Observe cross-case state with interior mutability

**Goal.** Count a rare event, capture a sample of failing inputs, or
build any aggregate that spans multiple cases — despite the property
closure being `Fn`, not `FnMut`.

**Uses.** `std::cell::Cell` (for `Copy` counters) or `std::cell::RefCell`
(for non-`Copy` accumulators).

```rust
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
[`examples/search_space.rs`](https://github.com/sile/noprop/blob/main/examples/search_space.rs)
(coverage gates stored in `Cell<usize>` values).

## Keep the trace pointing at the user's call site

**Goal.** When wrapping a primitive in a user-defined `sample_*`
helper, keep the trace pointing at the caller, not at the wrapper's
body.

**Uses.** `#[track_caller]` on the wrapper.

```rust
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
