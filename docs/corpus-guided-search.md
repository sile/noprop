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

## Choosing a feedback method

The four feedback methods observe different things; pick the one that
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
  every case reports a novel feature, the global registry hits
  `MAX_GLOBAL_FEATURES`, and novelty stops meaning anything.
- `transition(label, from, to)` — for stateful tests, when the abstract
  state change itself is what matters (role changes, protocol phase
  advances). The `(from, to)` pair is part of the feature identity, so
  "follower → leader" and "leader → follower" are different features
  even under the same label.
- `maximize(score)` — when "closeness to failure" can be designed as a
  single scalar. Unlike the semantic methods it never registers a
  feature; it only steers admission and eviction within a feature group
  (under the priority policy). It is optional: a case that never calls
  it is still admitted if it reports a novel feature.

The label must identify a *meaning*, not a call site: using the same
label for different meanings silently merges them (the search treats
the first coverage as sufficient); using different labels for the same
meaning splits one feature into many. `format!`-built labels are
discouraged — a representation change silently changes every feature
identity, and unbounded labels defeat the registry cap.

## Feature Registry

The runner keeps a global observation set of features, in
first-registration order, capped at `MAX_GLOBAL_FEATURES`. A feature
already present is never interesting again; once the cap is reached,
new features are not registered and never make a case interesting, so a
high-cardinality property cannot grow the registry without bound.

A per-case cap (`MAX_FEATURES_PER_CASE` in `src/rng.rs`) bounds the
features one case may report; the excess is discarded in report order.
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
  evicted; ties keep the earlier arrival. When the case carries a
  scalar priority, ties within that group break on the lowest score
  (missing scores count as the lowest).
- A case with no novel feature is admitted only when its priority beats
  the lowest-scored entry of a feature group it overlaps, replacing
  that entry (the targeted top-k replacement, restricted to one feature
  group). This keeps the search from drifting entirely away from a
  promising group once its features are covered.

When the corpus is full of scored entries and a new case with no
priority (it never called `maximize`, or called it with an invalid
value) registers novel features, the eviction tie-break treats the
missing score as the lowest: the new entry can evict itself on arrival.
Its features still enter the global registry — so they never make
another case interesting — but its choice sequence is discarded without
serving as a mutation parent. This is an accepted consequence of
"missing scores count as the lowest" under `SemanticWithPriority`; it
only occurs when scored and unscored cases are mixed, since an
all-unscored corpus evicts the earliest arrival instead (a plain FIFO
rotation). A property that wants every novel discovery to persist in
the corpus should call `maximize` consistently.

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
