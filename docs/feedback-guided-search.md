# Feedback-Guided Search

This document describes the design of noprop's feedback-guided search
policy, used for property testing over semantic feedback. Throughout
the document, **corpus** refers specifically to the bounded collection
of interesting cases that feedback-guided search maintains internally
(the same bound reported by
[`Stats::max_corpus_size`](crate::Stats::max_corpus_size)); it is not
another name for feedback-guided search itself.

## Key Characteristics

`Runner::run` samples inputs uniformly, so a property whose failure sits
in a narrow input region is found only with probability proportional to
that region's size. Feedback-guided search tracks semantic features —
finite events, caller-bucketed state values, and abstract state
transitions — and steers the search toward inputs that cover features no
earlier case covered.

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

`Runner::run_feedback_guided(cases, closure)` is the entry point. The
closure receives the same `&mut TestCaseContext` as `Runner::run`, so
the same property runs under both. See the
[`run_feedback_guided`](crate::Runner::run_feedback_guided)
documentation for a runnable example.

## Feedback Protocol

A feedback-guided case reports features by calling:

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

Feedback is not mandatory:

- An accepted case that reports no feature is simply not interesting: it
  never enters the corpus, and the run continues.
- A property failure (panic or returned `Err`) always beats any feedback
  consideration and is reported immediately.

The semantic methods are no-ops outside feedback-guided mode: the
uniform runner and `TestCaseContext::new` ignore them, and the feedback
state in those modes does not allocate.

## Choosing a feedback method

The three feedback methods observe different things; pick the one that
matches the property's domain:

- `event(label)` — for finite occurrences whose *count* matters only
  coarsely. Use it for protocol phases, snapshot installs, error paths
  reached. Repeated events saturate into buckets (1 / 2-3 / 4-7 / 8+),
  so the search distinguishes "visited once" from "visited many times"
  without needing exact counts.
- `bucket(label, value)` — for state values that are large or
  continuous, where the caller's domain knowledge picks the interesting
  ranges. Example: queue length bucketed as 0 / 1-4 / 5-16 / 17+.
  Bucket *before* reporting: a raw value that differs every case (a
  timestamp, a byte count, a sequence number) defeats the corpus —
  every case reports a novel feature, the global registry hits its
  cap, and novelty stops meaning anything.
  Aim for roughly 3–10 buckets per label: fewer than three collapses
  the signal to "hit / not hit" and the search loses its steering
  ability, while more than ten dilutes the registry — a single label
  at 100 buckets can eat roughly 10% of the registry cap (currently
  1024) before any other feature is reported. Logarithmic bands
  (`0 / 1-4 / 5-16 / 17-64 / 65+`) or fixed ranges (`0-1024 /
  1025-8192 / 8193-32768`) tend to balance signal and cost.
- `transition(label, from, to)` — for stateful tests, when the abstract
  state change itself is what matters (role changes, protocol phase
  advances). The `(from, to)` pair is part of the feature identity, so
  "follower → leader" and "leader → follower" are different features
  even under the same label.

The label must identify a *meaning*, not a call site: using the same
label for different meanings silently merges them (the search treats
the first coverage as sufficient); using different labels for the same
meaning splits one feature into many. `format!`-built labels are
discouraged — a representation change silently changes every feature
identity, and unbounded labels defeat the registry cap.

## Feature Registry

The runner keeps a global observation set of features, in
first-registration order, capped at 1024 features (currently). A
feature already present is never interesting again; once the cap is
reached, new features are not registered and never make a case
interesting, so a high-cardinality property cannot grow the registry
without bound.

A per-case cap (currently 64) bounds the features one case may
report; the excess is discarded in report order.
An event saturating to a higher bucket replaces its earlier feature and
does not count toward the per-case cap. Note that the replacement
itself can still be a *global* novelty: a bucket that no case has
reached before is registered as a new feature on the next case
boundary, which is how repeated events steer the search toward
high-visit paths.

## Corpus and Mutation

Accepted cases that register at least one novel feature enter the
accepted queue; rejected cases that register novel features enter a
separate rejected queue, kept as low-energy scaffolding for reaching
sparse preconditions (picked with probability
`1 / REJECTED_PICK_DENOM`; while the accepted queue is empty, the
rejected queue is the only source and is always picked). The combined
size of both queues is capped at `CORPUS_SIZE`.

Admission:

- A case with novel features is admitted while the combined size is
  below the cap.
- Once full, the entry with the fewest newly registered features is
  evicted; ties evict the earliest arrival.
- A case with no novel feature is never admitted.

A new case either restarts (with probability
`1 / RANDOM_RESTART_DENOM`) and records fresh from a new seed, or
explores: it picks a corpus entry and mutates it. Accepted picks are
uniform among entries; with probability `1 / REJECTED_PICK_DENOM` the
rejected queue is picked instead when it is non-empty.

Mutation rewrites each draw with probability `1 / MUTATION_DENOM`:
bounded-domain draws (Bounded / Choice) get a fresh value inside their
recorded constraint, while constraint-free draws (Raw: raw bytes, string
payload, …) are regenerated as a whole. A mutated candidate replays its
draws with generated tail draws for control flow the mutation
introduces, under the four exploratory replay rules and the per-case
generated-draw cap.

## Determinism and Reproduction

The whole run is reproducible from the seed. One runner PRNG supplies,
in a fixed order, the restart decisions, per-case seeds, rejected-queue
rolls, corpus picks, and mutation rolls, so a fixed seed yields a fixed
sequence of cases and mutations.

A failure report's reproduce hint reruns the exact failing seed with
the original case budget and names
`run_feedback_guided(cases, |ctx| ...)`, so the rerun reproduces the
same failure. The report also carries the failing case's candidate index
(across accepted and rejected cases) and the semantic features the
failing case reported, so the interesting input region is visible
without exposing the choice sequence itself.

## Known Limitations

- The search constants (corpus size, pick and mutation odds, restart
  odds, rejected-queue odds, feature caps) are initial guesses. Tuning
  them against synthetic targets is deferred until benchmark data
  exists.
- Feedback-guided search cannot create inputs outside the generator's
  support; generator bias and search policy effectiveness are kept
  separate when interpreting results.
