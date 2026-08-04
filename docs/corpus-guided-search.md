# Corpus-Guided Search

This document describes the design of noprop's corpus-guided search
policy, added for property testing with semantic feedback.

## Key Characteristics

`Runner::run` samples inputs uniformly, so a property whose failure sits
in a narrow input region is found only with probability proportional to
that region's size. Targeted search (see `targeted_search`) drives the
generators toward inputs that report a high scalar score. Corpus-guided
search instead tracks semantic features — finite events, caller-bucketed
state values, and abstract state transitions — and steers the search
toward inputs that cover features no earlier case covered.

Semantic features are reported by the property itself through
`TestCaseContext::event`, `bucket`, and `transition`. Unlike raw code
coverage, these features can observe abstract state that does not map to
a distinct control-flow edge: a stateful test's model state, a protocol
phase, a snapshot install. This matches the stateful-PBT use case, where
the same code path can run under many different semantic states.

The search loop is:

```text
record a case's draws and features
           │
           ▼
register novel features; admit interesting cases into a bounded corpus
           │
           ▼
pick a corpus entry, mutate it, and replay it
           │
           ▼
record the mutated case's draws and features
           ────────► repeat
```

`Runner::run_corpus_guided(closure)` is the entry point. The closure
receives the same `&mut TestCaseContext` as `Runner::run`, so the same
property runs under every policy. See the
[`run_corpus_guided`](crate::Runner::run_corpus_guided) documentation
for a runnable example.

## Feedback Protocol

A corpus-guided case reports features by calling:

- `TestCaseContext::event(label)` — reaching a finite event
- `TestCaseContext::bucket(label, value)` — a caller-bucketed state
  value
- `TestCaseContext::transition(label, from, to)` — an abstract state
  transition

Feature identity is the `(label, kind)` pair; the same label with a
different bucket value or transition endpoints is a different feature.
Repeated events within one case saturate into a fixed hit-count bucket
(1 / 2-3 / 4-7 / 8+ occurrences) so a case that visits an event many
times is distinguished from one that visits it once, without unbounded
counts.

Unlike targeted mode, feedback is not mandatory:

- An accepted case that reports no feature is simply not interesting: it
  never enters the corpus, and the run continues.
- `maximize` is an optional scalar priority. A case that never calls it,
  or calls it with `NaN` / infinity, proceeds without a priority; no
  missing / invalid feedback error is raised (targeted mode's
  `MissingFeedback` / `InvalidFeedback` do not apply here).
- A property failure (panic or returned `Err`) always beats any feedback
  consideration and is reported immediately.

The semantic methods are no-ops outside corpus-guided mode: the uniform
runner and `TestCaseContext::new` ignore them, and the feedback state in
those modes does not allocate.

## Feature Registry

The runner keeps a global observation set of features, in
first-registration order, capped at `MAX_GLOBAL_FEATURES`. A feature
already present is never interesting again; once the cap is reached,
new features are not registered and never make a case interesting, so a
high-cardinality property cannot grow the registry without bound.

A per-case cap (`MAX_FEATURES_PER_CASE` in `src/rng.rs`) bounds the
features one case may report; the excess is discarded in report order.
An event saturating to a higher bucket replaces its earlier feature and
does not count as a new one.

## Corpus and Mutation

Accepted cases that register at least one novel feature enter the
accepted queue; rejected cases that register novel features enter a
separate rejected queue, kept as low-energy scaffolding toward sparse
preconditions (picked with probability
`1 / REJECTED_PICK_DENOM`). The combined size of both queues is capped
at `CORPUS_SIZE`.

Admission:

- A case with novel features is admitted while the combined size is
  below the cap.
- Once full, the entry with the fewest newly registered features is
  evicted; ties keep the earlier arrival. When the case carries a
  scalar priority, ties within that group break on the lowest score
  (missing scores count as the lowest).
- A case with no novel feature is admitted only when its priority beats
  the lowest-scored entry of a feature group it overlaps, replacing
  that entry (the targeted top-k replacement, restricted to one feature
  group). This keeps the search from drifting entirely away from a
  promising group once its features are covered.

A new case either restarts (with probability
`1 / RANDOM_RESTART_DENOM`) and records fresh from a new seed, or
explores: it picks a corpus entry and mutates it. Accepted picks are
uniform among entries, except with probability `1 / LOW_SCORE_DENOM`
the lowest-scored accepted entry is picked instead, concentrating some
energy on the corpus's weak spot.

Mutation and exploratory replay are shared with targeted search: draws
are rewritten within their recorded constraints with probability
`1 / MUTATION_DENOM`, and a mutated candidate replays its draws with
generated tail draws for control flow the mutation introduces. The four
exploratory replay rules and the per-case generated-draw cap are
identical.

## Determinism and Reproduction

The whole run is reproducible from the seed. One runner PRNG supplies,
in a fixed order, the restart decisions, per-case seeds, rejected-queue
rolls, corpus picks, and mutation rolls, so a fixed seed yields a fixed
sequence of cases and mutations.

A failure report's reproduce hint reruns the exact failing seed with
the original iteration budget and names `run_corpus_guided`. The report
also carries the failing case's candidate index (across accepted and
rejected cases) and the semantic features the failing case reported, so
the interesting input region is visible without exposing the choice
sequence itself.

## Known Limitations

- The search constants (corpus size, pick and mutation odds, restart
  odds, rejected-queue odds, feature caps) are initial guesses. Tuning
  them against synthetic targets is deferred until benchmark data
  exists.
- Corpus-guided search cannot create inputs outside the generator's
  support; generator bias and search policy effectiveness are kept
  separate when interpreting results.
- A property whose features are all trivially covered on the first few
  cases leaves the corpus with few novel registrations; the
  scalar-priority replacement path exists precisely so a covered group
  can still be refined by score.
