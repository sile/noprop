# Targeted Search

This document describes the design of noprop's targeted search policy,
added for property testing with scalar feedback.

## Key Characteristics

`Runner::run` samples inputs uniformly, so a property whose failure sits
in a narrow input region is found only with probability proportional to
that region's size. Targeted search keeps the same property closure and
the same generators, but drives them toward inputs that are more likely
to fail: a case that reaches a verdict without failing still carries
information, and `TestCaseContext::maximize` turns that information into
a scalar score the runner can act on.

The search loop is:

```text
record a case's draws and score
          │
          ▼
admit into a bounded corpus of high-scoring cases
          │
          ▼
mutate the corpus candidate and replay it
          │
          ▼
record the mutated case's draws and score
          ────────► repeat
```

`Runner::run_targeted(closure)` is the entry point. The closure receives
the same `&mut TestCaseContext` as `Runner::run`, so the same property
runs under both policies:

```rust
use noprop::Runner;

let mut runner = Runner::new(0xDEAD_BEEF, 16);
runner
    .run_targeted(|ctx| {
        let x = noprop::sample_u32(ctx);
        ctx.maximize((x as f64) / u32::MAX as f64);
        Ok(())
    })
    .expect("targeted run must succeed");
```

## Scope

Targeted search owns:

- choice metadata recorded alongside each draw
- exploratory replay of mutated candidates
- the corpus, its admission and pick rules, and candidate mutation
- the scalar feedback protocol

It does not own:

- semantic coverage or feature-based novelty search (planned in a later
  issue)
- a general-purpose optimizer that returns an arbitrary objective's
  maximum
- shrink and failure-case minimization
- public sampler traits or pluggable search policies
- shared case-loop logic between `run` and `run_targeted` (the shared
  structure is deferred until the corpus-guided implementation lands)

## Feedback Protocol

A targeted case scores itself by calling
`TestCaseContext::maximize(score)`. Multiple calls aggregate to the
maximum, so a case reports its best attempt, not its last one.

`maximize` is a no-op outside targeted mode: the uniform runner and
`TestCaseContext::new` ignore it, and the feedback state in those modes
does not allocate.

A property failure (panic or returned `Err`) always beats a feedback
problem: the runner reports the failure, never a feedback error. An
accepted case that never called `maximize` ends the run with a missing
feedback error, and NaN or infinity ends it with an invalid feedback
error. Rejected cases are different: `reject_case` abandons the case
without scoring, the run continues, and the rejection does not satisfy
the feedback requirement of a later accepted case.

## Recording Model

Each case records a choice sequence: the draws its generators produced,
the constraint metadata each draw was recorded under, and the attempt
span structure that records how nested rejection scopes opened and
closed.

Draws are stored as raw bytes. The recorded value is the generator
output, not a normalized domain value: the property re-applies its own
constraint (for example `sample_below(ctx, n)` reduces the recorded
value with `% n`), so a stored value stays meaningful under any
constraint the mutated control flow imposes on that position.

Constraint metadata is one of:

- `Bounded` — a rejection-sampled integer with an upper bound
- `Choice` — an index into a finite choice list
- `Integer` — a plain integer primitive draw
- `Raw` — constraint-free bytes (raw byte slices, string payloads, …)

## Exploratory Replay

Replay matches recorded draws positionally, never by value: the mutated
candidate asks for draw N, and the runner serves it from recording
position N. The value at that position is what control flow depends on,
so mutation is what changes control flow.

A mutated candidate's control flow may legitimately diverge from the
recording, and the divergence is treated as search, not as corruption.
Four rules cover every divergence:

1. **Changed constraint at the same position.** The mutated control
   flow reads draw N under a different primitive or bound. The value is
   regenerated before it is replayed, so the value the property executed
   is exactly the value stored for the next generation.
2. **Changed width at the same position.** The recorded draw is dead;
   it is replaced in place with a freshly generated draw, keeping the
   sequence bounded.
3. **A request past the recording.** The draw is generated and appended
   to the sequence, extending it for future generations. Generated
   draws count toward a per-case cap; a case that exceeds the cap is
   rejected and charged to the rejection budget, so an unbounded loop
   opened by mutation still terminates.
4. **An unconsumed suffix.** Draws the mutated control flow never read
   are discarded at the case boundary, so stale values do not leak into
   the next generation.

Rule 1's executed-value-equals-stored-value invariant is the heart of
the design. An accepted exploratory case's sequence becomes the next
mutation seed, and mutation rewrites stored bytes in place. If the
stored value diverged from the value the property actually executed,
the next generation would mutate a value that never ran. This is the
problem specific to imperative PBT: a declarative setup has no control
flow to diverge, but imperative properties need the invariant spelled
out.

Attempt spans are validated in strict replay and are neither validated
nor recorded during exploration. A mutated candidate's span structure
may legitimately differ from the recording, and nothing consumes
exploratory spans, so the recorded structure is treated purely as a
mutation seed.

## Corpus and Mutation

Accepted cases enter a corpus of the highest-scoring candidates
(`CORPUS_SIZE = 64`). A new candidate is admitted unconditionally while
the corpus is not full; a full corpus keeps the candidate only when its
score beats the lowest-scored entry, and ties keep the incumbent.

A new case either restarts (with probability
`1 / RANDOM_RESTART_DENOM`) and records fresh from a new seed, or
explores: it picks a corpus entry and mutates it. Picks are uniform
among entries, except with probability `1 / LOW_SCORE_DENOM` the
lowest-scored entry is picked instead, concentrating some energy on the
corpus's weak spot.

Mutation rewrites each draw with probability `1 / MUTATION_DENOM`. The
rewrite strategy follows the constraint kind:

- `Bounded` and `Choice` draws are rewritten inside their recorded
  domain
- `Integer` draws are rewritten across their recorded width (1, 2, 4,
  8, or 16 bytes)
- `Raw` draws are regenerated as a whole

The draw count and span structure are preserved by mutation itself;
structural change comes from control flow divergence during
exploration, not from mutation.

## Determinism and Reproduction

The whole run is reproducible from the seed. One runner PRNG supplies,
in a fixed order, the restart decisions, per-case seeds, corpus picks,
and mutation rolls, so a fixed seed yields a fixed sequence of cases and
mutations.

A failure report's reproduce hint reruns the exact failing seed with
the original iteration budget, so the rerun hits the same rejection cap
and the same exit. In targeted mode the hint names `run_targeted` and
leaves the closure body as a placeholder for the original property
closure.

## Known Limitations

- A single-draw property whose failure requires one specific mutated
  value can lag uniform sampling: the initial mutation strategy pays a
  per-position redraw cost that uniform sampling does not. See
  `examples/targeted_demo.rs` for a measured comparison.
- The search constants (corpus size, pick and mutation odds, restart
  odds, draw cap) are initial guesses. Tuning them against synthetic
  targets is deferred to the benchmark issue.
- Targeted search cannot create inputs outside the generator's support;
  generator bias and search policy effectiveness are kept separate when
  interpreting results.
