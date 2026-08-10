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
- [Cluster-level invariant across multiple actors](#cluster-level-invariant-across-multiple-actors)
- [Bounded run-to-quiescence](#bounded-run-to-quiescence)
- [Cross-step invariant with append-only history](#cross-step-invariant-with-append-only-history)
- [Stateful streaming API driven by a command loop](#stateful-streaming-api-driven-by-a-command-loop)
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

```rust
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

```rust
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
[`examples/basics.rs`](https://github.com/sile/noprop/blob/main/examples/basics.rs)
("Idiom 2: boundary values").

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
[`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case),
[`examples/rejection.rs`](https://github.com/sile/noprop/blob/main/examples/rejection.rs).

## Model-based (stateful) property

**Goal.** Drive a model and a system-under-test with the same command
sequence, comparing them at every step.

**Uses.** [`Runner::run`](crate::Runner::run) or
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided),
plus [`sample_usize_in`](crate::sample_usize_in) and the primitives
the commands need.

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
to a realistic subject (an LRU cache with a plausible-looking bug).

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
by a command loop" recipe (model-free variant).

## Stateful streaming API driven by a command loop

**Goal.** Test a streaming API — one that does not have a single
"correct model" to compare against, but that accumulates side-effects
in a buffer — by driving it with a random command loop and asserting
the invariant on the final output alone.

**Uses.** A random command loop for the streaming interface
(feed / flush / reset-style calls), a single terminator call once the
loop ends, and one assert on the accumulated output.

```rust
# fn body() -> noprop::TestResult {
noprop::Runner::new(0).run(64, |ctx| {
    // Streaming SUT: an append-only byte buffer. Loop over random
    // Feed / Flush commands, then finalise once outside the loop.
    let mut buffer: Vec<u8> = Vec::new();
    let mut fed: Vec<u8> = Vec::new();

    let steps = noprop::sample_usize_in(ctx, 0..=8);
    for _ in 0..steps {
        match noprop::sample_usize_in(ctx, 0..2) {
            0 => {
                // Feed: append some random bytes.
                let n = noprop::sample_usize_in(ctx, 0..=4);
                let bytes = noprop::sample_bytes_vec(ctx, n);
                fed.extend_from_slice(&bytes);
                buffer.extend_from_slice(&bytes);
            }
            _ => {
                // Flush: no-op here, but a real streaming API might
                // emit a boundary marker or reset internal buffering.
            }
        }
    }
    // Terminator: called once, outside the loop. Real APIs finalise
    // padding, flush buffered bytes, and hand back the output here.
    let output = std::mem::take(&mut buffer);

    // The only invariant checked: the whole output round-trips the
    // full sequence of fed bytes.
    assert_eq!(
        output, fed,
        "streaming output did not round-trip fed bytes"
    );
    Ok(())
})?;
# Ok(())
# }
# body().unwrap();
```

**Notes.** The two-stage shape — random command loop, then one
terminator — matters: an API that requires a `finish()` call to emit
its trailing bytes will fail the round-trip if the terminator is
inside the loop. Keep the invariant on the *final* output only;
per-step assertions in a streaming pipeline usually can't tell
whether the byte the SUT just emitted is correct or is waiting for
more input. Streaming compressors and encoders (deflate, gzip),
incremental hashers (`update` / `finalize`), buffered writers, and
network protocol codecs that feed bytes in and emit framed messages
(HTTP, TLS record layer, WebSocket, RTMP) typically follow this
shape.

**See also.** The "Model-based (stateful) property" recipe (the
model-driven counterpart), the "Cluster-level invariant across
multiple actors" recipe (multi-SUT extension), the "Bounded
run-to-quiescence" recipe (bounding a settling protocol), the
"Cross-step invariant with append-only history" recipe (recipes
that keep per-step state alongside the SUT).

## Steer the search with feedback

**Goal.** Concentrate sampling on inputs that reach a semantic region
of interest, rather than sampling uniformly.

**Uses.**
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided),
[`TestCaseContext::event`](crate::TestCaseContext::event),
[`TestCaseContext::bucket`](crate::TestCaseContext::bucket),
[`TestCaseContext::transition`](crate::TestCaseContext::transition).

```rust
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

**See also.** The "Assert a coverage gate after the run" recipe below
(force a run to fail when the steered region was never reached),
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided),
[`docs::feedback_guided_search`](crate::docs::feedback_guided_search),
[`examples/feedback_guided.rs`](https://github.com/sile/noprop/blob/main/examples/feedback_guided.rs).

## Assert a coverage gate after the run

**Goal.** Force at least one case to exercise the invariant (or the
region) that the run is meant to check, so a run where no case reached
it fails instead of silently passing.

**Uses.** A `std::cell::Cell` counter incremented at the site where
the invariant actually runs, and a run-after
`assert!(counter > 0, ...)` that turns a zero count into a failure.
The [`Runner`](crate::Runner)'s `Display` embeds the seed and stats
so the failure is reproducible.

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

**Notes.** An invariant that fires only when the state reaches a
specific shape (a non-empty container, a committed entry, a fully
consumed buffer) can pass vacuously — every case may end before the
shape appears, and the invariant is never actually checked. Gate the
invariant with a counter that increments only where the invariant
runs, and turn a zero count into a run-after failure. The mechanism is
orthogonal to the search policy: it works the same under
[`Runner::run`](crate::Runner::run) and
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided).

Count only the accepted cases — the counter must increment where the
invariant actually runs, and the assertion should compare that count
against zero rather than the total attempt count. Rejection scopes
([`sample_with_rejection`](crate::sample_with_rejection),
[`TestCaseContext::reject_case`](crate::TestCaseContext::reject_case))
may discard cases mid-run; a rejected case that ended before the
invariant fired must not contribute to "the region was reached"
evidence.

The same counter + run-after assert plugs into
[`Runner::run_feedback_guided`](crate::Runner::run_feedback_guided) as
well: increment inside the same branch that calls `ctx.event(...)` so
the assert confirms the steered region was actually reached. When the
region is rare and the seed comes from
[`seed_from_env_or_time`](crate::seed_from_env_or_time), the region
may be reached under most seeds but not the one this run drew, and
the gate becomes a flake — gate only on regions the *generator* can
reach reliably (build the generator so the region is inside its
support).

**See also.** The "Model-based (stateful) property" recipe (the
gated invariant is the same pop-mismatch check as its main example),
the "Steer the search with feedback" recipe (feedback-guided use of
the same gate mechanism),
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
[`examples/basics.rs`](https://github.com/sile/noprop/blob/main/examples/basics.rs)
("Pitfall 1: the closure is `Fn`, not `FnMut`").

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
